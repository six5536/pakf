//! This module gives the path of the configuration.

use crate::RepoPath;

/// `config_path` returns the path of the configuration file, which holds the
/// `[[pack]]` entries.
pub fn config_path() -> RepoPath {
    RepoPath::parse("/.pakf/config.toml").expect("literal")
}
