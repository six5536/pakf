//! The `config` module finds the repository root, reads the `[[pack]]`
//! entries of the configuration.

mod file;
mod packs;
mod root;

pub use file::config_path;
pub use packs::{PackEntry, add_entry, declared_order, read_packs, remove_entry};
pub use root::{PathArg, discover_root, holds_git};
