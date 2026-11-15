//! The pack module copies a pack into a project and keeps it current: it reads the pack's manifest, records the pack in the lock,
//! fetches it through the user's `git`, and merges each later upstream
//! change with the project's edits through `git merge-file` (design-pack).

mod dest;
mod eol;
mod git;
mod ignore;
mod landing;
mod lock;
mod manifest;
mod mode;
mod order;
mod owner;
mod plan;
mod remap;
mod source;

pub use dest::{Dest, empties, entry_above, is_absent, read_dest};
pub(crate) use eol::{Reading, committed, has_crlf};
pub use eol::{is_base, is_binary, normalise};
pub use git::{TempDir, apply, diff, diff_modes, merge_file, shallow_clone};
pub(crate) use ignore::Ignores;
pub use landing::{below, is_text, own_ignores, refusal};
pub(crate) use lock::ordered;
pub use lock::{
    Accepted, LOCK_VERSION, Lock, LockedFile, LockedPack, PACKS_DIR, base_path, hash_of,
    instructions_of, lock_path, lock_text, pack_dir, read_lock,
};
pub use manifest::{
    MANIFEST, PackManifest, ids_named, is_pack_id, is_slug, parse_manifest, requires_newer,
    split_id, tool_version,
};
pub use mode::{carried_bit, carries_mode, file_is_executable, set_executable};
pub use order::precedence;
pub use owner::{Ownership, Shadow, Supply, Takeover, owners};
pub use plan::{Fetched, FileAction, PackPlan, Request, dropping, plan, stays_for_owner, supply};
pub use remap::Remap;
pub use source::{
    FILES, PackFiles, Source, clone_dir, destination, from_root, in_files, is_local, origin,
    read_dir, recorded, segments,
};
