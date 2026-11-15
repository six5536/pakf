//! This module implements `pakf diff`: the project's edits of
//! the files of packs, which it shows and never writes. `pakf patch`, in
//! `pack_patch`, takes its edits from the same function. It uses no network.

use std::{fmt::Write as _, fs, path::PathBuf};

use super::{
    pack::{edited_files, holds_accepted},
    pack_info::locked,
};
use crate::{Error, RepoPath, RepoRoot, Result, config, pack};

/// `PackDiffOptions` holds the options of `pakf diff`.
#[derive(Debug, Clone, Default)]
pub struct PackDiffOptions {
    /// The pack's name, or none for every pack in the lock.
    pub name: Option<String>,
    /// Whether to print the edits that the project accepted too.
    pub accepted: bool,
    /// The files and directories of the pack whose edits to print, as the
    /// user wrote them, or none for every file of the pack.
    pub paths: Vec<String>,
    /// The directory from which a relative path of `paths` is read.
    pub cwd: PathBuf,
}

/// A `PackDiff` is what `pakf diff` prints: the patch, and the notes for a
/// person, which go to stderr.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackDiff {
    /// The unified diff of the edits.
    pub patch: String,
    /// A note for each edited file that the patch leaves out, because an
    /// entry that isn't a readable regular file stands at its path.
    pub notes: Vec<String>,
}

/// `pack_diff_with_notes` returns the edits of the pack `options.name`, or of
/// each pack in the lock in the order of the packs, as one unified diff, as
/// `edits` finds them. With `options.paths`, it keeps the edits of the files
/// that the pack owns at each path or below it, and refuses, with every
/// reason at once, a path at or below which the pack owns no file, and
/// paths without a name. It writes no file.
// sokf-implements: CLI-8_AC-8
// sokf-implements: CLI-8_AC-9
// sokf-implements: CLI-8_AC-10
// sokf-implements: CLI-8_AC-11
pub fn pack_diff_with_notes(root: &RepoRoot, options: &PackDiffOptions) -> Result<PackDiff> {
    let Some(name) = options.name.as_deref() else {
        if !options.paths.is_empty() {
            return Err(Error::Pack(
                "the paths of `pakf diff` need the name of their pack".into(),
            ));
        }
        let lock = pack::read_lock(root)?;
        let mut out = PackDiff::default();
        for name in pack::ordered(root, &lock) {
            let Some(entry) = lock.packs.get(&name) else {
                continue;
            };
            let e = edits(root, &name, entry, options.accepted, Purpose::Diff, &|_| {
                true
            })?;
            out.patch.push_str(&e.patch);
            out.notes.extend(e.notes);
        }
        return Ok(out);
    };
    let (_, entry) = locked_by_name(root, name, "pakf diff")?;
    let mut paths = Vec::new();
    let mut refused = Vec::new();
    for arg in &options.paths {
        let Some(dest) = config::PathArg(arg).resolve_new(root, &options.cwd) else {
            refused.push(format!("{arg}: not a path in the repository"));
            continue;
        };
        let owns = entry
            .files
            .values()
            .filter(|f| f.shadowed_by.is_none())
            .any(|f| below(&f.path, &dest));
        if owns {
            paths.push(dest);
        } else {
            refused.push(format!(
                "{dest}: the pack {name} holds no file at the path or below it"
            ));
        }
    }
    if !refused.is_empty() {
        return Err(Error::Pack(refused.join("; ")));
    }
    let e = edits(
        root,
        name,
        &entry,
        options.accepted,
        Purpose::Diff,
        &|dest| paths.is_empty() || paths.iter().any(|p| below(dest, p)),
    )?;
    Ok(PackDiff {
        patch: e.patch,
        notes: e.notes,
    })
}

/// `locked_by_name` returns the lock and the record of the pack `name`, as
/// `locked` does. A path in the place of the name reads as the name, so the
/// refusal says that a path of `command` follows the name of its pack.
pub(super) fn locked_by_name(
    root: &RepoRoot,
    name: &str,
    command: &str,
) -> Result<(pack::Lock, pack::LockedPack)> {
    locked(root, name).map_err(|e| match e {
        Error::Pack(why) if name.contains('/') && !pack::is_pack_id(name) => Error::Pack(format!(
            "{why}; a path of `{command}` follows the name of its pack"
        )),
        e => e,
    })
}

/// `below` returns whether the destination `dest` is the path `path` or
/// lies below it.
fn below(dest: &str, path: &RepoPath) -> bool {
    RepoPath::parse(dest).is_some_and(|d| d.starts_with(path))
}

/// `Edits` is the patch of the edits of one pack.
#[derive(Debug, Default)]
pub(super) struct Edits {
    /// The unified diff of the edits.
    pub patch: String,
    /// A note for each edited file that the patch leaves out.
    pub notes: Vec<String>,
    /// The path in the pack of each file that the patch holds.
    pub rels: Vec<String>,
}

/// `edits` returns the edits of the pack `name`, whose record is `entry`, of
/// the files whose destination `select` keeps: for each edited file, the
/// difference between its base copy and the project's copy, with paths
/// relative to the pack's directory, and with each CRLF of either read as
/// LF, except a file whose version in the pack holds CRLF: its diff runs
/// from that version, and reads each line ending of the project's copy as
/// CRLF where the version ends each line so. The diff of a file of
/// `files/` carries the recorded and the current executable bit. A file
/// that holds its accepted version is in the diff only with `accepted`,
/// because the patch carries what goes back to the pack. It reads the project's copy by the one reading of a destination, and
/// leaves out, with a note that names `what`, the diff or the patch, a file
/// at whose path stands an entry that isn't a readable regular file, so
/// that the patch never deletes such a file. A file whose copy or base copy
/// is binary goes into the patch as a git binary patch, and into the diff as
/// the line `Binary file <PATH> differs`, with no content, where its bytes
/// differ from the base copy's.
// sokf-implements: CLI-8_AC-1
// sokf-implements: CLI-8_AC-2
// sokf-implements: CLI-8_AC-4
// sokf-implements: CLI-8_AC-6
// sokf-implements: CLI-8_AC-7
// sokf-implements: CLI-8_AC-14
// sokf-implements: CLI-8_AC-3
pub(super) fn edits(
    root: &RepoRoot,
    name: &str,
    entry: &pack::LockedPack,
    accepted: bool,
    what: Purpose,
    select: &dyn Fn(&str) -> bool,
) -> Result<Edits> {
    let edited = edited_files(root, entry);
    let mut out = Edits::default();
    for (rel, f) in &entry.files {
        if !edited.contains(&f.path)
            || !select(&f.path)
            || (!accepted && holds_accepted(root, rel, f))
        {
            continue;
        }
        let dest = RepoPath::parse(&f.path).map(|p| p.to_fs(root));
        let current = match dest.as_deref().map_or(pack::Dest::Absent, pack::read_dest) {
            pack::Dest::Unreadable(why) => {
                out.notes.push(format!(
                    "{}; the {} leaves it out",
                    cant_read(&f.path, &why),
                    what.word()
                ));
                continue;
            }
            read => read.into_bytes(),
        };
        let base_path = pack::base_path(name, rel).to_fs(root);
        let base = fs::read(&base_path).map_err(|e| Error::io(&base_path, e))?;
        // A pack's version that holds CRLF, which a pack commits without a
        // `text` attribute, under `-text`, or with a lone carriage return,
        // is the preimage. The copy
        // is read with CRLF where the version ends every line so, whatever
        // a checkout wrote, and as it stands otherwise, so that only the
        // copy's edits differ. Otherwise the pack's repository holds LF, so
        // the patch compares the files with each CRLF read as LF, as git's
        // checkout converts them, and a file with a NUL byte by its bytes.
        // sokf-implements: CLI-8_AC-13
        let version = pack::committed(&base, &f.base)
            .map(|r| r.read(&base).into_owned())
            .filter(|v| pack::has_crlf(v));
        let (base, current) = match version {
            Some(v) => {
                let copy = if *pack::Reading::Crlf.read(&v) == *v {
                    pack::Reading::Crlf
                } else {
                    pack::Reading::Bytes
                };
                let current = current.as_deref().map(|c| copy.read(c).into_owned());
                (v, current)
            }
            None => (
                pack::normalise(&base).into_owned(),
                current.as_deref().map(|c| pack::normalise(c).into_owned()),
            ),
        };
        // A file of `files/` carries its executable bit, which the lock
        // records, so the patch carries a change of the bit too.
        // sokf-implements: CLI-8_AC-5
        let modes = dest
            .as_deref()
            .filter(|_| pack::carries_mode(rel))
            .map(|p| (f.executable, pack::file_is_executable(p)));
        let binary = pack::is_binary(&base) || current.as_deref().is_some_and(pack::is_binary);
        // The pack refuses a binary file that pakf reads as text, so the
        // patch never sends one.
        // sokf-implements: CLI-9_AC-2
        if what == Purpose::Patch
            && pack::is_text(rel)
            && current.as_deref().is_some_and(pack::is_binary)
        {
            return Err(Error::Pack(format!(
                "{}: the file is binary, and the pack's {rel} is a file that pakf reads as text",
                f.path
            )));
        }
        if binary && what == Purpose::Diff && current.as_deref() != Some(&base[..]) {
            // A change of the executable bit of a file that stands keeps
            // git's mode lines, which show no content.
            if let Some((old, new)) = modes.filter(|(old, new)| current.is_some() && old != new) {
                let mode = |x: bool| if x { "100755" } else { "100644" };
                let _ = write!(
                    out.patch,
                    "diff --git a/{rel} b/{rel}\nold mode {}\nnew mode {}\n",
                    mode(old),
                    mode(new)
                );
            }
            out.rels.push(rel.clone());
            let _ = writeln!(out.patch, "Binary file {rel} differs");
            continue;
        }
        let diff = pack::diff_modes(rel, Some(&base), current.as_deref(), modes)?;
        if !diff.is_empty() {
            out.rels.push(rel.clone());
        }
        out.patch.push_str(&diff);
    }
    Ok(out)
}

/// A `Purpose` is what `edits` makes: the diff of `pakf diff`, which shows
/// the edits, or the patch of `pakf patch`, which sends them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Purpose {
    /// The diff of `pakf diff`.
    Diff,
    /// The patch of `pakf patch`.
    Patch,
}

impl Purpose {
    /// `word` returns the word of a note for the purpose.
    fn word(self) -> &'static str {
        match self {
            Purpose::Diff => "diff",
            Purpose::Patch => "patch",
        }
    }
}

/// `cant_read` returns what `pakf diff` and `pakf patch` say of the path
/// `path`, at which stands an entry that isn't a readable regular file, for
/// the reason `why`, in the note of an edited file and in the refusal of a
/// path of `pakf patch`.
pub(super) fn cant_read(path: &str, why: &str) -> String {
    format!("{path}: the file can't be read: {why}")
}
