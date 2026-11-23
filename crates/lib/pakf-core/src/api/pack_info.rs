//! This module implements `pakf status` and `remove`.
//! Neither of them uses the network.

use std::{collections::BTreeSet, fmt::Write as _};

use schemars::JsonSchema;
use serde::Serialize;

use super::{
    pack::{SyncedPack, edited_files, fixed_notes, holds_accepted, is_edited, remove},
    pack_keep::PackKeep,
    pack_ls::PackFileKind,
};
use crate::{Error, RepoPath, RepoRoot, Result, config, conflicts, pack, source::read_optional};

/// A `PackFileState` is the state of one file of a locked pack that the pack
/// owns, whatever its acceptance.
// `PackFileState` is a wire type: `pakf status --json` prints it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PackFileState {
    /// The file holds its base.
    Unchanged,
    /// The user edited the file.
    Edited,
    /// The file holds a conflict that a merge left.
    Conflicted,
    /// The user deleted the file, and a sync keeps it deleted.
    Deleted,
}

impl PackFileState {
    /// `label` returns the word that the report prints.
    pub fn label(self) -> &'static str {
        match self {
            PackFileState::Unchanged => "unchanged",
            PackFileState::Edited => "edited",
            PackFileState::Conflicted => "conflicted",
            PackFileState::Deleted => "deleted",
        }
    }
}

/// A `PackAcceptance` is the acceptance of a file of a pack whose acceptance
/// the lock records, beside the file's state.
// `PackAcceptance` is a wire type: `pakf ls --json` prints it, so a
// change of a variant changes the published schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PackAcceptance {
    /// The file holds the edit or the deletion that a maintainer accepted.
    Accepted,
    /// The lock records an acceptance that the file doesn't hold, because
    /// the file changed since, or holds its base again.
    Stale,
}

impl PackAcceptance {
    /// `ALL` lists the acceptances in the order in which the text counts
    /// them.
    pub const ALL: [PackAcceptance; 2] = [PackAcceptance::Accepted, PackAcceptance::Stale];

    /// `label` returns the word that the report prints.
    pub fn label(self) -> &'static str {
        match self {
            PackAcceptance::Accepted => "accepted",
            PackAcceptance::Stale => "stale",
        }
    }
}

/// A `PackState` is the state of one pack in the lock.
#[derive(Debug, Clone)]
pub struct PackState {
    /// The pack's id, `@scope/name`.
    pub name: String,
    /// The version that the lock records.
    pub version: String,
    /// The source.
    pub source: String,
    /// The ref of a git source.
    pub git_ref: Option<String>,
    /// Each file that differs from its base, with its state.
    pub files: Vec<PackFile>,
    /// Each file that a higher pack owns, with that pack, and whether the
    /// last sync changed this pack's version of it.
    pub shadowed: Vec<ShadowedFile>,
}

/// A `PackFile` is a file of a pack that differs from its base or has an
/// acceptance.
#[derive(Debug, Clone)]
pub struct PackFile {
    /// The file's destination.
    pub path: String,
    /// The file's kind, as `pakf ls` gives it.
    pub kind: PackFileKind,
    /// The file's state.
    pub state: PackFileState,
    /// The file's acceptance, when the lock records one.
    pub acceptance: Option<PackAcceptance>,
    /// The note of the file's acceptance, when the lock records one.
    pub note: Option<String>,
}

/// A `ShadowedFile` is a file of a pack that a higher pack owns.
#[derive(Debug, Clone)]
pub struct ShadowedFile {
    /// The file's destination.
    pub path: String,
    /// The pack that owns it.
    pub owner: String,
    /// Whether the last sync changed this pack's version of it.
    pub changed: bool,
}

/// A `PackStatus` is the report of `pakf status`.
#[derive(Debug, Clone)]
pub struct PackStatus {
    /// Each pack in the lock, in the order of the packs, from the lowest to
    /// the highest.
    pub packs: Vec<PackState>,
}

impl PackStatus {
    /// `drifted` returns whether a file of a pack is conflicted or deleted,
    /// or edited and not editable, and its acceptance isn't `accepted`.
    // sokf-implements: CLI-4_AC-3
    pub fn drifted(&self) -> bool {
        self.packs.iter().any(|p| {
            p.files.iter().any(|f| {
                let editable = f.kind == PackFileKind::Editable && f.state == PackFileState::Edited;
                f.state != PackFileState::Unchanged
                    && f.acceptance != Some(PackAcceptance::Accepted)
                    && !editable
            })
        })
    }

    /// `to_text` returns a line for each pack and a line for each of its
    /// files that differs from its base or has an acceptance: its state, its
    /// path, its acceptance in parentheses, and the note of the acceptance.
    pub fn to_text(&self) -> String {
        if self.packs.is_empty() {
            return "the lock holds no pack\n".into();
        }
        let mut out = String::new();
        for p in &self.packs {
            let source = match &p.git_ref {
                Some(r) => format!("{}@{r}", p.source),
                None => p.source.clone(),
            };
            let _ = writeln!(out, "{} {} from {source}", p.name, p.version);
            // sokf-implements: CLI-4_AC-5
            // sokf-implements: CLI-4_AC-6
            // sokf-implements: CLI-4_AC-8
            // sokf-implements: CLI-4_AC-15
            for f in &p.files {
                let _ = write!(out, "{} {}", f.state.label(), f.path);
                let marks: Vec<&str> = (f.kind == PackFileKind::Editable)
                    .then(|| f.kind.label())
                    .into_iter()
                    .chain(f.acceptance.map(PackAcceptance::label))
                    .collect();
                if !marks.is_empty() {
                    let _ = write!(out, " ({})", marks.join(", "));
                }
                if let Some(note) = &f.note {
                    let _ = write!(out, ": {note}");
                }
                out.push('\n');
            }
            for f in &p.shadowed {
                let changed = if f.changed {
                    ", changed in this pack at the last sync"
                } else {
                    ""
                };
                let _ = writeln!(out, "shadowed {} by the pack {}{changed}", f.path, f.owner);
            }
        }
        out
    }
}

/// `pack_status` reports each pack in the lock, in the order of the packs,
/// from the lowest to the highest, and each of its files that is edited,
/// conflicted, or deleted or that has an acceptance, with the files whose
/// acceptance is `accepted` after the others. It reads the working tree,
/// the configuration, and the lock only, and writes no file.
// sokf-implements: CLI-4_AC-1
// sokf-implements: CLI-4_AC-2
// sokf-implements: CLI-4_AC-7
// sokf-implements: CLI-4_AC-9
pub fn pack_status(root: &RepoRoot) -> Result<PackStatus> {
    let lock = pack::read_lock(root)?;
    let packs = pack::ordered(root, &lock)
        .into_iter()
        .filter_map(|name| lock.packs.get(&name).map(|p| (name, p)))
        .map(|(name, p)| {
            let mut files: Vec<PackFile> = p
                .files
                .iter()
                // A shadowed record is listed below, with its owner.
                .filter_map(|(rel, f)| {
                    let (state, acceptance) = file_state(root, rel, f)?;
                    (state != PackFileState::Unchanged || acceptance.is_some()).then(|| PackFile {
                        path: f.path.clone(),
                        kind: PackFileKind::of(rel, f),
                        state,
                        acceptance,
                        note: f.accepted.as_ref().map(|a| a.note.clone()),
                    })
                })
                .collect();
            files.sort_by(|a, b| {
                let accepted = |f: &PackFile| f.acceptance == Some(PackAcceptance::Accepted);
                (accepted(a), &a.path).cmp(&(accepted(b), &b.path))
            });
            // sokf-implements: CLI-4_AC-4
            let shadowed = p
                .files
                .values()
                .filter_map(|f| {
                    let owner = f.shadowed_by.clone()?;
                    Some((f, owner))
                })
                .map(|(f, owner)| ShadowedFile {
                    owner,
                    path: f.path.clone(),
                    changed: f.changed,
                })
                .collect();
            PackState {
                name,
                version: p.version.clone(),
                source: p.source.clone(),
                git_ref: p.git_ref.clone(),
                files,
                shadowed,
            }
        })
        .collect();
    Ok(PackStatus { packs })
}

/// `file_state` returns the state and the acceptance with which `pack
/// status` prints the file `rel` of a locked pack, whose record is `f`, or
/// none when another pack shadows it, because the owner's record describes
/// the file, and a shadowed record holds no acceptance. The state is
/// unchanged when the file holds its base, deleted when it is absent,
/// conflicted when it holds a conflict marker, and edited otherwise, such as
/// when it isn't text. The acceptance is none without an acceptance in the
/// lock, `accepted` while the file holds its accepted version, and `stale`
/// otherwise, and always for an unchanged file. `pakf ls` takes its states
/// from it too, so that the two commands agree.
// sokf-implements: CLI-5_AC-7
// sokf-implements: CLI-5_AC-25
pub(super) fn file_state(
    root: &RepoRoot,
    rel: &str,
    f: &pack::LockedFile,
) -> Option<(PackFileState, Option<PackAcceptance>)> {
    if f.shadowed_by.is_some() {
        return None;
    }
    if !is_edited(root, rel, f) {
        // A file that holds its base holds no edit to accept.
        let stale = f.accepted.as_ref().map(|_| PackAcceptance::Stale);
        return Some((PackFileState::Unchanged, stale));
    }
    let path = RepoPath::parse(&f.path);
    let dest = path
        .as_ref()
        .map_or(pack::Dest::Absent, |rp| pack::read_dest(&rp.to_fs(root)));
    // The bytes read as text with each invalid sequence replaced, as sync
    // and `pakf accept` read them, so that a conflict in a file that isn't
    // UTF-8 reads as one in every command. An entry that isn't a readable
    // regular file is an edit.
    // sokf-implements: CLI-2_AC-57
    // sokf-implements: CLI-2_AC-58
    let state = match (&dest, path) {
        (pack::Dest::Absent, _) => PackFileState::Deleted,
        (pack::Dest::File(bytes), Some(rp))
            if !conflicts::conflict_lines(&rp, &String::from_utf8_lossy(bytes)).is_empty() =>
        {
            PackFileState::Conflicted
        }
        _ => PackFileState::Edited,
    };
    let acceptance = f.accepted.as_ref().map(|_| {
        if holds_accepted(root, rel, f) {
            PackAcceptance::Accepted
        } else {
            PackAcceptance::Stale
        }
    });
    Some((state, acceptance))
}

/// A `PackRemoved` is the report of `pakf remove`.
#[derive(Debug, Clone)]
pub struct PackRemoved {
    /// The pack's id, `@scope/name`.
    pub name: String,
    /// Whether the command deleted the pack's `[[pack]]` entry.
    pub undeclared: bool,
    /// Whether the lock held the pack.
    pub locked: bool,
    /// Each file that the command deleted.
    pub removed: Vec<String>,
    /// Each edited file that the command kept.
    pub kept: Vec<String>,
    /// Each file that `--keep files` kept as the project's own.
    pub own: Vec<String>,
    /// Each pack that took over a file of the removed pack, with what the
    /// command did to each file.
    pub handed: Vec<SyncedPack>,
    /// What the command did with pakf's block: the report and the exit
    /// code of agent-harness-adapter.
    pub instructions: crate::Instructions,
}

impl PackRemoved {
    /// `conflicted` returns whether the merge of a file that the command
    /// handed to another pack conflicts.
    pub fn conflicted(&self) -> bool {
        self.handed.iter().any(|p| {
            p.files
                .iter()
                .any(|(_, a)| *a == pack::FileAction::Conflicted)
        })
    }

    /// `to_text` returns a line for each file, one for the configuration,
    /// and one for the lock.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for p in &self.removed {
            let _ = writeln!(out, "removed {p}");
        }
        for p in &self.kept {
            let _ = writeln!(out, "kept {p}: edited");
        }
        for p in &self.own {
            let _ = writeln!(out, "kept {p}: the project's own");
        }
        if self.undeclared {
            let _ = writeln!(
                out,
                "removed the pack {} from /.pakf/config.toml",
                self.name
            );
        }
        if self.locked {
            let _ = writeln!(out, "removed the pack {} from the lock", self.name);
        }
        for p in &self.handed {
            for (path, action) in &p.files {
                let _ = write!(out, "{} {path}: from the pack {}", action.label(), p.name);
                let _ = match p.reasons.get(path) {
                    Some(why) => writeln!(out, "; {why}"),
                    None => writeln!(out),
                };
            }
            for note in &p.notes {
                let _ = writeln!(out, "{note}");
            }
        }
        if let Some(line) = &self.instructions.held {
            let _ = writeln!(out, "{line}");
        }
        for note in fixed_notes(&self.handed) {
            let _ = writeln!(out, "note: {note}");
        }
        // sokf-implements: INST-1_AC-16
        out.push_str(&self.instructions.report);
        out
    }
}

/// `locked` returns the lock and the entry of the pack `name`, and refuses a
/// name that the lock doesn't hold.
pub(super) fn locked(root: &RepoRoot, name: &str) -> Result<(pack::Lock, pack::LockedPack)> {
    let lock = pack::read_lock(root)?;
    let entry = lock.packs.get(name).cloned().ok_or_else(|| {
        let names: Vec<&str> = lock.packs.keys().map(String::as_str).collect();
        Error::Pack(format!(
            "the lock holds no pack named `{name}`; it holds {}",
            if names.is_empty() {
                "none".to_string()
            } else {
                names.join(", ")
            }
        ))
    })?;
    Ok((lock, entry))
}

/// A `Removal` is what the removal of one locked pack deletes and keeps.
#[derive(Debug, Clone, Default)]
pub(super) struct Removal {
    /// The files and the base copies to delete.
    pub deletes: Vec<RepoPath>,
    /// Each file of the pack that the removal deletes and that exists.
    pub removed: Vec<String>,
    /// Each edited file that the removal keeps.
    pub kept: Vec<String>,
    /// Each file that `--keep files` keeps as the project's own and that
    /// exists.
    pub own: Vec<String>,
    /// The copy of the pack's `instructions.md` whose text the removal hands
    /// to agent-harness-adapter to keep, which stays until the adapter has
    /// taken the text.
    pub handed: Option<RepoPath>,
}

impl Removal {
    /// `settle` deletes the copy that the removal held back once
    /// agent-harness-adapter has taken its text, as `taken` says, whether or
    /// not the project edited it, because the text then stands where the
    /// adapter keeps it. It keeps the copy as the project's own after a
    /// refusal, or when the text never reached the adapter, so that the text
    /// stays in the project.
    // sokf-implements: CLI-7_AC-23
    // sokf-implements: CLI-7_AC-24
    pub(super) fn settle(&mut self, root: &RepoRoot, taken: bool) -> Result<()> {
        let Some(copy) = self.handed.take() else {
            return Ok(());
        };
        if taken {
            remove(root, &copy)?;
            self.removed.push(copy.as_str().to_string());
        } else {
            self.own.push(copy.as_str().to_string());
        }
        Ok(())
    }
}

/// `plan_removal` plans the removal of the locked pack `name`: it deletes
/// each file that holds its base, as `edited_files` says, keeps each other
/// file, and deletes the base copies. The copy of the pack's instructions
/// fragment is a file of the pack like any other, so an edited copy stays.
/// A file that another pack owns, or whose destination is one of
/// `owned_dests`, stays for that pack, as `stays_for_owner` says. It writes
/// nothing.
///
/// With `keep`, `--keep files` leaves each other file in place, apart from
/// the copy of the pack's instructions, which goes as it goes without
/// `--keep`. With `hands`, the removal hands the copy's text to
/// agent-harness-adapter, so it holds the copy back for `settle`.
// sokf-implements: CLI-7_AC-1
// sokf-implements: CLI-7_AC-2
// sokf-implements: CLI-7_AC-6
// sokf-implements: CLI-7_AC-8
// sokf-implements: CLI-7_AC-9
pub(super) fn plan_removal(
    root: &RepoRoot,
    name: &str,
    entry: &pack::LockedPack,
    owned_dests: &BTreeSet<RepoPath>,
    keep: PackKeep,
    hands: bool,
) -> Removal {
    let edited = edited_files(root, entry);
    let mut out = Removal::default();
    for (rel, f) in &entry.files {
        let Some(p) = RepoPath::parse(&f.path) else {
            continue;
        };
        if pack::stays_for_owner(f, &p, owned_dests) {
            out.deletes.push(pack::base_path(name, rel));
            continue;
        }
        let exists = !pack::is_absent(&p.to_fs(root));
        if hands && p == pack::instructions_of(name) {
            out.handed = exists.then_some(p);
            out.deletes.push(pack::base_path(name, rel));
            continue;
        }
        let own = keep.files && p != pack::instructions_of(name);
        if own {
            if exists {
                out.own.push(f.path.clone());
            }
        } else if edited.contains(&f.path) {
            if exists {
                out.kept.push(f.path.clone());
            }
        } else {
            if exists {
                out.removed.push(f.path.clone());
            }
            out.deletes.push(p);
        }
        out.deletes.push(pack::base_path(name, rel));
    }
    out
}

/// `apply_removal` deletes the files of `removal`, with each directory that
/// a deletion leaves empty.
pub(super) fn apply_removal(root: &RepoRoot, removal: &Removal) -> Result<()> {
    for p in &removal.deletes {
        remove(root, p)?;
    }
    Ok(())
}

/// `pack_remove` deletes the `[[pack]]` entry of the pack `name` from
/// `/.pakf/config.toml`, removes the pack as `plan_removal` says when the
/// lock holds it, hands each of its files whose destination another
/// declared pack owns to that pack, which merges an edited one, and drops
/// it from the lock. It deletes the lock when no pack remains. It refuses
/// a name that neither the configuration nor the lock holds, and a pack
/// that another pack of the lock depends on. It uses no network.
pub fn pack_remove(root: &RepoRoot, name: &str) -> Result<PackRemoved> {
    pack_remove_keeping(root, name, PackKeep::default())
}

/// `pack_remove_keeping` is `pack_remove` that keeps the parts `keep` of the
/// pack as the project's own: with `files`, every file of the pack stays.
/// With `instructions`, it refuses before it writes any file a copy of the
/// `instructions.md` of a pack of the lock that holds a conflict or isn't
/// readable, and hands the text of the pack's copy to agent-harness-adapter
/// to keep. The removal drops the pack from the lock before the block is
/// handed over, so the pack's text leaves the block.
// sokf-implements: CLI-7_AC-3
// sokf-implements: CLI-7_AC-4
// sokf-implements: CLI-7_AC-21
pub fn pack_remove_keeping(root: &RepoRoot, name: &str, keep: PackKeep) -> Result<PackRemoved> {
    let config_path = config::config_path();
    let text = read_optional(&config_path.to_fs(root))?;
    let config = match &text {
        Some(t) => config::remove_entry(t, name)?,
        None => None,
    };
    let lock = pack::read_lock(root)?;
    let locked = lock.packs.contains_key(name);
    // The refusal of a name that names no pack comes before the refusal of
    // a part, and before any write.
    // sokf-implements: CLI-7_AC-22
    if config.is_none() && !locked {
        // `locked` gives the error that lists the packs of the lock.
        self::locked(root, name)?;
    }
    let dependents = super::pack_run::dependents(&lock, name);
    if !dependents.is_empty() {
        return Err(Error::Pack(format!(
            "{name}: the pack {} depends on it; remove that pack first",
            dependents.join(", ")
        )));
    }
    // Without `instructions`, the pack's text goes with the pack.
    // sokf-implements: CLI-7_AC-17
    let kept_text = if keep.instructions {
        crate::harness::kept_text(root, &lock, name)?
    } else {
        None
    };
    let entries: Vec<config::PackEntry> = config::read_packs(root)?
        .into_iter()
        .filter(|e| e.name != name)
        .collect();
    let (report, kept_parts) = super::pack_run::run_keeping(
        root,
        &super::pack_run::Run {
            entries: &entries,
            fetching: super::pack_run::Fetching::Nothing,
            remove_only: Some(name),
            keep_existing: false,
            config: config.clone(),
        },
        keep,
        kept_text.as_deref(),
        None,
    )?;
    let (removed, kept) = report
        .undeclared
        .into_iter()
        .find(|u| u.name == name)
        .map(|u| (u.removed, u.kept))
        .unwrap_or_default();
    Ok(PackRemoved {
        name: name.to_string(),
        undeclared: config.is_some(),
        locked,
        removed,
        kept,
        own: kept_parts.own,
        handed: report.packs,
        instructions: report.instructions,
    })
}
