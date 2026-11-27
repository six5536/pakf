//! This module implements `pakf sync` and `pakf add`: it
//! reads the packs that `/.pakf/config.toml` declares, fetches each one,
//! plans the sync of all of them and the removal of each locked pack that
//! no entry declares, and writes the plans only when every one succeeds.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    path::{Path, PathBuf},
};

use super::pack_run::{Fetching, Run, run};
use crate::{
    Error, RepoPath, RepoRoot, Result,
    config::{self, PackEntry},
    pack::{self, Fetched, FileAction, Lock, PackPlan, Source},
    source::read_optional,
};

/// `PackSyncOptions` holds the options of `pakf sync`.
#[derive(Debug, Clone, Default)]
pub struct PackSyncOptions {
    /// The id of the one declared pack to sync, or `None` for every
    /// declared pack.
    pub name: Option<String>,
    /// Keep each existing file that a file of a pack lands on as an edit.
    pub keep_existing: bool,
}

/// `PackAddOptions` holds the options of `pakf add`.
#[derive(Debug, Clone, Default)]
pub struct PackAddOptions {
    /// The source: a git URL or path. A relative path resolves from `cwd`.
    pub source: String,
    /// The branch, tag, or commit of a git source.
    pub git_ref: Option<String>,
    /// The pack's directory in a git source.
    pub path: Option<String>,
    /// Keep each existing file that a file of the pack lands on as an edit.
    pub keep_existing: bool,
    /// The current directory, from which a relative `source` resolves.
    pub cwd: PathBuf,
}

/// A `SyncedPack` is the report of the sync of one pack.
#[derive(Debug, Clone)]
pub struct SyncedPack {
    /// The pack's id, `@scope/name`.
    pub name: String,
    /// The pack's version.
    pub version: String,
    /// The pack's source, as the lock records it.
    pub source: String,
    /// Each file that the sync changed or kept, with what it did.
    pub files: Vec<(String, FileAction)>,
    /// The reason that the line of a file gives after its path, by path,
    /// such as that the project and the pack both changed a binary file.
    pub reasons: BTreeMap<String, String>,
    /// A note for each file whose shadowed version the pack changed.
    pub notes: Vec<String>,
    /// Each file whose record the sync dropped and that it kept while the
    /// file holds a conflict.
    pub still_conflicted: Vec<String>,
    /// Whether the sync fetched the pack, or read it from its base copies
    /// because only the owners of its files changed.
    pub fetched: bool,
}

/// An `UndeclaredPack` is the report of the removal of a locked pack that
/// no `[[pack]]` entry declares.
#[derive(Debug, Clone)]
pub struct UndeclaredPack {
    /// The pack's id, `@scope/name`.
    pub name: String,
    /// Each file that the sync deleted.
    pub removed: Vec<String>,
    /// Each edited file that the sync kept.
    pub kept: Vec<String>,
    /// Each kept file that holds a conflict.
    pub still_conflicted: Vec<String>,
}

/// `DELETED_NOTE` is the note of the text of a sync or a removal that keeps
/// a deleted file deleted.
pub(super) const DELETED_NOTE: &str = "the deleted file stays deleted; to restore it, copy its base copy from /.pakf/packs/<ID>/base/ back";

/// A `PackSyncReport` is the report of `pakf sync` and `pakf add`.
#[derive(Debug, Clone)]
pub struct PackSyncReport {
    /// The pack that `pakf add` declared in the configuration.
    pub declared: Option<String>,
    /// What the command did with pakf's block: the report and the exit
    /// code of agent-harness-adapter.
    pub instructions: crate::Instructions,
    /// Each locked pack that no entry declares, which the sync removed.
    pub undeclared: Vec<UndeclaredPack>,
    /// Each pack that the sync wrote.
    pub packs: Vec<SyncedPack>,
}

impl PackSyncReport {
    /// `conflicted` returns whether a merge of the sync conflicts.
    pub fn conflicted(&self) -> bool {
        self.packs
            .iter()
            .any(|p| p.files.iter().any(|(_, a)| *a == FileAction::Conflicted))
    }

    /// `to_text` returns the report as text lines: a line for the pack that
    /// `pakf add` declared, a line for each pack that the sync removed or
    /// wrote, a line for each file, and the notes of `fixed_notes`.
    // sokf-implements: CLI-2_AC-9
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        if let Some(name) = &self.declared {
            let _ = writeln!(out, "declared the pack {name} in /.pakf/config.toml");
        }
        for u in &self.undeclared {
            let _ = writeln!(
                out,
                "removed the pack {}, which no [[pack]] entry declares",
                u.name
            );
            for p in &u.removed {
                let _ = writeln!(out, "removed {p}");
            }
            for p in &u.kept {
                let _ = writeln!(out, "kept {p}: edited");
            }
        }
        for p in &self.packs {
            if p.fetched {
                let _ = writeln!(out, "synced {} {} from {}", p.name, p.version, p.source);
            } else {
                let _ = writeln!(
                    out,
                    "changed the owners of the files of {} {}",
                    p.name, p.version
                );
            }
            for (path, action) in &p.files {
                let _ = match p.reasons.get(path) {
                    Some(why) => writeln!(out, "{} {path}: {why}", action.label()),
                    None => writeln!(out, "{} {path}", action.label()),
                };
            }
        }
        for p in &self.packs {
            for note in &p.notes {
                let _ = writeln!(out, "{note}");
            }
        }
        if let Some(line) = &self.instructions.held {
            let _ = writeln!(out, "{line}");
        }
        if self.conflicted() {
            out.push_str(
                "error: a merge conflicts: resolve the conflict markers in each conflicted file\n",
            );
        }
        for note in fixed_notes(&self.packs) {
            let _ = writeln!(out, "note: {note}");
        }
        if let Some(note) = self.still_conflicted_note() {
            let _ = writeln!(out, "note: {note}");
        }
        // sokf-implements: INST-1_AC-16
        out.push_str(&self.instructions.report);
        out
    }
}

impl PackSyncReport {
    /// `still_conflicted_note` returns the note of each file whose record
    /// the sync dropped and that still holds a conflict, as
    /// `still_conflicted_note` of the module says.
    pub(super) fn still_conflicted_note(&self) -> Option<String> {
        let undeclared = self.undeclared.iter().flat_map(|u| &u.still_conflicted);
        still_conflicted_note(undeclared.chain(self.packs.iter().flat_map(|p| &p.still_conflicted)))
    }
}

/// `still_conflicted_note` returns one note, without its `note: `, that
/// names each file of `paths`, which a command kept and whose record it
/// dropped while the file holds a conflict, such as `/a.md still holds a
/// conflict`, or none without such a file. pakf no longer reports the
/// conflict of a file that no pack owns, so the note says it once.
// sokf-implements: CLI-1_AC-48
pub(super) fn still_conflicted_note<'a>(paths: impl Iterator<Item = &'a String>) -> Option<String> {
    let paths: Vec<&str> = paths.map(String::as_str).collect();
    match paths.as_slice() {
        [] => None,
        [one] => Some(format!("{one} still holds a conflict")),
        [rest @ .., last] => Some(format!(
            "{} and {last} still hold a conflict",
            rest.join(", ")
        )),
    }
}

/// `fixed_notes` returns the notes that the text of a command that wrote
/// `packs` prints last: how to restore a deleted file, when the command kept
/// one deleted.
// sokf-implements: CLI-2_AC-18
pub(super) fn fixed_notes(packs: &[SyncedPack]) -> Vec<&'static str> {
    let deleted = packs
        .iter()
        .any(|p| p.files.iter().any(|(_, a)| *a == FileAction::Deleted));
    if deleted {
        vec![DELETED_NOTE]
    } else {
        Vec::new()
    }
}

/// `pack_sync` syncs the packs that `/.pakf/config.toml` declares: each
/// declared pack, or only the pack that `opts` names, from
/// the source, ref, and path of its entry, and removes each locked pack that
/// no entry declares. It writes no file unless the plan of every pack
/// succeeds.
// sokf-implements: CLI-2_AC-7
// sokf-implements: CLI-2_AC-10
// sokf-implements: CLI-2_AC-32
pub fn pack_sync(root: &RepoRoot, opts: &PackSyncOptions) -> Result<PackSyncReport> {
    refuse_own_links(root)?;
    let entries = config::read_packs(root)?;
    refuse_undeclared_lock(root, &entries)?;
    run(
        root,
        &Run {
            entries: &entries,
            fetching: match opts.name.as_deref() {
                Some(n) => Fetching::One(n),
                None => Fetching::All,
            },
            remove_only: None,
            keep_existing: opts.keep_existing,
            config: None,
        },
        None,
    )
}

/// `pack_add` declares the pack of a source in `/.pakf/config.toml` and
/// syncs it: it fetches the pack of the source, appends a `[[pack]]` entry for it at the
/// end of the list, with the remaps that the lock records for a pack that it
/// still holds, and syncs it. It refuses a pack whose name an entry
/// already declares, and writes the configuration only with the sync.
// sokf-implements: CLI-3_AC-1
// sokf-implements: CLI-3_AC-2
pub fn pack_add(root: &RepoRoot, opts: &PackAddOptions) -> Result<PackSyncReport> {
    refuse_own_links(root)?;
    // The lock's files are checked before the fetch.
    super::pack_run::refuse_links_out(root, &pack::read_lock(root)?, &[], true)?;
    let source = Source::Git(pack::recorded(&opts.source, root.as_path(), &opts.cwd));
    let fetched = fetch(root, &source, opts.git_ref.as_deref(), opts.path.as_deref())?;
    let name = fetched.pack.manifest.name.clone();
    let mut entries = config::read_packs(root)?;
    if entries.iter().any(|e| e.name == name) {
        return Err(Error::Pack(format!(
            "{name}: /.pakf/config.toml already declares the pack {name}; edit its [[pack]] entry to change its source"
        )));
    }
    refuse_undeclared_lock(root, &entries)?;
    // A pack that the lock still holds keeps the remaps of its last sync, so
    // that its files stay where they landed.
    let paths = pack::read_lock(root)?
        .packs
        .get(&name)
        .map(|p| p.paths.clone())
        .unwrap_or_default();
    let entry = PackEntry {
        name: name.clone(),
        source: source.label(),
        git_ref: opts.git_ref.clone(),
        path: fetched.path.clone(),
        paths,
    };
    let text = read_optional(&config::config_path().to_fs(root))?;
    let config = config::add_entry(text.as_deref(), &entry)?;
    entries.push(entry);
    let mut report = run(
        root,
        &Run {
            entries: &entries,
            fetching: Fetching::One(&name),
            remove_only: None,
            keep_existing: opts.keep_existing,
            config: Some(config),
        },
        Some(fetched),
    )?;
    report.declared = Some(name);
    Ok(report)
}

/// `refuse_own_links` refuses a command whose lock or configuration lies
/// below a symbolic link that leads out of the repository, or stands at one
/// or at a link that leads to no file, before the command reads either, so
/// that the refusal names the link, and no file outside the repository is
/// read as pakf's own.
// sokf-implements: CLI-2_AC-90
pub(super) fn refuse_own_links(root: &RepoRoot) -> Result<()> {
    pack::refuse_links_out(
        root,
        [(pack::lock_path(), true), (config::config_path(), true)],
    )
}

/// `refuse_undeclared_lock` refuses a sync while no `[[pack]]` entry
/// declares a pack and the lock holds one, as a lock that an earlier release
/// wrote does, because the sync would remove every locked pack. The error
/// gives an entry for each locked pack to add to the configuration.
// sokf-implements: CLI-2_AC-43
fn refuse_undeclared_lock(root: &RepoRoot, entries: &[PackEntry]) -> Result<()> {
    let lock = pack::read_lock(root)?;
    if !entries.is_empty() || lock.packs.is_empty() {
        return Ok(());
    }
    let mut text = String::new();
    for (name, p) in &lock.packs {
        text = config::add_entry(
            Some(&text),
            &PackEntry {
                name: name.clone(),
                source: p.source.clone(),
                // The entry follows the branch that the lock records.
                git_ref: p.git_ref.clone(),
                path: p.path.clone(),
                paths: p.paths.clone(),
            },
        )?;
    }
    Err(Error::Pack(format!(
        "no [[pack]] entry of /.pakf/config.toml declares a pack, and the lock holds {}; add these entries to /.pakf/config.toml to keep them, remove each with `pakf remove <NAME>`, or end the link with each and keep its files with `pakf remove <NAME> --keep files`:\n\n{text}",
        lock.packs.keys().cloned().collect::<Vec<_>>().join(", ")
    )))
}

/// `fetch` reads the pack of `source`. A git source is cloned at `git_ref` into a
/// temporary directory, from the root of the repository `root` when it is a
/// relative path, and the pack is read from its directory `path`.
pub(super) fn fetch(
    root: &RepoRoot,
    source: &Source,
    git_ref: Option<&str>,
    path: Option<&str>,
) -> Result<Fetched> {
    match source {
        Source::Git(url) => {
            let path = path
                .map(|p| p.trim_matches('/'))
                .filter(|p| !p.is_empty() && *p != ".");
            if let Some(p) = path
                && p.split('/').any(|s| s == "..")
            {
                return Err(Error::Pack(format!(
                    "--path {p}: the path leaves the repository"
                )));
            }
            let clone = pack::shallow_clone(&pack::from_root(url, root.as_path()), git_ref)?;
            let dir = match path {
                Some(p) => pack::clone_dir(&clone.worktree(), p)?,
                None => clone.worktree(),
            };
            let label = match path {
                Some(p) => format!("{url} at {p}"),
                None => url.clone(),
            };
            let mut files = pack::read_dir(&dir, &label)?;
            // git names each file from the clone's root without `.` or an
            // empty segment, which `clone_dir` also skips in `path`.
            let prefix: String = path
                .unwrap_or_default()
                .split('/')
                .filter(|s| !s.is_empty() && *s != ".")
                .fold(String::new(), |mut out, s| {
                    let _ = write!(out, "{s}/");
                    out
                });
            files.executable = clone
                .executables()?
                .into_iter()
                .filter_map(|p| p.strip_prefix(&prefix).map(str::to_string))
                .filter(|rel| rel.starts_with(pack::FILES) && files.files.contains_key(rel))
                .collect();
            Ok(Fetched {
                pack: files,
                source: source.clone(),
                git_ref: Some(clone.git_ref.clone()),
                path: path.map(str::to_string),
                commit: clone.commit.clone(),
                // The caller gives the remaps of the pack's entry.
                remap: pack::Remap::default(),
                offline: false,
            })
        }
    }
}

/// `apply` writes the plans and the lock. It deletes each file in the way
/// of a write, at the write's path, above it, or below it, before it writes
/// any file, so that a file of a pack that becomes a directory gives way to
/// the files below it, and a directory that becomes a file to the file. It
/// deletes every other file of the plans after the writes, so that a write
/// that fails leaves the fewest of them deleted. The files of a removed pack
/// are deleted before, by `apply_removal`.
// sokf-implements: CLI-2_AC-5
// sokf-implements: CLI-2_AC-65
// A sync writes each file as its plan holds it, and runs no formatter on it.
// sokf-implements: CLI-2_AC-88
pub(super) fn apply(root: &RepoRoot, plans: &[PackPlan], lock: &Lock) -> Result<()> {
    let writes: BTreeSet<&RepoPath> = plans
        .iter()
        .flat_map(|p| p.writes.iter().map(|(path, _)| path))
        .collect();
    let (first, last): (Vec<&RepoPath>, Vec<&RepoPath>) = plans
        .iter()
        .flat_map(|p| &p.deletes)
        .partition(|d| writes.iter().any(|w| w.starts_with(d) || d.starts_with(w)));
    for path in first {
        remove(root, path)?;
    }
    for plan in plans {
        for (path, bytes) in &plan.writes {
            write(root, path, bytes)?;
        }
        for (path, executable) in &plan.modes {
            let fs_path = path.to_fs(root);
            pack::set_executable(&fs_path, *executable).map_err(|e| Error::io(&fs_path, e))?;
        }
    }
    for path in last {
        remove(root, path)?;
    }
    write_lock(root, lock)
}

/// `write_lock` writes `lock`, or deletes the lock and the directory of the
/// packs when the lock holds no pack and nothing else is left in it.
pub(super) fn write_lock(root: &RepoRoot, lock: &Lock) -> Result<()> {
    if lock.packs.is_empty() {
        remove(root, &pack::lock_path())?;
        let _ = std::fs::remove_dir(root.as_path().join(pack::PACKS_DIR));
        return Ok(());
    }
    write(root, &pack::lock_path(), pack::lock_text(lock)?.as_bytes())
}

/// `write` writes `bytes` to the repository file `path`, through a temporary
/// file and a rename.
pub(super) fn write(root: &RepoRoot, path: &RepoPath, bytes: &[u8]) -> Result<()> {
    agent_harness_adapter_core::fs::write_atomic(&path.to_fs(root), bytes).map_err(Error::from)
}

/// `remove` deletes the repository file `path`, when it exists, and each
/// directory that it leaves empty: up to `/.pakf/packs/` for a file under
/// it, up to `/.pakf/` for any other file of pakf, which stays, and up to the
/// repository root for any other file.
// sokf-implements: CLI-2_AC-31
pub(super) fn remove(root: &RepoRoot, path: &RepoPath) -> Result<()> {
    let fs_path = path.to_fs(root);
    match std::fs::remove_file(&fs_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(Error::io(&fs_path, e)),
    }
    let packs = root.as_path().join(pack::PACKS_DIR);
    let pakf = root.as_path().join(".pakf");
    let stop = if fs_path.starts_with(&packs) {
        packs
    } else if fs_path.starts_with(&pakf) {
        pakf
    } else {
        root.as_path().to_path_buf()
    };
    let mut dir = fs_path.parent().map(Path::to_path_buf);
    while let Some(d) = dir {
        // A directory that isn't empty stays, and stops the climb.
        if d == stop || !d.starts_with(&stop) || std::fs::remove_dir(&d).is_err() {
            break;
        }
        dir = d.parent().map(Path::to_path_buf);
    }
    Ok(())
}

/// `edited_files` returns the destination of each file of the locked pack
/// `name` that is missing, whose content differs from its base, with each
/// CRLF read as LF, and from the meaning of its base copy, as `meaning`
/// reads it, or, for a file of `files/`, whose executable bit differs from
/// the bit that the lock records. A shadowed file holds another pack's
/// version, so it is never this pack's edit.
// sokf-implements: CLI-1_AC-25
// sokf-implements: CLI-8_AC-4
pub(super) fn edited_files(
    root: &RepoRoot,
    meaning: &pack::Meaning,
    name: &str,
    pack: &pack::LockedPack,
) -> BTreeSet<String> {
    pack.files
        .iter()
        .filter(|(rel, f)| is_edited(root, meaning, name, rel, f))
        .map(|(_, f)| f.path.clone())
        .collect()
}

/// `is_edited` returns whether the file `rel` of the locked pack `name`,
/// whose record is `f`, is one of the pack's edited files, as
/// `edited_files` says.
pub(super) fn is_edited(
    root: &RepoRoot,
    meaning: &pack::Meaning,
    name: &str,
    rel: &str,
    f: &pack::LockedFile,
) -> bool {
    if f.shadowed_by.is_some() {
        return false;
    }
    let Some(at) = RepoPath::parse(&f.path) else {
        return true;
    };
    let path = at.to_fs(root);
    // An absent file is a deletion, and an entry that isn't a readable
    // regular file is an edit.
    // sokf-implements: CLI-2_AC-58
    let dest = pack::read_dest(root, &at);
    let base_copy = pack::base_path(name, rel).to_fs(root);
    !f.holds_base(
        &dest,
        pack::carried_bit(rel, &dest, &path),
        &base_copy,
        meaning,
    )
}

/// `holds_accepted` returns whether the file `rel` of a locked pack, whose
/// record is `f`, holds the version that its acceptance records: the
/// accepted meaning, as `meaning` reads it, and the accepted executable bit
/// for a file of `files/`, or no file for an accepted deletion.
pub(super) fn holds_accepted(
    root: &RepoRoot,
    meaning: &pack::Meaning,
    rel: &str,
    f: &pack::LockedFile,
) -> bool {
    let Some(accepted) = &f.accepted else {
        return false;
    };
    let Some(at) = RepoPath::parse(&f.path) else {
        return accepted.holds(&pack::Dest::Absent, None, &RepoPath::root(), meaning);
    };
    let path = at.to_fs(root);
    let dest = pack::read_dest(root, &at);
    let bit = pack::carried_bit(rel, &dest, &path);
    accepted.holds(&dest, bit, &at, meaning)
}

#[cfg(test)]
mod tests {
    use super::still_conflicted_note;

    // sokf-tests: CLI-1_AC-48
    #[test]
    fn one_note_names_each_file_that_still_holds_a_conflict() {
        let note = |paths: &[&str]| {
            let paths: Vec<String> = paths.iter().map(|p| (*p).to_string()).collect();
            still_conflicted_note(paths.iter())
        };
        assert_eq!(note(&[]), None);
        assert_eq!(note(&["/a.md"]).unwrap(), "/a.md still holds a conflict");
        assert_eq!(
            note(&["/a.md", "/b.md", "/c.md"]).unwrap(),
            "/a.md, /b.md and /c.md still hold a conflict"
        );
    }
}
