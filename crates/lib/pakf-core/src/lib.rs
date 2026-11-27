//! pakf shares project files through packs. A pack is a set of files, kept in
//! a git repository, that pakf copies into a project and keeps in step: it
//! records the base of each file in a lock and merges each later change of
//! the pack into the project's edits.
//!
//! The binary (`crates/app/pakf`) parses the arguments and renders the
//! output. The logic that the binary drives lives here, so that tests can
//! run that logic without starting a process.
//!
//! **This crate is unstable.** It is at a 0.x version, and its interface can
//! change in any release.

#![warn(missing_docs)]

mod api;
mod config;
mod conflicts;
mod error;
mod git;
mod harness;
mod output_schema;
pub mod pack;
mod path;
mod query;
mod source;
#[cfg(test)]
mod test_support;
pub mod yaml;

pub use api::*;
pub use config::{PathArg, discover_root};
pub use error::{Error, Result};
pub use harness::Instructions;
pub use output_schema::{Output, output_schemas};
pub use path::{RepoPath, RepoRoot};
pub use query::Expr;
pub use source::LineEnding;

/// `VERSION` is the version of pakf, as its manifest gives it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::VERSION;

    #[test]
    fn the_version_is_the_manifests() {
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
    }
}
