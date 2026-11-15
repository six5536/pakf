//! This module holds the wire types of the JSON of `pakf status`,
//! `sync`, `add`, `remove`, and `accept`, and turns each
//! report of the core library into one. The `output_schema` module generates
//! the published schema of each report from its type, so a change of a field
//! changes the schema. A fact that a report shares with `pakf ls` takes the
//! key and the form of `pakf ls`, and every object holds every key, with
//! `null` where it has no value (CLI-1_AC-28).

// sokf-implements: CLI-1_AC-28
use schemars::JsonSchema;
use serde::Serialize;

use super::{
    pack::{PackSyncReport, SyncedPack, fixed_notes},
    pack_accept::PackAccepted,
    pack_info::{PackAcceptance, PackFileState, PackRemoved, PackStatus},
    pack_ls::every_key_required,
};
use crate::pack::FileAction;

/// One pack in the lock, with each of its files that differs from its base
/// or has an acceptance, and each of its files that a higher pack owns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(rename = "Pack", transform = every_key_required)]
pub struct StatusPack {
    /// The pack's id, `@scope/name`.
    pub name: String,
    /// The pack's version, as the lock records it.
    pub version: String,
    /// The pack's source, as the lock records it: a git URL or path.
    pub source: String,
    /// The branch, tag, or commit of the git source.
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,
    /// Each file that the pack owns and that differs from its base or has an
    /// acceptance, with the files whose acceptance is `accepted` after the
    /// others.
    pub files: Vec<StatusFile>,
    /// Each file of the pack that a higher pack owns.
    pub shadowed: Vec<StatusShadowed>,
}

/// A file of a pack with its state and its acceptance, as `pakf ls --json`
/// gives them (CLI-4_AC-11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(rename = "File", transform = every_key_required)]
pub struct StatusFile {
    /// The file's path from the repository root, such as `/LICENSE`.
    pub path: String,
    /// The state of the file, whatever its acceptance.
    pub state: PackFileState,
    /// The acceptance of the file, `accepted` while it holds its accepted
    /// version and `stale` while the lock records an acceptance that it
    /// doesn't hold, and `null` without an acceptance.
    pub acceptance: Option<PackAcceptance>,
    /// The note of the acceptance that the lock records for the file,
    /// `accepted` or `stale`, and `null` otherwise.
    pub note: Option<String>,
}

/// A file of a pack that a higher pack owns, so that the file holds the
/// higher pack's version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(rename = "Shadowed")]
pub struct StatusShadowed {
    /// The file's path from the repository root.
    pub path: String,
    /// The pack that owns the file.
    pub owner: String,
    /// Whether the last sync changed this pack's version of the file.
    pub changed: bool,
}

/// The report of `pakf status --json`: one entry for each pack in the
/// lock, in the order of the packs, from the lowest to the highest, as the
/// text lists them (CLI-4_AC-10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(transparent)]
#[schemars(title = "pakf status report")]
pub struct PackStatusReport(pub Vec<StatusPack>);

/// A file of a pack and what the command did to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(rename = "File")]
pub struct ActedFile {
    /// The file's path from the repository root.
    pub path: String,
    /// What the command did to the file, the word of its line of the text
    /// (CLI-2_AC-56).
    pub action: FileAction,
}

/// A pack that the command wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(rename = "Pack", transform = every_key_required)]
pub struct WrittenPack {
    /// The pack's id, `@scope/name`.
    pub name: String,
    /// The pack's version.
    pub version: String,
    /// The pack's source, as the lock records it.
    pub source: String,
    /// Whether the command fetched the pack, and `false` when it read the
    /// pack from its base copies because only the owners of its files
    /// changed.
    pub fetched: bool,
    /// Each file that the command changed or kept.
    pub files: Vec<ActedFile>,
}

/// A pack that the command removed, with what it did to each of its files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(rename = "RemovedPack")]
pub struct RemovedPack {
    /// The pack's id, `@scope/name`.
    pub name: String,
    /// Each file of the pack that the command deleted, `removed`, or kept
    /// because it is edited, `kept`.
    pub files: Vec<ActedFile>,
}

/// The report of `pakf sync --json` and `pakf add --json`: what the
/// command declared, removed, and wrote (CLI-2_AC-55).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(title = "pakf sync report", transform = every_key_required)]
pub struct PackSyncJson {
    /// The pack that `pakf add` declared in `/.pakf/config.toml`, and `null`
    /// for `pakf sync`.
    pub declared: Option<String>,
    /// Each pack of the lock that no `[[pack]]` entry declares, which the
    /// command removed.
    pub undeclared: Vec<RemovedPack>,
    /// Each pack that the command wrote, in the order of the text.
    pub packs: Vec<WrittenPack>,
    /// The report of agent-harness-adapter on pakf's block, as it stands,
    /// and an empty string when the command handed the adapter nothing.
    pub instructions: String,
    /// The notes of the text for a person, in the order of the text,
    /// without the `note: ` before each, such as an acceptance that the sync
    /// dropped, and a block that the command kept as it stands.
    pub notes: Vec<String>,
}

/// The report of `pakf remove --json`: what the command removed, and
/// what the packs that took over its files did (CLI-7_AC-7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(title = "pakf remove report")]
pub struct PackRemoveJson {
    /// The pack's id, `@scope/name`.
    pub name: String,
    /// Whether the command deleted the pack's `[[pack]]` entry from
    /// `/.pakf/config.toml`.
    pub entry: bool,
    /// Whether the lock held the pack, so that the command dropped it from
    /// the lock.
    pub locked: bool,
    /// Each file of the pack that the command deleted, `removed`, or kept
    /// because it is edited or because `--keep files` keeps it, `kept`.
    pub files: Vec<ActedFile>,
    /// Each pack that took over a file of the removed pack, with what the
    /// command did to each file.
    pub packs: Vec<WrittenPack>,
    /// The report of agent-harness-adapter on pakf's block, as it stands,
    /// and an empty string when the command handed the adapter nothing.
    pub instructions: String,
    /// The notes of the text for a person, in the order of the text,
    /// without the `note: ` before each, such as the note that names the
    /// base copy of a deleted file, and a block that the command kept as it
    /// stands.
    pub notes: Vec<String>,
}

/// A file whose acceptance the command wrote or withdrew, with the
/// acceptance that it has after the command, as `pakf ls --json` gives it
/// (CLI-6_AC-8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(rename = "File", transform = every_key_required)]
pub struct AcceptedFile {
    /// The file's path from the repository root.
    pub path: String,
    /// `accepted` for a file whose edit or deletion the command accepted,
    /// and `null` for a file whose acceptance it withdrew.
    pub acceptance: Option<PackAcceptance>,
    /// The note that the command recorded with the acceptance, and `null`
    /// for a file whose acceptance it withdrew.
    pub note: Option<String>,
}

/// The report of `pakf accept --json`: each file whose acceptance the
/// command wrote or withdrew (CLI-6_AC-7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(title = "pakf accept report")]
pub struct PackAcceptJson {
    /// Each file, in the order of the text.
    pub files: Vec<AcceptedFile>,
}

impl PackStatus {
    /// `to_json` returns the report as the array of `--json`.
    // sokf-implements: CLI-4_AC-10
    // sokf-implements: CLI-4_AC-11
    pub fn to_json(&self) -> PackStatusReport {
        PackStatusReport(
            self.packs
                .iter()
                .map(|p| StatusPack {
                    name: p.name.clone(),
                    version: p.version.clone(),
                    source: p.source.clone(),
                    git_ref: p.git_ref.clone(),
                    files: p
                        .files
                        .iter()
                        .map(|f| StatusFile {
                            path: f.path.clone(),
                            state: f.state,
                            acceptance: f.acceptance,
                            note: f.note.clone(),
                        })
                        .collect(),
                    shadowed: p
                        .shadowed
                        .iter()
                        .map(|f| StatusShadowed {
                            path: f.path.clone(),
                            owner: f.owner.clone(),
                            changed: f.changed,
                        })
                        .collect(),
                })
                .collect(),
        )
    }
}

impl PackSyncReport {
    /// `to_json` returns the report as the object of `--json`.
    // sokf-implements: CLI-2_AC-55
    // sokf-implements: CLI-2_AC-56
    // sokf-implements: INST-1_AC-17
    pub fn to_json(&self) -> PackSyncJson {
        PackSyncJson {
            declared: self.declared.clone(),
            undeclared: self
                .undeclared
                .iter()
                .map(|u| RemovedPack {
                    name: u.name.clone(),
                    files: removal(&u.removed, &u.kept),
                })
                .collect(),
            packs: self.packs.iter().map(written).collect(),
            instructions: self.instructions.report.clone(),
            notes: pack_notes(&self.packs)
                .chain(self.instructions.held.clone())
                .chain(fixed_notes(&self.packs).into_iter().map(str::to_string))
                .collect(),
        }
    }
}

impl PackRemoved {
    /// `to_json` returns the report as the object of `--json`.
    // sokf-implements: CLI-7_AC-7
    // sokf-implements: CLI-2_AC-56
    // sokf-implements: INST-1_AC-17
    pub fn to_json(&self) -> PackRemoveJson {
        PackRemoveJson {
            name: self.name.clone(),
            entry: self.undeclared,
            locked: self.locked,
            files: removal(
                &self.removed,
                &[self.kept.as_slice(), self.own.as_slice()].concat(),
            ),
            packs: self.handed.iter().map(written).collect(),
            instructions: self.instructions.report.clone(),
            notes: pack_notes(&self.handed)
                .chain(self.instructions.held.clone())
                .chain(fixed_notes(&self.handed).into_iter().map(str::to_string))
                .collect(),
        }
    }
}

impl PackAccepted {
    /// `to_json` returns the report as the object of `--json`: each file
    /// that the command accepted, with its note, then each file whose
    /// acceptance it withdrew.
    // sokf-implements: CLI-6_AC-7
    // sokf-implements: CLI-6_AC-8
    pub fn to_json(&self) -> PackAcceptJson {
        let accepted = self.accepted.iter().map(|path| AcceptedFile {
            path: path.clone(),
            acceptance: Some(PackAcceptance::Accepted),
            note: self.note.clone(),
        });
        let withdrawn = self.withdrawn.iter().map(|path| AcceptedFile {
            path: path.clone(),
            acceptance: None,
            note: None,
        });
        PackAcceptJson {
            files: accepted.chain(withdrawn).collect(),
        }
    }
}

/// `written` returns the pack that a sync wrote, in the form of the JSON.
fn written(p: &SyncedPack) -> WrittenPack {
    WrittenPack {
        name: p.name.clone(),
        version: p.version.clone(),
        source: p.source.clone(),
        fetched: p.fetched,
        files: p
            .files
            .iter()
            .map(|(path, action)| ActedFile {
                path: path.clone(),
                action: *action,
            })
            .collect(),
    }
}

/// `removal` returns the files that a removal deleted and those that it
/// kept, in the order of the text, each with its action.
fn removal(removed: &[String], kept: &[String]) -> Vec<ActedFile> {
    let acted = |paths: &[String], action: FileAction| {
        paths
            .iter()
            .map(move |path| ActedFile {
                path: path.clone(),
                action,
            })
            .collect::<Vec<_>>()
    };
    let mut files = acted(removed, FileAction::Removed);
    files.extend(acted(kept, FileAction::Kept));
    files
}

/// `pack_notes` returns the notes of the packs `packs`, in the order of the
/// text, without the `note: ` that the text puts before each.
fn pack_notes(packs: &[SyncedPack]) -> impl Iterator<Item = String> + '_ {
    packs.iter().flat_map(|p| &p.notes).map(|note| {
        note.strip_prefix("note: ")
            .unwrap_or(note.as_str())
            .to_string()
    })
}
