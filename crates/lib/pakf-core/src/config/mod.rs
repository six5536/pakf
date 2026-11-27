//! The `config` module finds the repository root, reads the `[[pack]]`
//! entries of the configuration, and its table `[compare]`.

mod compare;
mod file;
mod packs;
mod root;

pub use compare::{Compare, LayOut, TIMEOUT, read_compare};
pub use file::config_path;
pub use packs::{PackEntry, add_entry, declared_order, read_packs, remove_entry};
pub use root::{PathArg, discover_root, holds_git};
