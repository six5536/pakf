//! This module holds the tests of the planner that read the meaning of a
//! pack's file: whether the copy is edited, whether the pack's new version
//! changes the base, and whether two texts mean the same, each through the
//! command's `Meaning` (the decision "The four cases of a sync" of
//! design-pack).

use std::fs;

use super::planner::Planner;
use crate::{
    RepoPath,
    pack::{
        eol::{is_base, same_text},
        lock::{LockedFile, base_path, hash_of},
    },
};

impl Planner<'_> {
    /// `holds_old` returns whether the copy `cur` holds its old base, whose
    /// hash is `base` and whose base copy is that of the file `rel` of the
    /// pack `pack`: its bytes hash to the base, as `is_base` reads them, or
    /// they equal the base copy's bytes once each CRLF of both is read as LF,
    /// so that an LF copy of a base that the pack committed with CRLF, and a
    /// base that a lock of the version 2 recorded, hold it too. It parses no
    /// text and starts no program.
    // sokf-implements: CLI-2_AC-85
    pub(super) fn holds_old(&self, base: &str, pack: &str, rel: &str, cur: &[u8]) -> bool {
        is_base(cur, base)
            || fs::read(base_path(pack, rel).to_fs(self.root)).is_ok_and(|b| same_text(cur, &b))
    }

    /// `changes` returns whether the pack's new version `upstream` of the
    /// file at `dest` changes its base, whose hash is `base` and whose base
    /// copy is that of the file `rel` of the pack `pack`: where the version
    /// neither hashes to the base nor means the same as the base copy, as
    /// `Meaning::same` reads it, so that a pack that only re-lays a file
    /// changes nothing.
    pub(super) fn changes(
        &self,
        base: &str,
        pack: &str,
        rel: &str,
        dest: &RepoPath,
        upstream: &[u8],
    ) -> bool {
        hash_of(upstream) != base
            && !self
                .meaning
                .same_as_file(dest, upstream, &base_path(pack, rel).to_fs(self.root))
    }

    /// `means` returns whether the text `a` of the file at `dest` means the
    /// same as `b`: the same bytes with each CRLF read as LF, or the same
    /// meaning, as `Meaning::same` reads it.
    pub(super) fn means(&self, dest: &RepoPath, a: &[u8], b: &[u8]) -> bool {
        same_text(a, b) || self.meaning.same(dest, a, b)
    }

    /// `edited` returns whether the copy `cur` of the file at `dest` differs
    /// from its base, whose hash is `base` and whose base copy is that of
    /// the file `rel` of the pack `pack`, in its text, as the test of an
    /// edit reads it, apart from its executable bit, which the plan of the
    /// bit decides on its own.
    pub(super) fn edited(
        &self,
        base: &str,
        pack: &str,
        rel: &str,
        dest: &RepoPath,
        cur: &[u8],
    ) -> bool {
        !is_base(cur, base)
            && !self
                .meaning
                .unedited(dest, cur, &base_path(pack, rel).to_fs(self.root))
    }

    /// `news` returns, for the pack's file `rel` at `dest`, whose new version
    /// is `upstream`, which the lock records as `recorded` and which holds
    /// `after` after the sync, whether `after` means the pack's new version,
    /// and whether the new version changes the file's base, as
    /// `follow_acceptance` reads them.
    pub(super) fn news(
        &self,
        dest: &RepoPath,
        rel: &str,
        upstream: &[u8],
        recorded: Option<&LockedFile>,
        after: Option<&[u8]>,
    ) -> (bool, bool) {
        (
            after.is_some_and(|b| self.means(dest, b, upstream)),
            recorded.is_some_and(|f| self.changes(&f.base, self.name, rel, dest, upstream)),
        )
    }
}
