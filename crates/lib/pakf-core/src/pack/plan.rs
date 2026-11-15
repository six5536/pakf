//! This module plans what a sync of one pack writes: it decides the action
//! for each file of the pack and refuses a sync that would break a rule of
//! design-pack, before any file is written.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    landing,
    lock::{Lock, LockedFile, LockedPack},
    manifest::{requires_newer, tool_version},
    owner::{Ownership, Supply},
    remap::Remap,
    source::{PackFiles, Source, destination},
};
use crate::{Error, RepoPath, RepoRoot, Result};

mod moved;
mod planner;

use planner::Planner;
pub use planner::dropping;

/// `pack_error` returns the error `m` of the sync of the pack `name`, which
/// names the pack first.
fn pack_error(name: &str, m: impl std::fmt::Display) -> Error {
    Error::Pack(format!("{name}: {m}"))
}

/// A `Fetched` is a pack read from its source, with what the lock records
/// of the source.
#[derive(Debug, Clone)]
pub struct Fetched {
    /// The pack's manifest and files.
    pub pack: PackFiles,
    /// The source.
    pub source: Source,
    /// The ref of a git source.
    pub git_ref: Option<String>,
    /// The pack's directory in a git source.
    pub path: Option<String>,
    /// The resolved commit.
    pub commit: String,
    /// The remaps of the paths of the pack's files, from the `paths` of its
    /// entry, or, for a pack read from the lock, from the lock's record.
    pub remap: Remap,
    /// Whether the pack was read from its base copies and its lock entry,
    /// without its source, because this sync doesn't fetch it.
    pub offline: bool,
}

impl Fetched {
    /// `version` returns the version that the lock records for the pack:
    /// the version of its manifest, or, for a pack read from the lock, the
    /// version that the lock records.
    pub fn version(&self) -> String {
        self.pack.manifest.version.clone()
    }
}

/// A `FileAction` is what a command of the packs does to one file of a pack,
/// as the word of the file's line of the text.
// `FileAction` is a wire type: `pakf sync --json` and `pakf remove
// --json` print it, so a change of a variant changes the published schemas.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum FileAction {
    /// The sync wrote a file that didn't exist.
    Added,
    /// The sync replaced an unedited file with the pack's new version.
    Updated,
    /// The sync merged the pack's change into an edited file.
    Merged,
    /// The merge conflicts, and the file holds git's conflict markers.
    Conflicted,
    /// The sync kept an existing file as an edit of the pack's version.
    KeptExisting,
    /// The command deleted an unedited file that the pack no longer holds,
    /// or a file of a pack that it removed.
    Removed,
    /// The command kept an edited file that the pack no longer holds, a file
    /// of a pack that it removed, or an entry that isn't a readable regular
    /// file, which no change of the pack reaches.
    Kept,
    /// The pack changed its version of a file that a higher pack owns, so
    /// the file holds the higher pack's version.
    Overridden,
    /// The user deleted a file that the lock records, and the sync keeps it
    /// deleted, because a deletion is an edit of the project.
    Deleted,
}

impl FileAction {
    /// `label` returns the word that the report prints.
    pub fn label(self) -> &'static str {
        match self {
            FileAction::Added => "added",
            FileAction::Updated => "updated",
            FileAction::Merged => "merged",
            FileAction::Conflicted => "conflicted",
            FileAction::KeptExisting => "kept-existing",
            FileAction::Removed => "removed",
            FileAction::Kept => "kept",
            FileAction::Overridden => "overridden",
            FileAction::Deleted => "deleted",
        }
    }
}

/// A `PackPlan` is everything that a sync of one pack writes.
#[derive(Debug, Clone)]
pub struct PackPlan {
    /// The pack's name.
    pub name: String,
    /// The pack's new lock entry.
    pub entry: LockedPack,
    /// The action on each file that changes, by destination.
    pub actions: Vec<(RepoPath, FileAction)>,
    /// The files to write, with their content.
    pub writes: Vec<(RepoPath, Vec<u8>)>,
    /// The files to delete.
    pub deletes: Vec<RepoPath>,
    /// The executable bit to give each file of `files/` that the sync
    /// writes, or whose bit the pack changes.
    pub modes: Vec<(RepoPath, bool)>,
    /// A note for each file whose shadowed version the pack changed, which
    /// names the pack that owns the file.
    pub notes: Vec<String>,
    /// The reason that the line of a file gives after its path, by
    /// destination, such as that the project and the pack both changed a
    /// binary file that the sync kept.
    pub reasons: BTreeMap<RepoPath, String>,
}

/// A `Request` holds the options of a sync that apply to one pack.
#[derive(Debug, Clone, Default)]
pub struct Request {
    /// Whether `--keep-existing` is given.
    pub keep_existing: bool,
    /// The files that the same sync deletes before it writes, because it
    /// removes a pack that no entry declares, or because a declared pack no
    /// longer holds them. The planner reads each as absent.
    pub vacated: BTreeSet<RepoPath>,
    /// The destinations that a declared pack of the sync owns. A file that
    /// the pack no longer holds stays when its destination is among them,
    /// because it passes to the pack that owns it.
    pub owned_dests: BTreeSet<RepoPath>,
}

/// `stays_for_owner` says whether the file `dest` that `f` records, which
/// its pack no longer holds or which goes with its pack, stays for another
/// pack: a file that another pack shadowed holds the owner's version, and a
/// file whose destination is one of `owned_dests` passes to the declared
/// pack that owns it.
pub fn stays_for_owner(f: &LockedFile, dest: &RepoPath, owned_dests: &BTreeSet<RepoPath>) -> bool {
    f.shadowed_by.is_some() || owned_dests.contains(dest)
}

/// `supply` returns each destination of the pack `fetched`.
pub fn supply(fetched: &Fetched) -> Supply {
    let name = &fetched.pack.manifest.name;
    let files = fetched
        .pack
        .files
        .keys()
        .filter_map(|rel| RepoPath::parse(&destination(name, &fetched.remap, rel)?))
        .collect();
    Supply { files }
}

/// `plan` plans the sync of `fetched` into the repository. `own` says which
/// of its destinations another pack owns, and which it takes over from
/// another pack. It refuses the sync, with no file written, when the pack
/// needs a newer pakf, when a file lands on an existing file that the lock
/// doesn't record, that differs from the pack's version, or when a new file
/// lands below an entry that isn't a directory, and `--keep-existing` is
/// absent, or when an edited file needs a merge and its base copy is
/// missing. A pack whose source or path differs from the lock's record
/// moves to it, and keeps its bases. It also refuses a pack with a file
/// that isn't text, a file of `files/` that lands where no pack writes, and
/// two files that land at one destination, as the landing module says.
// sokf-implements: CLI-2_AC-12
// sokf-implements: CLI-2_AC-14
// sokf-implements: CLI-2_AC-15
// sokf-implements: CLI-2_AC-19
pub fn plan(
    root: &RepoRoot,
    lock: &Lock,
    fetched: &Fetched,
    request: &Request,
    own: &Ownership,
) -> Result<PackPlan> {
    let manifest = &fetched.pack.manifest;
    let name = &manifest.name;
    admit(fetched)?;
    let out = PackPlan {
        name: name.clone(),
        entry: LockedPack {
            source: fetched.source.label(),
            git_ref: fetched.git_ref.clone(),
            path: fetched.path.clone(),
            paths: fetched.remap.written().clone(),
            commit: fetched.commit.clone(),
            version: fetched.version(),
            depends: manifest.depends.clone(),
            files: BTreeMap::new(),
        },
        actions: Vec::new(),
        writes: Vec::new(),
        deletes: Vec::new(),
        modes: Vec::new(),
        notes: Vec::new(),
        reasons: BTreeMap::new(),
    };
    let mut planner = Planner {
        root,
        fetched,
        request,
        own,
        name,
        existing: lock.packs.get(name),
        out,
        unrecorded: Vec::new(),
        conflicted: Vec::new(),
        below: Vec::new(),
        binary_kept: BTreeSet::new(),
        carried: BTreeSet::new(),
    };
    for (rel, upstream) in &fetched.pack.files {
        let Some(dest) = destination(name, &fetched.remap, rel) else {
            continue;
        };
        let dest = RepoPath::parse(&dest)
            .ok_or_else(|| pack_error(name, format_args!("{rel}: not a path")))?;
        let file = planner.target(rel, upstream, dest);
        if let Some(shadow) = own.shadowed.get(&file.dest) {
            planner.shadowed(&file, shadow);
        } else {
            planner.owned(file)?;
        }
    }
    if !planner.conflicted.is_empty() {
        return Err(pack_error(
            name,
            format_args!(
                "a file of the pack holds a conflict that an earlier sync left: {}; resolve it, then sync again",
                planner.conflicted.join(", ")
            ),
        ));
    }
    let mut refusals = Vec::new();
    if !planner.unrecorded.is_empty() {
        refusals.push(format!(
            "the pack's files land on files that the lock doesn't record: {}; delete a file to take the pack's version, or pass --keep-existing to keep each as an edit of the pack's version",
            planner.unrecorded.join(", ")
        ));
    }
    // sokf-implements: CLI-2_AC-63
    if !planner.below.is_empty() {
        refusals.push(format!(
            "the pack's new files land below entries that aren't directories: {}; move an entry away to take the pack's version, or pass --keep-existing to keep each entry, which leaves the pack's file deleted",
            planner.below.join(", ")
        ));
    }
    if !refusals.is_empty() {
        return Err(pack_error(name, refusals.join("; ")));
    }
    planner.remove_dropped();
    let mut out = planner.out;
    out.actions.sort();
    Ok(out)
}

/// `admit` refuses the pack `fetched` when it needs a newer pakf, when a
/// file of it lands where no pack writes, and when two of its files land at
/// one destination.
fn admit(fetched: &Fetched) -> Result<()> {
    let manifest = &fetched.pack.manifest;
    let name = &manifest.name;
    if requires_newer(&manifest.requires_pakf, tool_version()) {
        return Err(pack_error(
            name,
            format_args!(
                "the pack requires pakf {}, and this is pakf {}; upgrade pakf",
                manifest.requires_pakf,
                tool_version()
            ),
        ));
    }
    let refused = landing::check(&fetched.pack, &fetched.remap);
    if !refused.is_empty() {
        return Err(pack_error(name, refused.join("; ")));
    }
    let collided = landing::collisions(&fetched.pack, &fetched.remap);
    if !collided.is_empty() {
        return Err(pack_error(name, collided.join("; ")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::FileAction;

    // The JSON of a file's action is the word of its line of the text.
    // sokf-tests: CLI-2_AC-56
    #[test]
    fn the_json_of_each_action_is_its_label() {
        for action in [
            FileAction::Added,
            FileAction::Updated,
            FileAction::Merged,
            FileAction::Conflicted,
            FileAction::KeptExisting,
            FileAction::Removed,
            FileAction::Kept,
            FileAction::Overridden,
            FileAction::Deleted,
        ] {
            assert_eq!(
                serde_json::to_value(action).unwrap(),
                serde_json::json!(action.label())
            );
        }
    }
}
