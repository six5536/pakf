//! This module runs one sync of the packs that `/.pakf/config.toml`
//! declares, for `pakf sync`, `pakf add`, and `pakf remove`. A sync
//! plans every declared pack together, because the owner of a file depends
//! on every pack: it fetches the packs that it syncs, and reads each other
//! pack from its base copies and its lock entry, without the network.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

use super::{
    pack::{PackSyncReport, SyncedPack, UndeclaredPack, apply, fetch, write},
    pack_info::{Removal, apply_removal, plan_removal},
    pack_keep::PackKeep,
};
use crate::{
    Error, RepoPath, RepoRoot, Result,
    config::{self, PackEntry},
    pack::{
        self, Fetched, Lock, LockedPack, PackFiles, PackManifest, PackPlan, Remap, Request, Source,
    },
};

/// `Fetching` says which declared packs a sync fetches from their sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Fetching<'a> {
    /// Every declared pack.
    All,
    /// The one declared pack of this name.
    One(&'a str),
    /// No pack: every pack comes from its base copies.
    Nothing,
}

/// A `Run` holds what one sync reads and writes beside the lock.
pub(super) struct Run<'a> {
    /// The declared packs, in the order of the configuration.
    pub entries: &'a [PackEntry],
    /// The packs that the sync fetches.
    pub fetching: Fetching<'a>,
    /// The one locked pack that no entry declares that the sync removes, or
    /// `None` for each such pack.
    pub remove_only: Option<&'a str>,
    /// Whether `--keep-existing` is given.
    pub keep_existing: bool,
    /// The text of the configuration to write with the sync, if it changes.
    pub config: Option<String>,
}

/// `run` syncs the packs of `r`. `prefetched` holds a pack that the caller
/// already fetched. It removes each locked pack that no entry declares, and
/// leaves each of its files whose destination a declared pack owns to that
/// pack, as a declared pack leaves a file that it no longer holds only when
/// another declared pack owns it. It
/// refuses a missing or an older dependency and a cycle of dependencies,
/// orders the packs, decides the owner of each destination, and plans every
/// declared pack. It writes the configuration, the
/// files, and the lock only after every plan succeeds.
pub(super) fn run(root: &RepoRoot, r: &Run, prefetched: Option<Fetched>) -> Result<PackSyncReport> {
    run_keeping(root, r, PackKeep::default(), None, prefetched).map(|(report, _)| report)
}

/// A `Kept` is what a removal kept of the pack `Run::remove_only` as the
/// project's own, with `--keep`.
#[derive(Debug, Clone, Default)]
pub(super) struct Kept {
    /// Each file that `--keep files` kept and that exists.
    pub own: Vec<String>,
}

impl Kept {
    /// `of` returns what the removal of the pack `removed` kept: its files
    /// that `removals` keeps as the project's own, which it takes from them.
    fn of(removals: &mut [(String, Removal)], removed: Option<&str>) -> Kept {
        Kept {
            own: removals
                .iter_mut()
                .find(|(n, _)| Some(n.as_str()) == removed)
                .map(|(_, r)| std::mem::take(&mut r.own))
                .unwrap_or_default(),
        }
    }
}

/// `run_keeping` is `run`, which keeps the parts `keep` of the pack
/// `Run::remove_only` as the project's own, hands agent-harness-adapter
/// the text `kept_text` of the pack's instructions to keep, and returns what
/// it kept.
// sokf-implements: CLI-2_AC-33
// sokf-implements: CLI-7_AC-5
pub(super) fn run_keeping(
    root: &RepoRoot,
    r: &Run,
    keep: PackKeep,
    kept_text: Option<&str>,
    prefetched: Option<Fetched>,
) -> Result<(PackSyncReport, Kept)> {
    let lock = pack::read_lock(root)?;
    check_entries(r, &lock)?;
    let declared: BTreeSet<&str> = r.entries.iter().map(|e| e.name.as_str()).collect();
    let undeclared: Vec<String> = lock
        .packs
        .keys()
        .filter(|n| !declared.contains(n.as_str()) && r.remove_only.is_none_or(|o| o == *n))
        .cloned()
        .collect();
    let mut working = lock.clone();
    for name in &undeclared {
        working.packs.remove(name);
    }
    let removed = r.remove_only.filter(|n| undeclared.iter().any(|u| u == n));
    // The clones live until the plans are written, because a plan holds
    // only the bytes that it read from them.
    let packs = gather(root, r, &lock, prefetched)?;
    check_depends(r.entries, &packs)?;
    let order = order_of(&packs)?;
    let base = Request {
        keep_existing: r.keep_existing,
        vacated: BTreeSet::new(),
        owned_dests: BTreeSet::new(),
    };
    let supplies: BTreeMap<String, pack::Supply> = packs
        .iter()
        .map(|(entry, f)| (entry.name.clone(), pack::supply(f)))
        .collect();
    let owners = pack::owners(&order, &supplies, &lock);
    // A file of a removed pack, or a file that a declared pack no longer
    // holds, passes to the declared pack that owns its destination, so
    // neither the removal nor the plan of the pack that it left deletes it.
    let owned_dests: BTreeSet<RepoPath> = supplies
        .values()
        .flat_map(|s| s.files.iter().cloned())
        .collect();
    let mut removals = plan_removals(
        root,
        &lock,
        &undeclared,
        (r.remove_only, keep, kept_text.is_some()),
        &owned_dests,
    );
    // The sync deletes the files of the removed packs, and the files that
    // each declared pack drops, before it writes, so that each plan reads
    // them as absent.
    // sokf-implements: CLI-2_AC-65
    let vacated: BTreeSet<RepoPath> = removals
        .iter()
        .flat_map(|(_, r)| r.deletes.iter().cloned())
        .chain(
            packs
                .iter()
                .flat_map(|(e, f)| pack::dropping(root, lock.packs.get(&e.name), f, &owned_dests)),
        )
        .collect();
    let mut plans: Vec<(bool, PackPlan)> = Vec::new();
    for (entry, f) in &packs {
        let request = Request {
            vacated: vacated.clone(),
            owned_dests: owned_dests.clone(),
            ..base.clone()
        };
        let plan = pack::plan(root, &working, f, &request, &owners[&entry.name])?;
        working.packs.insert(plan.name.clone(), plan.entry.clone());
        plans.push((!f.offline, plan));
    }
    let lock_written = !working.packs.is_empty();
    refuse_writes_below_entries(root, &removals, &plans, lock_written, r.config.is_some())?;
    if let Some(text) = &r.config {
        write(root, &config::config_path(), text.as_bytes())?;
    }
    for (_, removal) in &removals {
        apply_removal(root, removal)?;
    }
    let written: Vec<PackPlan> = plans.iter().map(|(_, p)| p.clone()).collect();
    apply(root, &written, &working)?;
    // The block follows the lock, so it is handed over after the lock.
    let instructions = crate::harness::refresh(root, &working, kept_text);
    // The copy whose text went to the adapter goes once the adapter took it.
    let taken = instructions.held.is_none()
        && instructions.exit < agent_harness_adapter_core::cli::EXIT_FAILURE;
    for (_, removal) in &mut removals {
        removal.settle(root, taken)?;
    }
    let kept = Kept::of(&mut removals, removed);
    Ok((sync_report(removals, plans, instructions), kept))
}

/// `plan_removals` plans the removal of each pack of `undeclared`, and keeps
/// the parts `keep.1` of the pack `keep.0`, whose copy of `instructions.md`
/// it holds back for agent-harness-adapter where `keep.2` says.
fn plan_removals(
    root: &RepoRoot,
    lock: &Lock,
    undeclared: &[String],
    keep: (Option<&str>, PackKeep, bool),
    owned_dests: &BTreeSet<RepoPath>,
) -> Vec<(String, Removal)> {
    undeclared
        .iter()
        .map(|name| {
            let (k, hands) = if keep.0 == Some(name.as_str()) {
                (keep.1, keep.2)
            } else {
                (PackKeep::default(), false)
            };
            let r = plan_removal(root, name, &lock.packs[name], owned_dests, k, hands);
            (name.clone(), r)
        })
        .collect()
}

/// `refuse_writes_below_entries` refuses a sync that writes a file below
/// an entry that is neither a directory nor a link to one once the
/// deletions of `removals` and `plans` are done, at a directory that those
/// deletions leave standing, or below another file that the sync writes,
/// before it writes any file, because the write of such a
/// file fails after the writes before it. The files are those that `plans`
/// write, the base copies among them, the lock when `lock_written` says
/// that the sync writes it, and the configuration when `config` does.
// sokf-implements: CLI-2_AC-66
fn refuse_writes_below_entries(
    root: &RepoRoot,
    removals: &[(String, Removal)],
    plans: &[(bool, PackPlan)],
    lock_written: bool,
    config: bool,
) -> Result<()> {
    let gone: BTreeSet<&RepoPath> = removals
        .iter()
        .flat_map(|(_, r)| &r.deletes)
        .chain(plans.iter().flat_map(|(_, p)| &p.deletes))
        .collect();
    let gone = |p: &RepoPath| gone.contains(p);
    // The sync deletes the lock in place of writing it when no pack is left.
    let (lock, config) = (
        lock_written.then(pack::lock_path),
        config.then(config::config_path),
    );
    let writes: BTreeSet<&RepoPath> = plans
        .iter()
        .flat_map(|(_, p)| p.writes.iter().map(|(path, _)| path))
        .chain(lock.as_ref())
        .chain(config.as_ref())
        .collect();
    let mut below = Vec::new();
    for path in &writes {
        if let Some(dir) =
            std::iter::successors(path.parent(), RepoPath::parent).find(|d| writes.contains(d))
        {
            below.push(format!("{path} below {dir}, which the sync also writes"));
        } else if let Some(dir) = pack::entry_above(root, path, &gone) {
            below.push(format!("{path} below {dir}"));
        } else if fs::symlink_metadata(path.to_fs(root)).is_ok_and(|m| m.is_dir())
            && !pack::empties(root, path, &gone)
        {
            below.push(format!("{path}, at which a directory stands"));
        } else if fs::symlink_metadata(path.to_fs(root)).is_ok_and(|m| m.is_symlink())
            && fs::metadata(path.to_fs(root)).is_ok_and(|m| m.is_dir())
            && !gone(path)
        {
            // A write follows a link at its path, and fails on a directory.
            below.push(format!("{path}, at which a link to a directory stands"));
        }
    }
    if below.is_empty() {
        return Ok(());
    }
    Err(Error::Pack(format!(
        "the sync writes files below entries that aren't directories, or at directories, so it writes no file: {}; move each entry of the project away, or remove one of two packs whose files collide, then sync again",
        below.join("; ")
    )))
}

/// `check_entries` refuses a sync that fetches a pack that no entry of `r`
/// declares, and a sync that fetches when no entry declares a pack and
/// `lock` holds none.
fn check_entries(r: &Run, lock: &Lock) -> Result<()> {
    if let Fetching::One(n) = r.fetching
        && !r.entries.iter().any(|e| e.name == n)
    {
        // A bare name gets the hint of each id of that name.
        let ids = pack::ids_named(r.entries.iter().map(|e| e.name.as_str()), n);
        let hint = if ids.is_empty() {
            String::new()
        } else {
            format!("; a pack is named by its id, such as {}", ids.join(" or "))
        };
        return Err(Error::Pack(format!(
            "no [[pack]] entry of /.pakf/config.toml declares the pack {n}{hint}; add it with `pakf add <SOURCE>`"
        )));
    }
    if r.fetching != Fetching::Nothing && r.entries.is_empty() && lock.packs.is_empty() {
        return Err(Error::Pack(
            "no [[pack]] entry of /.pakf/config.toml declares a pack; add one with `pakf add <SOURCE>`"
                .into(),
        ));
    }
    Ok(())
}

/// `gather` returns each declared pack of `r` that the sync plans, with its
/// files. It fetches each pack that the sync fetches, taking `prefetched`
/// in place of its fetch when it is that pack, and reads each other pack
/// from its base copies and its entry of `lock`. It skips a declared pack
/// that no sync has written yet, and refuses a source that holds another
/// pack.
fn gather<'a>(
    root: &RepoRoot,
    r: &Run<'a>,
    lock: &Lock,
    mut prefetched: Option<Fetched>,
) -> Result<Vec<(&'a PackEntry, Fetched)>> {
    let mut packs: Vec<(&PackEntry, Fetched)> = Vec::new();
    for entry in r.entries {
        let fetch_it = match r.fetching {
            Fetching::All => true,
            Fetching::One(n) => entry.name == n,
            Fetching::Nothing => false,
        };
        let f = if fetch_it {
            let mut f = match prefetched.take() {
                Some(f) if f.pack.manifest.name == entry.name => f,
                other => {
                    prefetched = other;
                    fetch(
                        root,
                        &Source::parse(&entry.source),
                        entry.git_ref.as_deref(),
                        entry.path.as_deref(),
                    )?
                }
            };
            f.remap = remap_of(&entry.name, &entry.paths)?;
            f
        } else if let Some(locked) = lock.packs.get(&entry.name) {
            from_lock(root, &entry.name, locked)?
        } else {
            // A declared pack that no sync has written yet waits for a
            // sync that fetches it.
            continue;
        };
        if f.pack.manifest.name != entry.name {
            return Err(Error::Pack(format!(
                "{}: the source {} holds the pack {}",
                entry.name,
                f.source.label(),
                f.pack.manifest.name
            )));
        }
        packs.push((entry, f));
    }
    Ok(packs)
}

/// `order_of` returns the names of `packs` from the lowest to the highest,
/// so that each pack comes after the packs that it depends on, and refuses
/// a cycle of dependencies.
fn order_of(packs: &[(&PackEntry, Fetched)]) -> Result<Vec<String>> {
    let list: Vec<String> = packs.iter().map(|(e, _)| e.name.clone()).collect();
    let depends: BTreeMap<String, Vec<String>> = packs
        .iter()
        .map(|(e, f)| {
            (
                e.name.clone(),
                f.pack.manifest.depends.keys().cloned().collect(),
            )
        })
        .collect();
    pack::precedence(&list, &depends)
}

/// `sync_report` returns the report of a sync that applied `removals` and
/// `plans`, each plan with whether the sync fetched its pack. It lists a pack that the sync didn't fetch only when its
/// plan acts or notes something.
fn sync_report(
    removals: Vec<(String, Removal)>,
    plans: Vec<(bool, PackPlan)>,
    instructions: crate::Instructions,
) -> PackSyncReport {
    PackSyncReport {
        declared: None,
        instructions,
        undeclared: removals
            .into_iter()
            .map(|(name, r)| UndeclaredPack {
                name,
                removed: r.removed,
                kept: r.kept,
            })
            .collect(),
        packs: plans
            .into_iter()
            .filter(|(fetched, p)| *fetched || !p.actions.is_empty() || !p.notes.is_empty())
            .map(|(fetched, p)| SyncedPack {
                fetched,
                version: p.entry.version.clone(),
                source: p.entry.source.clone(),
                files: p
                    .actions
                    .into_iter()
                    .map(|(d, a)| (d.as_str().to_string(), a))
                    .collect(),
                reasons: p
                    .reasons
                    .into_iter()
                    .map(|(d, why)| (d.as_str().to_string(), why))
                    .collect(),
                notes: p.notes,
                name: p.name,
            })
            .collect(),
    }
}

/// `check_depends` refuses a pack that depends on a pack that no entry of
/// `entries` declares, or on a pack whose version is older than the
/// minimum that it names. `packs` holds each pack that the sync plans.
// sokf-implements: CLI-2_AC-34
// sokf-implements: CLI-2_AC-35
fn check_depends(entries: &[PackEntry], packs: &[(&PackEntry, Fetched)]) -> Result<()> {
    for (entry, f) in packs {
        for (dep, min) in &f.pack.manifest.depends {
            if !entries.iter().any(|e| e.name == *dep) {
                return Err(Error::Pack(format!(
                    "{}: the pack depends on the pack {dep}, which no [[pack]] entry of /.pakf/config.toml declares; add it with `pakf add <SOURCE>`",
                    entry.name
                )));
            }
            let Some((_, d)) = packs.iter().find(|(e, _)| e.name == *dep) else {
                return Err(Error::Pack(format!(
                    "{}: the pack depends on the pack {dep}, which no sync has written yet; run `pakf sync`",
                    entry.name
                )));
            };
            let found = d.version();
            if pack::requires_newer(min, &found) {
                return Err(Error::Pack(format!(
                    "{}: the pack depends on {dep} {min} or newer, and {dep} is at {found}; sync a newer {dep}",
                    entry.name
                )));
            }
        }
    }
    Ok(())
}

/// `remap_of` returns the remaps of the table `paths` of the pack `name`.
pub(super) fn remap_of(name: &str, paths: &BTreeMap<String, String>) -> Result<Remap> {
    Remap::new(paths).map_err(|why| Error::Pack(format!("{name}: the paths of the pack: {why}")))
}

/// `from_lock` reads the locked pack `name` from its base copies and its
/// lock entry, as the last sync fetched it, with the remaps that the lock
/// records, without its source.
fn from_lock(root: &RepoRoot, name: &str, locked: &LockedPack) -> Result<Fetched> {
    let mut files = BTreeMap::new();
    for rel in locked.files.keys() {
        let base = pack::base_path(name, rel);
        let bytes = fs::read(base.to_fs(root)).map_err(|_| {
            Error::Pack(format!(
                "{name}: the base copy {base} is missing; restore it from git, or sync the pack with `pakf sync {name}`"
            ))
        })?;
        files.insert(rel.clone(), bytes);
    }
    Ok(Fetched {
        pack: PackFiles {
            manifest: PackManifest {
                name: name.to_string(),
                version: locked.version.clone(),
                requires_pakf: "0.0.0".into(),
                description: String::new(),
                depends: locked.depends.clone(),
                // A shadowed record carries its pack's declaration too.
                editable: locked
                    .files
                    .iter()
                    .filter(|(_, f)| f.editable)
                    .filter_map(|(rel, _)| rel.strip_prefix("files").map(str::to_string))
                    .collect(),
            },
            files,
            executable: locked
                .files
                .iter()
                .filter(|(_, f)| f.executable)
                .map(|(rel, _)| rel.clone())
                .collect(),
        },
        source: Source::parse(&locked.source),
        git_ref: locked.git_ref.clone(),
        path: locked.path.clone(),
        commit: locked.commit.clone(),
        remap: remap_of(name, &locked.paths)?,
        offline: true,
    })
}

/// `dependents` returns each pack of `lock`, other than `name`, that
/// depends on the pack `name`.
pub(super) fn dependents<'a>(lock: &'a Lock, name: &str) -> Vec<&'a str> {
    lock.packs
        .iter()
        .filter(|(n, p)| *n != name && p.depends.contains_key(name))
        .map(|(n, _)| n.as_str())
        .collect()
}
