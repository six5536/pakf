//! This module decides what a sync does to a file that the lock records and
//! that its pack no longer holds: it deletes the file where it is unedited,
//! as the test of an edit reads it, and keeps it otherwise.

use std::collections::BTreeSet;

use super::{
    Fetched, FileAction,
    moved::{Held, held},
};
use crate::{
    RepoPath, RepoRoot,
    pack::{
        dest::{Dest, read_dest},
        lock::{LockedFile, LockedPack, base_path},
        meaning::Meaning,
        mode,
    },
};

/// `dropping` returns the destination of each file that `existing`, the
/// lock's entry of the pack `fetched`, records, that the pack no longer
/// holds at that destination, and that its sync deletes, as
/// `remove_dropped` and `dropped` say, so that the plan of every pack reads
/// it as deleted before the sync writes.
pub fn dropping(
    root: &RepoRoot,
    meaning: &Meaning,
    existing: Option<&LockedPack>,
    fetched: &Fetched,
    owned_dests: &BTreeSet<RepoPath>,
) -> BTreeSet<RepoPath> {
    let name = &fetched.pack.manifest.name;
    existing
        .into_iter()
        .flat_map(|e| &e.files)
        .filter(|(rel, f)| {
            matches!(
                held(root, name, fetched, rel, f),
                Held::Moved | Held::Dropped
            )
        })
        .filter_map(|(rel, f)| dropped(root, meaning, name, rel, f, owned_dests))
        .filter(|(_, action)| *action == FileAction::Removed)
        .map(|(dest, _)| dest)
        .collect()
}

/// `dropped` returns what a sync does to the file `rel` that `f` records,
/// which its pack no longer holds: it removes the file where it holds its
/// base, as `LockedFile::holds_base` says, its executable bit among it, and
/// keeps it otherwise, as the removal of a pack does.
/// It returns none, and the sync reports nothing of the file, when the file
/// is absent, when its destination is one of `owned_dests`, which a
/// declared pack owns, because the file then passes to that pack, whose
/// plan decides it, and when another pack shadowed the pack's version, as
/// `stays_for_owner` says and the removal of a pack does too.
// sokf-implements: CLI-2_AC-50
// sokf-implements: CLI-2_AC-51
// sokf-implements: CLI-2_AC-53
// sokf-implements: CLI-2_AC-54
// sokf-implements: CLI-2_AC-67
pub(super) fn dropped(
    root: &RepoRoot,
    meaning: &Meaning,
    name: &str,
    rel: &str,
    f: &LockedFile,
    owned_dests: &BTreeSet<RepoPath>,
) -> Option<(RepoPath, FileAction)> {
    let dest = RepoPath::parse(&f.path)?;
    if super::stays_for_owner(f, &dest, owned_dests) {
        return None;
    }
    // An entry that isn't a readable regular file is an edit, so it stays.
    // sokf-implements: CLI-2_AC-58
    let path = dest.to_fs(root);
    let cur = read_dest(root, &dest);
    if cur == Dest::Absent {
        return None;
    }
    let bit = mode::carried_bit(rel, &cur, &path);
    let base_copy = base_path(name, rel).to_fs(root);
    let action = if f.holds_base(&cur, bit, &base_copy, meaning) {
        FileAction::Removed
    } else {
        FileAction::Kept
    };
    Some((dest, action))
}
