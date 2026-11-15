//! This module reads what the new version of a pack holds of each file that
//! the lock records: the file at its recorded destination, the file at
//! another destination that the remaps of the entry moved it to, or no file
//! (the decision "A moved destination" of design-pack).

use std::fs;

use super::Fetched;
use crate::{
    RepoPath, RepoRoot,
    pack::{lock::LockedFile, source::destination},
};

/// A `Held` is what the pack's new version holds of a file that the lock
/// records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Held {
    /// The file, at its recorded destination, or at one that differs from it
    /// only in the case of its letters where both name one entry or none.
    Same,
    /// The file, at another destination, because the remaps of the entry
    /// changed.
    Moved,
    /// No file at that path of the pack.
    Dropped,
}

/// `held` returns what the pack `fetched`, named `name`, holds of its file
/// `rel` that `f` records. A destination that differs from the recorded one
/// only in the case of its letters is the same destination where both name
/// one entry, as on a filesystem that ignores the case, or neither names
/// one, and another destination otherwise. Each step of the plan reads a
/// move of the file from this one function, so that the deletions and the
/// plan agree.
pub(super) fn held(
    root: &RepoRoot,
    name: &str,
    fetched: &Fetched,
    rel: &str,
    f: &LockedFile,
) -> Held {
    let dest = fetched
        .pack
        .files
        .contains_key(rel)
        .then(|| destination(name, &fetched.remap, rel))
        .flatten();
    match dest {
        None => Held::Dropped,
        Some(d) if d == f.path => Held::Same,
        Some(d) if d.to_lowercase() == f.path.to_lowercase() && same_entry(root, &d, &f.path) => {
            Held::Same
        }
        Some(_) => Held::Moved,
    }
}

/// `same_entry` returns whether the two destinations `a` and `b` name one
/// entry of the working tree, or neither names one.
fn same_entry(root: &RepoRoot, a: &str, b: &str) -> bool {
    let meta = |p: &str| RepoPath::parse(p).and_then(|p| fs::symlink_metadata(p.to_fs(root)).ok());
    match (meta(a), meta(b)) {
        (None, None) => true,
        #[cfg(unix)]
        (Some(x), Some(y)) => {
            use std::os::unix::fs::MetadataExt;
            (x.dev(), x.ino()) == (y.dev(), y.ino())
        }
        // Windows opens a path in any case of its letters.
        #[cfg(not(unix))]
        (Some(_), Some(_)) => cfg!(windows),
        _ => false,
    }
}
