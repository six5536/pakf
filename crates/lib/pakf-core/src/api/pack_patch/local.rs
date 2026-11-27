//! This module finds the directory of a pack whose source is a local
//! directory, which `pakf patch --apply` writes and the walk of `-r` leaves
//! out.

use std::path::{Path, PathBuf};

use crate::{Error, RepoRoot, Result, pack};

/// `apply_target` returns the pack's directory in the working tree of the
/// source of the pack `name`, whose record is `entry`, and the directory as
/// the source names it, or refuses a source that isn't a local directory.
pub(super) fn apply_target(
    root: &RepoRoot,
    name: &str,
    entry: &pack::LockedPack,
) -> Result<(PathBuf, String)> {
    let Some((url, dir)) = local_dir(root, &entry.source, entry.path.as_deref()) else {
        return Err(Error::Pack(format!(
            "--apply: the pack {name} comes from {}, which isn't a local directory; `-o <FILE>` writes the patch for the pack's repository",
            entry.source
        )));
    };
    let mut label = url.trim_end_matches('/').to_string();
    for segment in pack::segments(entry.path.as_deref().unwrap_or("")) {
        label.push('/');
        label.push_str(segment);
    }
    Ok((dir?, label))
}

/// `local_dir` returns the source `source` of a pack, and the pack's
/// directory `path` in it, where the source is a local directory.
pub(super) fn local_dir(
    root: &RepoRoot,
    source: &str,
    path: Option<&str>,
) -> Option<(String, Result<PathBuf>)> {
    let pack::Source::Git(url) = pack::Source::parse(source);
    if !pack::is_local(&url) {
        return None;
    }
    // The source joins the root without `./`, so that a message names the
    // directory as the disk does.
    let source: PathBuf = Path::new(&pack::from_root(&url, root.as_path()))
        .components()
        .collect();
    let dir = pack::clone_dir(&source, path.unwrap_or(""));
    Some((url, dir))
}
