//! This module defines one entry point for each command, and re-exports
//! those entry points with the reports that the commands share.

mod pack;
mod pack_accept;
mod pack_diff;
mod pack_info;
mod pack_keep;
mod pack_ls;
mod pack_patch;
mod pack_report;
mod pack_run;

pub use pack::{
    PackAddOptions, PackSyncOptions, PackSyncReport, SyncedPack, UndeclaredPack, pack_add,
    pack_sync,
};
pub use pack_accept::{PackAcceptOptions, PackAccepted, pack_accept};
pub use pack_diff::{PackDiff, PackDiffOptions, pack_diff_with_notes};
pub use pack_info::{
    PackAcceptance, PackFile, PackFileState, PackRemoved, PackState, PackStatus, pack_remove,
    pack_remove_keeping, pack_status,
};
pub use pack_keep::PackKeep;
pub use pack_ls::{
    PackFileKind, PackLs, PackLsAcceptance, PackLsCounts, PackLsEntry, PackLsListing,
    PackLsOptions, PackLsReport, PackLsState, pack_ls,
};
pub use pack_patch::{PackPatch, PackPatchOptions, pack_patch};
pub use pack_report::{
    AcceptedFile, ActedFile, PackAcceptJson, PackRemoveJson, PackStatusReport, PackSyncJson,
    RemovedPack, StatusFile, StatusPack, StatusShadowed, WrittenPack,
};
