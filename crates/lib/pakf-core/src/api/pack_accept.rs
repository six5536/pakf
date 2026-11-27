//! This module implements `pakf accept`, which records that a project
//! keeps an edit or a deletion of a pack's file on purpose, so that `pack
//! status --check` passes on the file. It writes only the lock and
//! uses no network.

use std::{fmt::Write as _, path::PathBuf};

use super::pack::{is_edited, write_lock};
use crate::{
    Error, RepoPath, RepoRoot, Result,
    config::PathArg,
    pack::{self, Accepted},
};

/// `PackAcceptOptions` holds the options of `pakf accept`.
#[derive(Debug, Clone, Default)]
pub struct PackAcceptOptions {
    /// The files to accept, or whose acceptance to withdraw, as the user
    /// wrote them.
    pub paths: Vec<String>,
    /// Why the project keeps the edits.
    pub note: Option<String>,
    /// Whether to withdraw the acceptances instead.
    pub withdraw: bool,
    /// The directory from which a relative path is read.
    pub cwd: PathBuf,
}

/// A `PackAccepted` is the report of `pakf accept`.
#[derive(Debug, Clone, Default)]
pub struct PackAccepted {
    /// Each file whose edit the command accepted.
    pub accepted: Vec<String>,
    /// Each file whose acceptance the command withdrew.
    pub withdrawn: Vec<String>,
    /// The note that the command recorded with each acceptance, or none
    /// with `--withdraw`.
    pub note: Option<String>,
    /// A warning for each formatter or command of `[compare]` that was
    /// missing, failed, or ran past its limit, which the command prints on
    /// stderr, outside its JSON.
    pub warnings: Vec<String>,
}

impl PackAccepted {
    /// `to_text` returns a line for each file.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for p in &self.accepted {
            let _ = writeln!(out, "accepted {p}");
        }
        for p in &self.withdrawn {
            let _ = writeln!(out, "withdrew the acceptance of {p}");
        }
        if out.is_empty() {
            out.push_str("no acceptance changed\n");
        }
        out
    }
}

/// `unedited` returns the refusal of the unedited file at `dest`, the file
/// `rel` of the pack `name` whose record is `record`: that it holds the
/// pack's version, or, where its bytes differ from the base copy's, that it
/// means the same.
fn unedited(
    root: &RepoRoot,
    dest: &RepoPath,
    name: &str,
    rel: &str,
    record: &pack::LockedFile,
) -> String {
    let laid = match pack::read_dest(root, dest) {
        pack::Dest::File(b) => {
            !pack::is_base(&b, &record.base)
                && std::fs::read(pack::base_path(name, rel).to_fs(root))
                    .is_ok_and(|base| pack::normalise(&base) != pack::normalise(&b))
        }
        _ => false,
    };
    if laid {
        format!("{dest}: the file means the same as the pack's version")
    } else {
        format!("{dest}: the file holds the pack's version")
    }
}

/// `pack_accept` records in the lock that the project keeps each file of
/// `options.paths` as it stands, with the note, or withdraws the acceptance
/// of each. It refuses, before it writes, a missing note, a path that no
/// pack in the lock owns, a file that holds its base, a file that holds a
/// conflict, and the edit of an editable file.
// sokf-implements: CLI-6_AC-1
// sokf-implements: CLI-6_AC-2
// sokf-implements: CLI-6_AC-3
// sokf-implements: CLI-6_AC-4
// sokf-implements: CLI-6_AC-5
// sokf-implements: CLI-6_AC-6
pub fn pack_accept(root: &RepoRoot, options: &PackAcceptOptions) -> Result<PackAccepted> {
    let note = options.note.as_deref().map(str::trim).unwrap_or_default();
    if !options.withdraw && note.is_empty() {
        return Err(Error::Pack(
            "an acceptance needs a note; pass --note with the reason that the project keeps the edit"
                .into(),
        ));
    }
    // The lock is the one file that an acceptance writes, and the
    // configuration sets its comparison.
    // sokf-implements: CLI-2_AC-90
    super::pack::refuse_own_links(root)?;
    let mut lock = pack::read_lock(root)?;
    // An acceptance is a line of the lock, so it is recorded by the
    // library's own reading alone; the reading of `pakf status` is opened
    // only for a file that the first reads as unedited, so that `pakf
    // accept` never refuses a file that `pakf status` reports as edited.
    let meaning = pack::Meaning::open(root, pack::For::Writes)?;
    let reports = pack::Meaning::open(root, pack::For::Reports)?;
    let accepted = accept(root, options, note, &mut lock, &meaning, &reports);
    meaning.finish();
    reports.settle(accepted, |r, w| r.warnings = w)
}

/// `accept` records the acceptances of `pack_accept` in the lock `lock`,
/// with the library's own reading `meaning` and the reading of `pakf
/// status` `reports`, and returns the report without its warnings, which
/// `pack_accept` adds.
fn accept(
    root: &RepoRoot,
    options: &PackAcceptOptions,
    note: &str,
    lock: &mut pack::Lock,
    meaning: &pack::Meaning,
    reports: &pack::Meaning,
) -> Result<PackAccepted> {
    let mut refused = Vec::new();
    let mut targets: Vec<(RepoPath, String, String, Option<Accepted>)> = Vec::new();
    for arg in &options.paths {
        let Some(dest) = PathArg(arg).resolve_new(root, &options.cwd) else {
            refused.push(format!("{arg}: not a path in the repository"));
            continue;
        };
        let Some((name, rel)) = lock
            .owner_of(dest.as_str())
            .map(|(n, r)| (n.to_string(), r.to_string()))
        else {
            refused.push(format!("{dest}: no pack in the lock owns the file"));
            continue;
        };
        // A file that two arguments name is accepted once.
        if targets.iter().any(|(d, ..)| *d == dest) {
            continue;
        }
        if options.withdraw {
            targets.push((dest, name, rel, None));
            continue;
        }
        let record = &lock.packs[&name].files[&rel];
        // sokf-implements: CLI-1_AC-43
        if !is_edited(root, meaning, &name, &rel, record)
            && !is_edited(root, reports, &name, &rel, record)
        {
            refused.push(unedited(root, &dest, &name, &rel, record));
            continue;
        }
        let path = dest.to_fs(root);
        // An entry that isn't a readable regular file is no deletion, and
        // holds no version to accept.
        // sokf-implements: CLI-2_AC-57
        // sokf-implements: CLI-6_AC-9
        let bytes = match pack::read_dest(root, &dest) {
            pack::Dest::File(b) => Some(b),
            pack::Dest::Absent => None,
            pack::Dest::Unreadable(why) => {
                refused.push(format!("{dest}: the file can't be read: {why}"));
                continue;
            }
        };
        // sokf-implements: CLI-6_AC-4
        if let Some(b) = &bytes
            && record.holds_conflict(root, &name, &rel, b)
        {
            refused.push(format!(
                "{dest}: the file holds a conflict; resolve it, then accept it"
            ));
            continue;
        }
        // An editable file's edit needs no acceptance, and its deletion
        // still does.
        // sokf-implements: CLI-6_AC-10
        if bytes.is_some() && lock.packs[&name].files[&rel].editable {
            refused.push(format!(
                "{dest}: the file is editable in the pack {name} and needs no acceptance"
            ));
            continue;
        }
        let bit = bytes.is_some() && pack::carries_mode(&rel) && pack::file_is_executable(&path);
        let accepted = Accepted::of(bytes.as_deref(), bit, note, &dest, meaning);
        targets.push((dest, name, rel, Some(accepted)));
    }
    if !refused.is_empty() {
        return Err(Error::Pack(refused.join("; ")));
    }
    let mut report = PackAccepted {
        note: (!options.withdraw).then(|| note.to_string()),
        ..PackAccepted::default()
    };
    let mut changed = false;
    for (dest, name, rel, accepted) in targets {
        let file = lock
            .packs
            .get_mut(&name)
            .and_then(|p| p.files.get_mut(&rel))
            .ok_or_else(|| Error::Internal(format!("no record of {dest}")))?;
        match accepted {
            None if file.accepted.take().is_some() => {
                report.withdrawn.push(dest.to_string());
                changed = true;
            }
            None => {}
            Some(a) => {
                changed |= file.accepted.as_ref() != Some(&a);
                file.accepted = Some(a);
                report.accepted.push(dest.to_string());
            }
        }
    }
    if changed {
        write_lock(root, lock)?;
    }
    Ok(report)
}
