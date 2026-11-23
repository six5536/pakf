//! This module implements `pakf patch`: the patch that sends a
//! project's edits of a pack's files, and the files that the pack doesn't
//! hold yet, back to the pack's repository, and with `--apply` its
//! application to the working tree of a pack whose source is a local
//! directory. It uses no network.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use super::pack_diff::{Purpose, cant_read, edits};
use crate::{Error, RepoPath, RepoRoot, Result, config, pack};

/// `PackPatchOptions` holds the options of `pakf patch`.
#[derive(Debug, Clone, Default)]
pub struct PackPatchOptions {
    /// The pack's name.
    pub name: String,
    /// Whether to take the edits that the project accepted too.
    pub accepted: bool,
    /// The files of the pack whose edits to take, and the files to add to
    /// the pack, as the user wrote them, or none for every edit of the pack.
    pub paths: Vec<String>,
    /// Whether a directory of `paths` takes every file below it.
    pub recursive: bool,
    /// Whether to apply the patch to the working tree of the pack's
    /// directory, where the pack's source is a local directory.
    pub apply: bool,
    /// The directory from which a relative path of `paths` is read.
    pub cwd: PathBuf,
}

/// A `PackPatch` is what `pakf patch` makes: the patch, the notes for a
/// person, which go to stderr, and, with `--apply`, the files that it
/// applied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackPatch {
    /// The unified diff of the edits and of the new files.
    pub patch: String,
    /// A note for each edited file that the patch leaves out, because an
    /// entry that isn't a readable regular file stands at its path, or
    /// because a named file holds its accepted version without `--accepted`,
    /// for each named new file that an ignore names, and for each file that
    /// the walk of `-r` leaves out because sync would refuse to land it.
    pub notes: Vec<String>,
    /// With `--apply`, the path of each file of the patch in the pack's
    /// directory, as the source names the directory.
    pub applied: Vec<String>,
}

/// `pack_patch` returns the patch of the pack `options.name`: the edits that
/// `edits` finds, of every file of the pack or of those that
/// `options.paths` selects, followed by each new file of the pack among the
/// paths, as `selection` finds them. With `options.apply`, it refuses a
/// source that isn't a local directory before it reads a file, and then
/// applies the patch to the working tree of the pack's directory.
// sokf-implements: CLI-9_AC-6
// sokf-implements: CLI-9_AC-12
// sokf-implements: CLI-9_AC-15
pub fn pack_patch(root: &RepoRoot, options: &PackPatchOptions) -> Result<PackPatch> {
    let name = options.name.as_str();
    let (lock, entry) = super::pack_diff::locked_by_name(root, name, "pakf patch")?;
    let target = options
        .apply
        .then(|| apply_target(root, name, &entry))
        .transpose()?;
    // The paths refuse before any diff runs, so a refusal prints no patch.
    let chosen = if options.paths.is_empty() {
        None
    } else {
        Some(selection(root, &lock, &entry, options)?)
    };
    // An editable file's edit is the project's own, so a patch without a
    // path leaves it out.
    // sokf-implements: CLI-9_AC-36
    let editable = editable_dests(root, &entry);
    let e = edits(
        root,
        name,
        &entry,
        options.accepted,
        Purpose::Patch,
        &|dest| match &chosen {
            Some(c) => c.edits.contains(dest),
            None => !editable.contains(dest),
        },
    )?;
    let mut out = PackPatch {
        patch: e.patch,
        notes: e.notes,
        applied: Vec::new(),
    };
    out.notes
        .extend(chosen.iter().flat_map(|c| c.notes.iter().cloned()));
    let mut rels = e.rels;
    // sokf-implements: CLI-9_AC-3
    // sokf-implements: CLI-9_AC-4
    for n in chosen.iter().flat_map(|c| &c.new) {
        let modes = pack::carries_mode(&n.rel).then_some((false, n.executable));
        out.patch.push_str(&pack::diff_modes(
            &n.rel,
            None,
            Some(&pack::normalise(&n.bytes)),
            modes,
        )?);
        rels.push(n.rel.clone());
    }
    if let Some((dir, label)) = target
        && !out.patch.is_empty()
    {
        pack::apply(&dir, &out.patch)?;
        out.applied = rels.iter().map(|rel| format!("{label}/{rel}")).collect();
    }
    Ok(out)
}

/// `editable_dests` returns the destination of each editable file that the
/// pack of the record `entry` owns and at which an entry stands, so that
/// the patch still carries the deletion of an editable file, which needs an
/// acceptance as any deletion does.
fn editable_dests(root: &RepoRoot, entry: &pack::LockedPack) -> BTreeSet<String> {
    entry
        .files
        .values()
        .filter(|f| f.editable && f.shadowed_by.is_none())
        .filter(|f| RepoPath::parse(&f.path).is_some_and(|p| !pack::is_absent(&p.to_fs(root))))
        .map(|f| f.path.clone())
        .collect()
}

/// `apply_target` returns the pack's directory in the working tree of the
/// source of the pack `name`, whose record is `entry`, and the directory as
/// the source names it, or refuses a source that isn't a local directory.
fn apply_target(
    root: &RepoRoot,
    name: &str,
    entry: &pack::LockedPack,
) -> Result<(PathBuf, String)> {
    let Some((url, dir)) = local_dir(root, &entry.source, entry.path.as_deref()) else {
        return Err(Error::Pack(format!(
            "--apply: the pack {name} comes from {}, which isn't a local directory, so pakf has no working tree to write; `-o <FILE>` writes the patch for the pack's repository",
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
fn local_dir(
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

/// A `NewFile` is a file of the project that `pakf patch` adds to the pack.
struct NewFile {
    /// The file's path in the pack.
    rel: String,
    /// The file's bytes.
    bytes: Vec<u8>,
    /// Whether the file is executable, which only a file of `files/` keeps.
    executable: bool,
}

/// A `Selection` is what the paths of `pakf patch` take: the destinations of
/// the files of the pack whose edits the patch carries, and the new files.
#[derive(Default)]
struct Selection {
    edits: BTreeSet<String>,
    new: Vec<NewFile>,
    /// A note for each file that the patch leaves out with a word, as
    /// `PackPatch::notes` says.
    notes: Vec<String>,
}

/// A `Selector` holds what `selection` reads once.
struct Selector<'a> {
    root: &'a RepoRoot,
    options: &'a PackPatchOptions,
    entry: &'a pack::LockedPack,
    /// The destination of each file that the pack owns.
    owned: BTreeSet<RepoPath>,
    /// The destination of each editable file that the pack owns, which the
    /// patch takes only where a path names it.
    editable: BTreeSet<String>,
    /// The pack's directory in the source of each pack that the lock
    /// records or the configuration declares whose source is a local
    /// directory of the repository, by the pack's name.
    sources: BTreeMap<RepoPath, String>,
    /// The owner of each destination that a pack in the lock records.
    owners: BTreeMap<String, String>,
    /// The remaps of the paths of the pack's files that the lock records.
    remap: pack::Remap,
    /// The ignores, which leave paths out of the new files, read when the
    /// patch first meets a new file, so that an edit reads none: `None`
    /// before, and an ignore file that can't be read after.
    ignores: Option<std::result::Result<pack::Ignores, String>>,
    /// Whether the patch has refused an ignore file that can't be read.
    broken_ignores_refused: bool,
    /// The path in the pack and the destination of each new file so far.
    landed: Vec<(String, String)>,
    refused: Vec<String>,
    seen: BTreeSet<RepoPath>,
    out: Selection,
}

/// `selection` reads the paths of `options`, in their order and once each:
/// a file that the pack owns is an edit, and a file that no pack records is
/// a new file, while a directory takes, with `options.recursive`, both kinds
/// of file below it. It refuses, with every reason at once, a path outside
/// the repository, a file that another pack owns, a directory without
/// `options.recursive`, a path at which nothing stands and below which the
/// pack records no file, a file that stands where the pack holds files
/// below it, an entry that is neither a readable regular file nor a
/// directory, which it reads by the one reading of a destination, and a new
/// file that has no place in the pack.
// sokf-implements: CLI-9_AC-5
// sokf-implements: CLI-9_AC-27
// sokf-implements: CLI-9_AC-28
// sokf-implements: CLI-9_AC-7
fn selection(
    root: &RepoRoot,
    lock: &pack::Lock,
    entry: &pack::LockedPack,
    options: &PackPatchOptions,
) -> Result<Selection> {
    let mut owners = BTreeMap::new();
    for (name, p) in &lock.packs {
        for f in p.files.values() {
            let owner = f.shadowed_by.clone().unwrap_or_else(|| name.clone());
            owners.entry(f.path.clone()).or_insert(owner);
        }
    }
    let owned = entry
        .files
        .values()
        .filter(|f| f.shadowed_by.is_none())
        .filter_map(|f| RepoPath::parse(&f.path))
        .collect();
    // A pack that the configuration declares and that no sync has recorded
    // yet has its source too (CLI-9_AC-24), which only the walk of `-r`
    // reads.
    let declared = if options.recursive {
        config::read_packs(root)?
    } else {
        Vec::new()
    };
    let mut sources = BTreeMap::new();
    let all = lock
        .packs
        .iter()
        .map(|(name, p)| (name, p.source.as_str(), p.path.as_deref()))
        .chain(
            declared
                .iter()
                .map(|p| (&p.name, p.source.as_str(), p.path.as_deref())),
        );
    let mut broken = Vec::new();
    for (name, source, path) in all {
        let dir = match local_dir(root, source, path) {
            Some((_, Ok(dir))) => dir,
            // A source whose directory pakf can't place can't be left out,
            // so `-r` refuses it, as sync does (CLI-2_AC-17).
            Some((_, Err(e))) => {
                broken.push(format!("the source of the pack {name}: {e}"));
                continue;
            }
            None => continue,
        };
        // A pack at the root of its local source holds the repository's
        // own layout, so no directory lies in it apart.
        if let Some(dir) = RepoPath::from_fs(root, &dir).filter(|d| *d != RepoPath::root()) {
            sources.entry(dir).or_insert_with(|| name.clone());
        }
    }
    let remap = super::pack_run::remap_of(&options.name, &entry.paths)?;
    if options.recursive && !broken.is_empty() {
        return Err(Error::Pack(broken.join("; ")));
    }
    let mut s = Selector {
        root,
        options,
        entry,
        owned,
        editable: editable_dests(root, entry),
        sources,
        owners,
        remap,
        ignores: None,
        broken_ignores_refused: false,
        landed: Vec::new(),
        refused: Vec::new(),
        seen: BTreeSet::new(),
        out: Selection::default(),
    };
    for arg in &options.paths {
        let Some(dest) = config::PathArg(arg).resolve_new(root, &options.cwd) else {
            s.refused
                .push(format!("{arg}: not a path in the repository"));
            continue;
        };
        s.named(&dest)?;
    }
    if !s.refused.is_empty() {
        return Err(Error::Pack(s.refused.join("; ")));
    }
    Ok(s.out)
}

impl Selector<'_> {
    /// `holds_below` returns whether the pack owns a file below `dest`.
    fn holds_below(&self, dest: &RepoPath) -> bool {
        self.owned.iter().any(|d| d != dest && d.starts_with(dest))
    }

    /// `named` takes the path `dest` that the user named.
    fn named(&mut self, dest: &RepoPath) -> Result<()> {
        if !self.seen.insert(dest.clone()) {
            return Ok(());
        }
        // A path that names an editable file takes its edit.
        // sokf-implements: CLI-9_AC-37
        if self.owned.contains(dest) {
            self.owned_file(dest, true);
            return Ok(());
        }
        if let Some(owner) = self.owners.get(dest.as_str()) {
            self.refused.push(format!(
                "{dest}: the pack {owner} owns the file, so its patch carries the edit"
            ));
            return Ok(());
        }
        let path = dest.to_fs(self.root);
        let holds = self.holds_below(dest);
        if is_dir(&path) || (holds && pack::is_absent(&path)) {
            if self.options.recursive {
                return self.directory(dest);
            }
            self.refused.push(format!(
                "{dest}: a directory, and the patch takes the files below a directory only with -r"
            ));
            return Ok(());
        }
        if holds {
            self.refused.push(in_the_way(dest));
            return Ok(());
        }
        match pack::read_dest(&path) {
            pack::Dest::File(bytes) => self.new_file(dest, bytes, false),
            pack::Dest::Absent => self.refused.push(format!("{dest}: no such file")),
            pack::Dest::Unreadable(why) => self.refused.push(cant_read(dest.as_str(), &why)),
        }
        Ok(())
    }

    /// `owned_file` takes the edit of the file `dest` that the pack owns,
    /// and refuses an entry at its path that isn't a readable regular file.
    /// A file that the user `named` and that holds its accepted version gets
    /// a note without `--accepted`, because the patch leaves it out.
    // sokf-implements: CLI-9_AC-18
    // sokf-implements: CLI-9_AC-34
    fn owned_file(&mut self, dest: &RepoPath, named: bool) {
        if let pack::Dest::Unreadable(why) = pack::read_dest(&dest.to_fs(self.root)) {
            self.refused.push(cant_read(dest.as_str(), &why));
            return;
        }
        let held = self
            .entry
            .files
            .iter()
            .find(|(_, f)| f.path == dest.as_str())
            .is_some_and(|(rel, f)| super::pack::holds_accepted(self.root, rel, f));
        if named && held && !self.options.accepted {
            self.out.notes.push(format!(
                "{dest}: the file holds the edit that the project accepted, so the patch leaves it out; --accepted takes it"
            ));
        }
        self.out.edits.insert(dest.as_str().to_string());
    }

    /// `directory` takes each file of the pack below the directory `dir`,
    /// and each file below it that git doesn't ignore and that no pack in
    /// the lock records, and leaves out each file that another pack owns, the
    /// files under `/.pakf/` outside the pack's own folder of
    /// `/.pakf/ignore.d/`, each path that the ignores name, and the pack's
    /// directory in each local source of a pack. It refuses a directory that is a git
    /// repository of its own or lies below one.
    // sokf-implements: CLI-9_AC-8
    // sokf-implements: CLI-9_AC-9
    // sokf-implements: CLI-9_AC-19
    // sokf-implements: CLI-9_AC-20
    // sokf-implements: CLI-9_AC-22
    // sokf-implements: CLI-9_AC-24
    // sokf-implements: CLI-9_AC-26
    fn directory(&mut self, dir: &RepoPath) -> Result<()> {
        let owned: Vec<RepoPath> = self
            .owned
            .iter()
            .filter(|d| d.starts_with(dir))
            .cloned()
            .collect();
        let holds = !owned.is_empty();
        // An editable file below the directory goes into the patch only
        // where a path names it, before or after the directory.
        // sokf-implements: CLI-9_AC-36
        for dest in owned {
            if self.editable.contains(dest.as_str()) {
                continue;
            }
            if self.seen.insert(dest.clone()) {
                self.owned_file(&dest, false);
            }
        }
        // A directory of the pack's files that the project deleted holds no
        // file to walk, and pakf's and git's own files are no pack's, nor is
        // a directory that git ignores. A named directory that holds none of
        // the pack's files and that the walk skips is refused, so that its
        // files go missing with a word.
        let fs_dir = dir.to_fs(self.root);
        if pack::is_absent(&fs_dir) {
            return Ok(());
        }
        if is_own(dir, &self.options.name) {
            self.skip(
                dir,
                holds,
                "pakf's or git's own files, which no pack takes as new files",
            );
            return Ok(());
        }
        // The ignores name a directory as git does.
        // sokf-implements: CLI-9_AC-35
        if let Some(by) = self.ignored(dir, true) {
            let why = format!("{by} names the directory, so the patch takes no new file below it");
            self.skip(dir, holds, &why);
            return Ok(());
        }
        // Inside a git repository, a failure of git, or a file that git
        // can't read, leaves the patch without git's rules, so it refuses,
        // while outside one no rule of git applies.
        let listed = match crate::git::ignored(self.root, &[dir.as_str().to_string()]) {
            Ok(ignored) => ignored.warning().map_or(Ok(ignored.paths), Err),
            Err(why) if in_repository(self.root) => Err(why),
            // sokf-implements: CLI-9_AC-21
            Err(_) => Ok(BTreeSet::new()),
        };
        if let Ok(ignored) = &listed
            && is_ignored(dir, ignored)
        {
            self.skip(
                dir,
                holds,
                "git ignores the directory, so the patch takes no file below it",
            );
            return Ok(());
        }
        // A directory that the patch refuses whatever git ignores gives its
        // own reason, before git's.
        if let Some((source, name)) = self.sources.iter().find(|(s, _)| dir.starts_with(s)) {
            self.refused.push(format!(
                "{dir}: the directory lies in {source}, the source of the pack {name}, whose files no pack takes as new files"
            ));
            return Ok(());
        }
        // A directory below a git repository of its own lies in that
        // repository, whose ignored files the outer git names none of, and
        // the pack's own files there go back by their own paths, as for the
        // repository's own directory.
        let repository = std::iter::successors(Some(dir.clone()), RepoPath::parent)
            .take_while(|d| *d != RepoPath::root())
            .find(|d| is_repository(&d.to_fs(self.root)));
        if let Some(repository) = repository {
            let what = if repository == *dir {
                String::new()
            } else {
                format!("the directory lies in {repository}, ")
            };
            self.refused.push(format!(
                "{dir}: {what}a git repository of its own, whose files the patch takes only by their own paths"
            ));
            return Ok(());
        }
        let ignored = match listed {
            Ok(ignored) => ignored,
            Err(why) => {
                self.refused.push(format!(
                    "{dir}: pakf can't tell which files below the directory git ignores: {why}"
                ));
                return Ok(());
            }
        };
        self.walk(dir, &ignored)
    }

    /// `skip` refuses the directory `dir` that the walk skips for the reason
    /// `why`, unless it `holds` files of the pack.
    fn skip(&mut self, dir: &RepoPath, holds: bool, why: &str) {
        if !holds {
            self.refused.push(format!("{dir}: {why}"));
        }
    }

    /// `walk` takes each entry below the directory `dir` that git doesn't
    /// ignore, in the order of their names, follows no symbolic link, and
    /// enters no git repository of its own, whose files git leaves to that
    /// repository. It refuses a directory that it can't read and a name that
    /// isn't UTF-8, so that the patch leaves out no file without a word.
    fn walk(&mut self, dir: &RepoPath, ignored: &BTreeSet<String>) -> Result<()> {
        let entries = match fs::read_dir(dir.to_fs(self.root)) {
            Ok(entries) => entries,
            Err(e) => {
                self.refused.push(cant_read(dir.as_str(), &e.to_string()));
                return Ok(());
            }
        };
        let mut names = Vec::new();
        for entry in entries {
            match entry.map(|e| e.file_name().into_string()) {
                Ok(Ok(name)) => names.push(name),
                Ok(Err(name)) => self.refused.push(format!(
                    "{dir}/{}: the name isn't UTF-8, and every path of a pack is",
                    name.to_string_lossy()
                )),
                Err(e) => self.refused.push(cant_read(dir.as_str(), &e.to_string())),
            }
        }
        names.sort();
        for name in names {
            let Some(child) = dir.join(&name) else {
                continue;
            };
            let own = self.options.name.as_str();
            if is_own(&child, own)
                || is_ignored(&child, ignored)
                || self.sources.contains_key(&child)
            {
                continue;
            }
            // A path that a pack records is that pack's, whatever stands
            // there, so the walk takes nothing at it or below it.
            if self.owners.contains_key(child.as_str()) {
                continue;
            }
            let path = child.to_fs(self.root);
            let child_is_dir = is_dir(&path);
            // The walk checked each directory above the child on its way down.
            // sokf-implements: CLI-9_AC-9
            let ignored_here = self
                .ignores()
                .and_then(|ig| ig.ignored_here(&child, child_is_dir))
                .is_some();
            if child_is_dir {
                if !ignored_here && !is_repository(&path) {
                    self.walk(&child, ignored)?;
                }
                continue;
            }
            if !self.seen.insert(child.clone()) {
                continue;
            }
            // A file in the way of the pack's directory is refused, ignored
            // or not, because the pack's files below it read as deleted.
            if self.holds_below(&child) {
                self.refused.push(in_the_way(&child));
                continue;
            }
            if ignored_here {
                continue;
            }
            match pack::read_dest(&path) {
                pack::Dest::File(bytes) => self.new_file(&child, bytes, true),
                pack::Dest::Unreadable(why) => self.refused.push(cant_read(child.as_str(), &why)),
                pack::Dest::Absent => {}
            }
        }
        Ok(())
    }

    /// `ignores` returns the ignores, which it reads the first time, or
    /// none where an ignore file can't be read, whose refusal
    /// `refuse_broken_ignores` gives once a new file meets it.
    fn ignores(&mut self) -> Option<&pack::Ignores> {
        let root = self.root;
        self.ignores
            .get_or_insert_with(|| {
                pack::Ignores::read(root).map_err(|e| match e {
                    crate::Error::Pack(m) => m,
                    other => other.to_string(),
                })
            })
            .as_ref()
            .ok()
    }

    /// `ignored` returns the ignore file whose pattern names `path`, or a
    /// directory above it, where `is_dir` says that `path` is a directory,
    /// or none.
    fn ignored(&mut self, path: &RepoPath, is_dir: bool) -> Option<RepoPath> {
        self.ignores()?.ignored(path, is_dir)
    }

    /// `refuse_broken_ignores` refuses, once, an ignore file that can't be
    /// read, when the patch meets a new file, because a patch without one of
    /// its ignores would send a file that it names, and a patch of edits
    /// alone reads no ignore.
    // sokf-implements: CONF-5_AC-3
    fn refuse_broken_ignores(&mut self) -> bool {
        self.ignores();
        match &self.ignores {
            Some(Err(why)) => {
                if !self.broken_ignores_refused {
                    self.broken_ignores_refused = true;
                    self.refused.push(why.clone());
                }
                true
            }
            _ => false,
        }
    }

    /// `new_file` adds the file `dest`, which holds `bytes`, at its place in
    /// the pack, through the inverse of the pack's layout with the remaps
    /// that the lock records, or refuses a file that sync would refuse to
    /// land, and a named file that has no place in the pack. It leaves out
    /// a file that the walk of `-r` found, as `walked` says, that has no
    /// place in the pack, because the remaps send no file of the pack there,
    /// and a file that the ignores name, with a note for a named one, and a
    /// file that sync would refuse to land, which it refuses where named.
    // sokf-implements: CLI-9_AC-33
    fn new_file(&mut self, dest: &RepoPath, bytes: Vec<u8>, walked: bool) {
        if self.refuse_broken_ignores() {
            return;
        }
        // The walk checked the ignores of the file and of each directory
        // above it.
        if !walked && let Some(by) = self.ignored(dest, false) {
            self.out.notes.push(format!(
                "{dest}: {by} names the path, so the patch leaves it out"
            ));
            return;
        }
        let rel = match pack::origin(&self.options.name, &self.remap, dest.as_str()) {
            Ok(rel) => rel,
            Err(_) if walked => return,
            Err(why) => {
                self.refused.push(format!("{dest}: {why}"));
                return;
            }
        };
        if let Some(why) = pack::refusal(&self.options.name, &rel, &bytes, dest.as_str()) {
            // The walk leaves out, with a word, a file that no pack lands
            // there, such as a file in another pack's folder of a
            // `*.d/pakf/` directory.
            if walked {
                self.out.notes.push(format!(
                    "{dest}: sync would refuse the file, so the patch leaves it out: {why}"
                ));
            } else {
                self.refused
                    .push(format!("{dest}: sync would refuse the file: {why}"));
            }
            return;
        }
        // Two files of a pack never land at one path in two cases of its
        // letters (CLI-2_AC-69).
        let lower = dest.as_str().to_lowercase();
        let other = self
            .entry
            .files
            .iter()
            .map(|(r, f)| (r.as_str(), f.path.as_str()))
            .chain(self.landed.iter().map(|(r, d)| (r.as_str(), d.as_str())))
            .find(|(_, d)| d.to_lowercase() == lower);
        if let Some((other, _)) = other {
            let why = format!(
                "{rel} lands at {dest}, where the pack's {other} lands too, so the pack has no one version of the file"
            );
            if walked {
                self.out.notes.push(format!(
                    "{dest}: sync would refuse the file, so the patch leaves it out: {why}"
                ));
            } else {
                self.refused
                    .push(format!("{dest}: sync would refuse the file: {why}"));
            }
            return;
        }
        self.landed.push((rel.clone(), dest.as_str().to_string()));
        let path = dest.to_fs(self.root);
        let executable = pack::carries_mode(&rel) && pack::file_is_executable(&path);
        self.out.new.push(NewFile {
            rel,
            bytes,
            executable,
        });
    }
}

/// `in_the_way` returns the refusal of a file at `dest`, below which the
/// pack holds files.
// sokf-implements: CLI-9_AC-17
fn in_the_way(dest: &RepoPath) -> String {
    format!(
        "{dest}: a file stands where the pack holds a directory of files, and the patch turns no directory of the pack into a file"
    )
}

/// `is_own` returns whether `dest` lies in a `.git` directory, or in
/// `/.pakf/`, which holds pakf's own files and the copies of the packs,
/// outside the folder of the pack `name` in `/.pakf/ignore.d/`, where the
/// landing of a file lets the pack write, never a file that the pack takes
/// new. A directory above that folder, on the way to it, is no own file
/// either, so that a named directory and the walk of `-r` alike reach the
/// folder. It compares `.git` and `.pakf` without regard to the case of
/// letters, as the landing of a file does, because macOS and Windows open
/// `.GIT/` as `.git/`, and the rest of the way in the case that pakf reads.
fn is_own(dest: &RepoPath, name: &str) -> bool {
    if dest.segments().any(|s| s.eq_ignore_ascii_case(".git")) {
        return true;
    }
    if !dest
        .segments()
        .next()
        .is_some_and(|s| s.eq_ignore_ascii_case(".pakf"))
    {
        return false;
    }
    let folder = pack::own_ignores(name);
    let path = dest.as_str();
    !(path == folder || pack::below(&folder, path) || pack::below(path, &folder))
}

/// `is_ignored` returns whether git ignores `dest` or a directory above it,
/// as `ignored`, the list of `git ls-files --directory`, names them.
fn is_ignored(dest: &RepoPath, ignored: &BTreeSet<String>) -> bool {
    std::iter::successors(Some(dest.clone()), RepoPath::parent)
        .any(|p| ignored.contains(p.as_str()))
}

/// `in_repository` returns whether the root lies in a git repository, as a
/// `.git` at the root or in a directory above it marks, as for the search
/// of the root, which git can fail to read, such as with an owner that
/// `safe.directory` doesn't allow.
fn in_repository(root: &RepoRoot) -> bool {
    root.as_path().ancestors().any(config::holds_git)
}

/// `is_repository` returns whether a `.git` entry of any kind stands in the
/// directory `dir`, a dangling link among them, which marks a git
/// repository of its own, so that the walk leaves a doubtful one to its own
/// rules rather than take its files with the outer repository's.
fn is_repository(dir: &Path) -> bool {
    fs::symlink_metadata(dir.join(".git")).is_ok()
}

/// `is_dir` returns whether a directory, and no link, stands at `path`.
fn is_dir(path: &std::path::Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}
