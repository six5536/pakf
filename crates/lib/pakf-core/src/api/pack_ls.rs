//! This module implements `pakf ls`: it lists the entries of
//! the tree as `ls` does, with the pack, the kind, the state, and the
//! acceptance of each file that a pack delivered in place of the file's
//! attributes. It reads the lock, the working tree, and git's list of the
//! files that it ignores, and writes nothing. The `text` module renders its
//! text.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

use schemars::JsonSchema;
use serde::Serialize;

use super::pack_info::{PackAcceptance, PackFileState, file_state};
use crate::{Error, RepoPath, RepoRoot, Result, config::PathArg, pack};

mod text;

/// A `PackFileKind` is what a file of a pack is, from its place in the pack
/// and the declaration of the pack that owns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PackFileKind {
    /// A file of the pack's `files/`, at its path from the repository root
    /// after the remaps of the pack's entry.
    File,
    /// A file of the pack's `files/` that the pack that owns it declares
    /// editable, whose edit needs no acceptance.
    Editable,
    /// The copy of the pack's `instructions.md`.
    Instructions,
}

impl PackFileKind {
    /// `of` returns the kind of the file whose path in the pack is `rel` and
    /// whose record is `f`.
    // sokf-implements: CLI-5_AC-37
    pub(super) fn of(rel: &str, f: &pack::LockedFile) -> PackFileKind {
        if rel == "instructions.md" {
            PackFileKind::Instructions
        } else if f.editable {
            PackFileKind::Editable
        } else {
            PackFileKind::File
        }
    }

    /// `label` returns the word that the text prints.
    pub fn label(self) -> &'static str {
        match self {
            PackFileKind::File => "file",
            PackFileKind::Editable => pack::EDITABLE,
            PackFileKind::Instructions => "instructions",
        }
    }
}

/// A `PackLsState` is the state of a file of a pack in `pakf ls`, whatever
/// its acceptance: one of the states of `pakf status`. The variants run from
/// the least serious to the most, so that a directory takes the greatest
/// state below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PackLsState {
    /// The file holds its base.
    Unchanged,
    /// The user edited the file.
    Edited,
    /// The user deleted the file.
    Deleted,
    /// The file holds a conflict that a merge left.
    Conflicted,
}

impl PackLsState {
    /// `ALL` lists the states from the most serious to the least, in the
    /// order in which the text counts them.
    pub const ALL: [PackLsState; 4] = [
        PackLsState::Conflicted,
        PackLsState::Deleted,
        PackLsState::Edited,
        PackLsState::Unchanged,
    ];

    /// `label` returns the word that the text prints.
    pub fn label(self) -> &'static str {
        match self {
            PackLsState::Unchanged => "unchanged",
            PackLsState::Edited => "edited",
            PackLsState::Deleted => "deleted",
            PackLsState::Conflicted => "conflicted",
        }
    }

    /// `differs` returns whether a file in the state differs from its base.
    fn differs(self) -> bool {
        self > PackLsState::Unchanged
    }
}

impl From<PackFileState> for PackLsState {
    fn from(state: PackFileState) -> Self {
        match state {
            PackFileState::Unchanged => PackLsState::Unchanged,
            PackFileState::Edited => PackLsState::Edited,
            PackFileState::Conflicted => PackLsState::Conflicted,
            PackFileState::Deleted => PackLsState::Deleted,
        }
    }
}

/// A `PackLsCounts` holds the number of the files of packs below a
/// directory, at any depth, the number in each state, and, apart, the
/// number with each acceptance, so that the states add up to the files.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct PackLsCounts {
    /// The number of the files of packs below the directory.
    pub files: usize,
    /// The number of those files that are unchanged.
    pub unchanged: usize,
    /// The number of those files that are edited.
    pub edited: usize,
    /// The number of those files that are conflicted.
    pub conflicted: usize,
    /// The number of those files that are deleted.
    pub deleted: usize,
    /// The number of those files, in any state, whose acceptance is
    /// `accepted`.
    pub accepted: usize,
    /// The number of those files, in any state, whose acceptance is
    /// `stale`.
    pub stale: usize,
}

impl PackLsCounts {
    /// `add` counts one file in the state `state` with the acceptance
    /// `acceptance`.
    // sokf-implements: CLI-5_AC-32
    fn add(&mut self, state: PackLsState, acceptance: Option<PackAcceptance>) {
        self.files += 1;
        *self.of_mut(state) += 1;
        if let Some(a) = acceptance {
            *self.with_mut(a) += 1;
        }
    }

    /// `of` returns the number of the files in the state `state`.
    pub fn of(&self, state: PackLsState) -> usize {
        match state {
            PackLsState::Unchanged => self.unchanged,
            PackLsState::Edited => self.edited,
            PackLsState::Deleted => self.deleted,
            PackLsState::Conflicted => self.conflicted,
        }
    }

    /// `with` returns the number of the files with the acceptance
    /// `acceptance`.
    pub fn with(&self, acceptance: PackAcceptance) -> usize {
        match acceptance {
            PackAcceptance::Accepted => self.accepted,
            PackAcceptance::Stale => self.stale,
        }
    }

    fn with_mut(&mut self, acceptance: PackAcceptance) -> &mut usize {
        match acceptance {
            PackAcceptance::Accepted => &mut self.accepted,
            PackAcceptance::Stale => &mut self.stale,
        }
    }

    fn of_mut(&mut self, state: PackLsState) -> &mut usize {
        match state {
            PackLsState::Unchanged => &mut self.unchanged,
            PackLsState::Edited => &mut self.edited,
            PackLsState::Deleted => &mut self.deleted,
            PackLsState::Conflicted => &mut self.conflicted,
        }
    }
}

/// An entry is one file or directory that `pakf ls` lists.
// `PackLsEntry` is a wire type: `pakf ls --json` prints it, and the
// `output_schema` module generates `ls-report.schema.json` from it, so
// a change of a field changes the published schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(rename = "Entry", transform = every_key_required)]
pub struct PackLsEntry {
    /// The entry's path from the repository root, such as `/LICENSE`.
    pub path: String,
    /// Whether the entry is a directory.
    pub directory: bool,
    /// The id of the pack that owns the file, and `null` for a directory and
    /// for a file that no pack delivered.
    pub pack: Option<String>,
    /// The kind of the file of a pack, from its place in the pack and the
    /// declaration of the pack that owns it, and `null` for a directory and
    /// for a file that no pack delivered.
    pub kind: Option<PackFileKind>,
    /// The state of the file of a pack, whatever its acceptance, or the
    /// state of the directory, from the files of packs below it, and `null`
    /// for a file that no pack delivered and for a directory that holds none.
    pub state: Option<PackLsState>,
    /// The acceptance of the file of a pack, `accepted` while it holds its
    /// accepted version and `stale` while the lock records an acceptance
    /// that it doesn't hold, or `accepted` for a directory whose files of
    /// packs that differ from their base all have that acceptance, and
    /// `null` otherwise.
    pub acceptance: Option<PackAcceptance>,
    /// The note of the acceptance that the lock records for the file,
    /// `accepted` or `stale`, and `null` otherwise.
    pub note: Option<String>,
    /// For a directory, the number of the files of packs below it, the
    /// number in each state, and the number with each acceptance, and
    /// `null` for a file.
    pub counts: Option<PackLsCounts>,
}

/// `every_key_required` marks each key of an entry as required in the
/// schema, because an entry holds every key, with `null` where it has no
/// value, so that the shape stays the same for every entry.
pub(super) fn every_key_required(schema: &mut schemars::Schema) {
    let keys: Vec<serde_json::Value> = schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .map(|p| p.keys().cloned().map(serde_json::Value::String).collect())
        .unwrap_or_default();
    schema.insert("required".into(), serde_json::Value::Array(keys));
}

/// The report of `pakf ls --json`: one entry for each entry that the
/// command lists, in the order of its text (CLI-5_AC-20).
// `PackLsReport` is a wire type, as `PackLsEntry` is.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(transparent)]
#[schemars(title = "pakf ls report")]
pub struct PackLsReport(pub Vec<PackLsEntry>);

/// `PackLsOptions` holds the options of `pakf ls`.
#[derive(Debug, Clone, Default)]
pub struct PackLsOptions {
    /// The files and directories to list, as the user wrote them. None
    /// lists the current directory.
    pub paths: Vec<String>,
    /// The current directory, from which a relative path is read.
    pub cwd: PathBuf,
    /// Whether to list every directory below each directory too.
    pub recursive: bool,
    /// Whether to list each dotfile and dot-directory too.
    pub all: bool,
    /// Whether to list each entry that git ignores too.
    pub ignored: bool,
    /// The pack whose files alone to list.
    pub pack: Option<String>,
    /// The state of the files alone to list.
    pub state: Option<PackLsState>,
    /// The acceptance of the files alone to list.
    pub acceptance: Option<PackLsAcceptance>,
}

/// A `PackLsAcceptance` is the acceptance that `--acceptance` keeps: one of
/// the acceptances, none for the files without one, or `unaccepted` for the
/// files whose acceptance is `stale` or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackLsAcceptance {
    /// The files whose acceptance is `accepted`.
    Accepted,
    /// The files whose acceptance is `stale`.
    Stale,
    /// The files without an acceptance.
    NoAcceptance,
    /// The files whose acceptance is `stale` or none.
    Unaccepted,
}

impl PackLsAcceptance {
    /// `keeps` returns whether a file with the acceptance `acceptance`
    /// passes the filter.
    // sokf-implements: CLI-5_AC-35
    fn keeps(self, acceptance: Option<PackAcceptance>) -> bool {
        match self {
            PackLsAcceptance::Accepted => acceptance == Some(PackAcceptance::Accepted),
            PackLsAcceptance::Stale => acceptance == Some(PackAcceptance::Stale),
            PackLsAcceptance::NoAcceptance => acceptance.is_none(),
            PackLsAcceptance::Unaccepted => {
                matches!(acceptance, None | Some(PackAcceptance::Stale))
            }
        }
    }
}

/// A `PackLsListing` is one group of entries: the paths given that aren't
/// directories, or the entries of one directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackLsListing {
    /// The directory whose entries the group holds, or none for the paths
    /// given that aren't directories.
    pub dir: Option<String>,
    /// The entries, in the order of the text.
    pub entries: Vec<PackLsEntry>,
}

/// A `PackLs` is the report of `pakf ls`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackLs {
    /// The groups, in the order of the text.
    pub listings: Vec<PackLsListing>,
    /// Whether the text names each directory above its entries, as `ls`
    /// does for more than one path and with `-R`.
    pub headers: bool,
    /// A warning for each formatter or command of `[compare]` that was
    /// missing, failed, or ran past its limit, which the command prints on
    /// stderr, outside its JSON.
    pub warnings: Vec<String>,
}

impl PackLs {
    /// `to_json` returns the report as the array of `--json`, one entry for
    /// each listed entry, in the order of the text.
    pub fn to_json(&self) -> PackLsReport {
        PackLsReport(
            self.listings
                .iter()
                .flat_map(|l| l.entries.iter().cloned())
                .collect(),
        )
    }
}

/// A `Record` is what `pakf ls` knows of one destination of the lock.
#[derive(Debug, Clone)]
struct Record {
    pack: String,
    kind: PackFileKind,
    state: PackLsState,
    acceptance: Option<PackAcceptance>,
    note: Option<String>,
}

/// `records` returns one record for each destination of the lock that a
/// pack owns: the record of that pack, with the state and the acceptance
/// that `pakf status` gives it.
// sokf-implements: CLI-5_AC-6
// sokf-implements: CLI-5_AC-7
// sokf-implements: CLI-5_AC-8
// sokf-implements: CLI-5_AC-25
// sokf-implements: CLI-5_AC-26
fn records(
    root: &RepoRoot,
    meaning: &pack::Meaning,
    lock: &pack::Lock,
) -> BTreeMap<String, Record> {
    let mut owned = BTreeMap::new();
    for name in pack::ordered(root, lock) {
        let Some(p) = lock.packs.get(&name) else {
            continue;
        };
        for (rel, f) in &p.files {
            let kind = PackFileKind::of(rel, f);
            // `file_state` gives no state to a shadowed record, whose file
            // the record of its owner describes.
            let Some((state, acceptance)) = file_state(root, meaning, &name, rel, f) else {
                continue;
            };
            // The note belongs to the acceptance, `accepted` or `stale`, as
            // in `pakf status`.
            owned.insert(
                f.path.clone(),
                Record {
                    pack: name.clone(),
                    kind,
                    state: state.into(),
                    acceptance,
                    note: f.accepted.as_ref().map(|a| a.note.clone()),
                },
            );
        }
    }
    owned
}

/// `ignored_paths` returns each path that git ignores below the
/// directories `dirs`, as `git::ignored` lists them. Without git, or when
/// git fails, inside a git repository or outside one, it returns no path,
/// so a caller that must tell a failure apart calls `git::ignored`. It
/// drops git's warning that it can't read an ignore file, and keeps each
/// path that git still lists.
// sokf-implements: CLI-5_AC-11
// sokf-implements: CLI-5_AC-12
pub(super) fn ignored_paths(root: &RepoRoot, dirs: &[String]) -> BTreeSet<String> {
    crate::git::ignored(root, dirs)
        .map(|ignored| ignored.paths)
        .unwrap_or_default()
}

/// `join` returns the path of the entry `name` of the directory `dir`.
fn join(dir: &str, name: &str) -> String {
    if dir == "/" {
        format!("/{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// `below` returns the prefix of every path below the directory `dir`.
fn below(dir: &str) -> String {
    if dir == "/" {
        "/".to_string()
    } else {
        format!("{dir}/")
    }
}

/// A `Lister` holds what one run of `pakf ls` reads once.
struct Lister<'a> {
    root: &'a RepoRoot,
    options: &'a PackLsOptions,
    records: BTreeMap<String, Record>,
    /// Each path that git ignores, or none with `--ignored`.
    ignored: Option<BTreeSet<String>>,
}

impl Lister<'_> {
    /// `under` returns the records of the destinations below the directory
    /// `dir`, at any depth.
    fn under<'s>(&'s self, dir: &str) -> impl Iterator<Item = (&'s String, &'s Record)> + 's {
        let prefix = below(dir);
        self.records
            .range(prefix.clone()..)
            .take_while(move |(p, _)| p.starts_with(&prefix))
    }

    /// `holds` returns whether `path` is a destination of the lock or a
    /// directory above one.
    fn holds(&self, path: &str) -> bool {
        self.records.contains_key(path) || self.under(path).next().is_some()
    }

    /// `is_ignored` returns whether git ignores `path` or a directory above
    /// it.
    fn is_ignored(&self, path: &str) -> bool {
        let Some(ignored) = &self.ignored else {
            return false;
        };
        let mut p = path;
        loop {
            if ignored.contains(p) {
                return true;
            }
            match p.rfind('/') {
                Some(0) | None => return false,
                Some(i) => p = &p[..i],
            }
        }
    }

    /// `keeps` returns whether a record passes the filters of `--pack`,
    /// `--state`, and `--acceptance`.
    // sokf-implements: CLI-5_AC-33
    // sokf-implements: CLI-5_AC-34
    // sokf-implements: CLI-5_AC-35
    fn keeps(&self, record: &Record) -> bool {
        self.options.pack.as_ref().is_none_or(|p| *p == record.pack)
            && self.options.state.is_none_or(|s| s == record.state)
            && self
                .options
                .acceptance
                .is_none_or(|a| a.keeps(record.acceptance))
    }

    /// `filtered` returns whether `--pack`, `--state`, or `--acceptance` is
    /// given.
    fn filtered(&self) -> bool {
        self.options.pack.is_some()
            || self.options.state.is_some()
            || self.options.acceptance.is_some()
    }

    /// `entry` returns the entry of `path`, or none when the filters leave
    /// it out: a file without a record that they keep, or a directory that
    /// holds none.
    // sokf-implements: CLI-5_AC-1
    // sokf-implements: CLI-5_AC-18
    // sokf-implements: CLI-5_AC-35
    fn entry(&self, path: &str, directory: bool) -> Option<PackLsEntry> {
        if directory {
            let kept: Vec<&Record> = self
                .under(path)
                .map(|(_, r)| r)
                .filter(|r| self.keeps(r))
                .collect();
            if self.filtered() && kept.is_empty() {
                return None;
            }
            let mut counts = PackLsCounts::default();
            for r in &kept {
                counts.add(r.state, r.acceptance);
            }
            let (state, acceptance) = directory_state(&kept);
            return Some(PackLsEntry {
                path: path.to_string(),
                directory: true,
                pack: None,
                kind: None,
                state,
                acceptance,
                note: None,
                counts: Some(counts),
            });
        }
        let record = self.records.get(path).filter(|r| self.keeps(r));
        if self.filtered() && record.is_none() {
            return None;
        }
        Some(PackLsEntry {
            path: path.to_string(),
            directory: false,
            pack: record.map(|r| r.pack.clone()),
            kind: record.map(|r| r.kind),
            state: record.map(|r| r.state),
            acceptance: record.and_then(|r| r.acceptance),
            note: record.and_then(|r| r.note.clone()),
            counts: None,
        })
    }

    /// `is_dir` returns whether `path` is a directory: false for a
    /// destination of the lock, true for a directory above one, and
    /// otherwise as the disk says, following a symbolic link only with
    /// `follow`, as `ls` follows one that names a path, or none when it is
    /// neither on disk nor a destination nor above one.
    // The lock decides over the disk, so that a destination that the user
    // replaced with a directory, or a directory of destinations that the user
    // replaced with a file, lists as the lock records it.
    fn is_dir(&self, path: &str, follow: bool) -> Option<bool> {
        if self.records.contains_key(path) {
            return Some(false);
        }
        if self.under(path).next().is_some() {
            return Some(true);
        }
        let fs = RepoPath::parse(path)?.to_fs(self.root);
        let meta = if follow {
            fs::metadata(&fs).or_else(|_| fs::symlink_metadata(&fs))
        } else {
            fs::symlink_metadata(&fs)
        };
        meta.ok().map(|m| m.is_dir())
    }

    /// `list_dir` lists the entries of the directory `dir` into `out`, and,
    /// with `-R`, each directory among them after it.
    // sokf-implements: CLI-5_AC-4
    // sokf-implements: CLI-5_AC-5
    // sokf-implements: CLI-5_AC-8
    // sokf-implements: CLI-5_AC-9
    // sokf-implements: CLI-5_AC-10
    fn list_dir(&self, dir: &str, out: &mut Vec<PackLsListing>) {
        let mut names: BTreeSet<String> = BTreeSet::new();
        let fs_dir = RepoPath::parse(dir).map(|p| p.to_fs(self.root));
        if let Some(read) = fs_dir.and_then(|d| fs::read_dir(d).ok()) {
            names.extend(
                read.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned()),
            );
        }
        // Each destination that is absent from disk stays in its directory,
        // and each absent directory above one stays in the directory above
        // it.
        let prefix = below(dir);
        for (path, _) in self.under(dir) {
            let rest = &path[prefix.len()..];
            names.insert(rest.split('/').next().unwrap_or(rest).to_string());
        }
        let mut entries = Vec::new();
        let mut subdirs = Vec::new();
        for name in names {
            let path = join(dir, &name);
            let directory = self.is_dir(&path, false).unwrap_or(false);
            let delivered = self.holds(&path);
            if !delivered
                && ((!self.options.all && name.starts_with('.')) || self.is_ignored(&path))
            {
                continue;
            }
            if let Some(entry) = self.entry(&path, directory) {
                if directory {
                    subdirs.push(path);
                }
                entries.push(entry);
            }
        }
        out.push(PackLsListing {
            dir: Some(dir.to_string()),
            entries,
        });
        if self.options.recursive {
            for sub in subdirs {
                self.list_dir(&sub, out);
            }
        }
    }
}

/// `directory_state` returns the state and the acceptance of a directory
/// whose files of packs, below it at any depth, have the records `records`:
/// the most serious state of the files that differ from their base and
/// whose acceptance isn't `accepted`, with no acceptance; failing that, the
/// most serious state of the files that differ from their base, with the
/// acceptance `accepted`; and failing that, `unchanged` where a file holds
/// its base, with no acceptance.
// sokf-implements: CLI-5_AC-13
// sokf-implements: CLI-5_AC-30
// sokf-implements: CLI-5_AC-31
// sokf-implements: CLI-5_AC-38
fn directory_state(records: &[&Record]) -> (Option<PackLsState>, Option<PackAcceptance>) {
    let worst =
        |keep: &dyn Fn(&Record) -> bool| records.iter().filter(|r| keep(r)).map(|r| r.state).max();
    // An editable file that is edited counts as an accepted file, because
    // neither fails the check.
    let accepted = |r: &Record| {
        r.acceptance == Some(PackAcceptance::Accepted)
            || (r.kind == PackFileKind::Editable && r.state == PackLsState::Edited)
    };
    if let Some(state) = worst(&|r| r.state.differs() && !accepted(r)) {
        return (Some(state), None);
    }
    if let Some(state) = worst(&|r| r.state.differs()) {
        return (Some(state), Some(PackAcceptance::Accepted));
    }
    (worst(&|_| true), None)
}

/// `pack_ls` lists each path of `options.paths`, or the current directory
/// without one, as `ls` does: the paths that aren't directories first, then
/// the entries of each directory, each group in the order of the names, with
/// the pack, the kind, the state, the acceptance, and the note of each file
/// of a pack, and the state, the acceptance, and the counts of each
/// directory. It refuses, with every reason at once,
/// a path that is neither an entry of the tree nor a destination of the lock
/// nor a directory above one. It reads the lock, the configuration for the
/// order of the packs, the working tree, and git's list of the files that it
/// ignores, and writes nothing.
// sokf-implements: CLI-5_AC-2
// sokf-implements: CLI-5_AC-3
// sokf-implements: CLI-5_AC-19
// sokf-implements: CLI-5_AC-22
// sokf-implements: CLI-5_AC-23
pub fn pack_ls(root: &RepoRoot, options: &PackLsOptions) -> Result<PackLs> {
    let lock = pack::read_lock(root)?;
    let meaning = pack::Meaning::open(root, pack::For::Reports)?;
    let listed = listing(root, options, &lock, &meaning);
    meaning.settle(listed, |r, w| r.warnings = w)
}

/// `listing` returns the listing of `pack_ls` of the lock `lock`, with the
/// comparison `meaning`, without its warnings, which `pack_ls` adds.
fn listing(
    root: &RepoRoot,
    options: &PackLsOptions,
    lock: &pack::Lock,
    meaning: &pack::Meaning,
) -> Result<PackLs> {
    let mut lister = Lister {
        root,
        options,
        records: records(root, meaning, lock),
        ignored: None,
    };
    let mut refused = Vec::new();
    if let Some(name) = options
        .pack
        .as_ref()
        .filter(|n| !lock.packs.contains_key(*n))
    {
        let names: Vec<&str> = lock.packs.keys().map(String::as_str).collect();
        refused.push(format!(
            "--pack: the lock holds no pack named `{name}`; it holds {}",
            if names.is_empty() {
                "none".to_string()
            } else {
                names.join(", ")
            }
        ));
    }
    let mut paths = Vec::new();
    if options.paths.is_empty() {
        match RepoPath::from_fs(root, &options.cwd) {
            Some(p) => paths.push((".".to_string(), p)),
            None => refused.push("the current directory lies outside the repository".to_string()),
        }
    }
    for arg in &options.paths {
        match PathArg(arg).resolve_new(root, &options.cwd) {
            Some(p) => paths.push((arg.clone(), p)),
            None => refused.push(format!("{arg}: not a path in the repository")),
        }
    }
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for (arg, path) in paths {
        // A path that ends with `/` names a directory, as for `ls`.
        let dir_only = arg.len() > 1 && arg.ends_with('/');
        match lister.is_dir(path.as_str(), true) {
            Some(true) => dirs.push(path.as_str().to_string()),
            Some(false) if !dir_only => files.push(path.as_str().to_string()),
            Some(false) => refused.push(format!("{arg}: not a directory")),
            None => refused.push(format!(
                "{arg}: no such entry in the tree, and no file of a pack that the lock records"
            )),
        }
    }
    if !refused.is_empty() {
        return Err(Error::Pack(refused.join("; ")));
    }
    files.sort();
    dirs.sort();
    // git's list matters only below a directory that the command lists,
    // and not with a filter, which keeps only the files of packs.
    if !options.ignored && !dirs.is_empty() && !lister.filtered() {
        lister.ignored = Some(ignored_paths(root, &dirs));
    }
    let headers = files.len() + dirs.len() > 1 || options.recursive;
    let mut listings = Vec::new();
    let entries: Vec<PackLsEntry> = files
        .iter()
        .filter_map(|p| lister.entry(p, false))
        .collect();
    if !entries.is_empty() {
        listings.push(PackLsListing { dir: None, entries });
    }
    for dir in &dirs {
        lister.list_dir(dir, &mut listings);
    }
    Ok(PackLs {
        listings,
        headers,
        warnings: Vec::new(),
    })
}

#[cfg(test)]
mod tests;
