//! This module follows the acceptance of a file of a pack through a sync:
//! it moves an acceptance to the file that a clean merge writes, and drops
//! one with a note where the sync changes what the acceptance covers.

use super::FileAction;
use crate::{
    RepoPath,
    pack::{eol::same_text, lock::Accepted},
};

/// A `Followed` is what a sync did to one file of a pack that has an
/// acceptance.
pub(super) struct Followed<'a> {
    /// The file's destination.
    pub(super) dest: &'a RepoPath,
    /// Whether the sync kept the file as a binary file that the project and
    /// the pack both changed.
    pub(super) binary_kept: bool,
    /// What the sync did to the file.
    pub(super) action: Option<FileAction>,
    /// Whether the file held its accepted version before the sync.
    pub(super) held: bool,
    /// The file's content after the sync, or none for a deleted file.
    pub(super) after: Option<&'a [u8]>,
    /// The file's executable bit after the sync, for a file of `files/`.
    pub(super) bit: Option<bool>,
    /// The pack's new version of the file.
    pub(super) upstream: &'a [u8],
    /// Whether the pack's new version is executable.
    pub(super) executable: bool,
    /// Whether the sync changed the pack's version of the file.
    pub(super) changed: bool,
}

/// `follow_acceptance` returns the acceptance `accepted` after the sync
/// that `f` describes, and pushes a note for each acceptance that it drops
/// or keeps against a change of the pack. A clean merge into the accepted
/// version moves the acceptance to the merged file, because the project's
/// lines are unchanged. A conflict, a merge into a file that changed since
/// its acceptance, a binary file that the sync keeps against the pack's
/// change, and a file that holds the pack's new version drop it.
// sokf-implements: CLI-2_AC-44
// sokf-implements: CLI-2_AC-45
// sokf-implements: CLI-2_AC-46
// sokf-implements: CLI-2_AC-47
// sokf-implements: CLI-2_AC-48
pub(super) fn follow_acceptance(
    accepted: Accepted,
    f: &Followed,
    notes: &mut Vec<String>,
) -> Option<Accepted> {
    let dest = f.dest;
    let holds_base = f.after.is_some_and(|b| same_text(b, f.upstream))
        && f.bit.is_none_or(|b| b == f.executable);
    if holds_base {
        return None;
    }
    match f.action {
        Some(FileAction::Conflicted) => {
            notes.push(format!(
                "note: the merge into {dest} conflicts, so its acceptance is dropped; accept it again with `pakf accept` after you resolve the conflict"
            ));
            None
        }
        // A binary file that the project and the pack both changed takes
        // its acceptance again against the pack's new version.
        // sokf-implements: CLI-2_AC-28
        _ if f.binary_kept => {
            notes.push(format!(
                "note: the pack changed the binary file {dest}, whose edit is accepted, so the acceptance is dropped; accept it again with `pakf accept` to keep your version"
            ));
            None
        }
        Some(FileAction::Merged) if !f.held => {
            notes.push(format!(
                "note: the sync merged the pack's change into {dest}, which changed since its acceptance, so the acceptance is dropped"
            ));
            None
        }
        _ => {
            if accepted.deleted && f.after.is_none() && f.changed {
                notes.push(format!(
                    "note: the pack changed {dest}, whose deletion is accepted; the file stays deleted"
                ));
            }
            // A file that held its accepted version follows the sync, as a
            // clean merge or the pack's change of the executable bit.
            Some(if f.held {
                Accepted::of(f.after, f.bit.unwrap_or(false), &accepted.note)
            } else {
                accepted
            })
        }
    }
}
