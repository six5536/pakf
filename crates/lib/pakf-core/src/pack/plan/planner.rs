//! This module holds the state that the steps of `plan` share, and the
//! steps that decide the action for each file of a pack.

use std::{collections::BTreeSet, fs};

use super::{
    Fetched, FileAction, PackPlan, Request,
    follow::{Followed, follow_acceptance},
    moved::{Held, held},
};
use crate::{
    Error, RepoPath, RepoRoot, Result,
    pack::{
        dest::{Dest, empties, entry_above, read_dest},
        eol::{is_binary, keep_ending, normalise, same_text},
        git,
        lock::{Accepted, LockedFile, LockedPack, base_path, hash_of, holds_conflict},
        meaning::Meaning,
        mode::{self, carries_mode, file_is_executable},
        owner::{Ownership, Shadow, Takeover},
        source::FILES,
    },
};

/// A `Planner` is the state that the steps of `plan` share: the inputs of
/// the sync of one pack, and the plan that the steps build.
pub(super) struct Planner<'a> {
    /// The repository.
    pub(super) root: &'a RepoRoot,
    /// The comparison of the command.
    pub(super) meaning: &'a Meaning,
    /// The pack.
    pub(super) fetched: &'a Fetched,
    /// The options of the sync.
    pub(super) request: &'a Request,
    /// Which destinations another pack owns, or the pack takes over.
    pub(super) own: &'a Ownership,
    /// The pack's name.
    pub(super) name: &'a str,
    /// The pack's entry in the lock, before the sync.
    pub(super) existing: Option<&'a LockedPack>,
    /// The plan so far.
    pub(super) out: PackPlan,
    /// The files that land on existing files that the lock doesn't record.
    pub(super) unrecorded: Vec<String>,
    /// The files that hold a conflict that an earlier sync left.
    pub(super) conflicted: Vec<String>,
    /// The new files that land below an entry that isn't a directory, each
    /// with that entry.
    pub(super) below: Vec<String>,
    /// The binary files that the sync keeps against a change of the pack,
    /// which no merge reaches.
    pub(super) binary_kept: BTreeSet<RepoPath>,
    /// The path in the pack of each file that the remaps of the entry move
    /// and whose deletion moves with it, because no entry stands at either
    /// destination.
    pub(super) carried: BTreeSet<String>,
}

/// A `Target` is one file of the pack, with what the repository and the
/// lock hold of it.
pub(super) struct Target<'a> {
    /// The file's path in the pack.
    rel: &'a str,
    /// The pack's new version of the file.
    upstream: &'a [u8],
    /// The file's destination.
    pub(super) dest: RepoPath,
    /// What stands at the file's destination in the repository.
    current: Dest,
    /// The lock's record of the file.
    old: Option<&'a LockedFile>,
    /// Whether the pack's new version is an executable file of `files/`.
    executable: bool,
    /// The entry above the destination that isn't a directory, which
    /// stands in the way of a new file that the lock doesn't record and
    /// that the pack doesn't take over.
    above: Option<RepoPath>,
}

impl<'a> Planner<'a> {
    /// `fail` returns the error `m` of the pack.
    fn fail(&self, m: impl std::fmt::Display) -> Error {
        super::pack_error(self.name, m)
    }

    /// `target` reads what the repository and the lock hold of the pack's
    /// file `rel`, whose new version is `upstream` and which lands at `dest`.
    /// It reads a file that the same sync deletes as absent, and a
    /// directory too when the sync deletes each file below it, and finds
    /// the entry in the way of a new file that the lock doesn't record.
    pub(super) fn target(
        &mut self,
        rel: &'a str,
        upstream: &'a [u8],
        dest: RepoPath,
    ) -> Target<'a> {
        let gone = |p: &RepoPath| self.request.vacated.contains(p);
        let current = if gone(&dest) {
            Dest::Absent
        } else {
            match read_dest(self.root, &dest) {
                // sokf-implements: CLI-2_AC-65
                Dest::Unreadable(_) if empties(self.root, &dest, &gone) => Dest::Absent,
                d => d,
            }
        };
        // A record at another destination, which the remaps of the entry
        // moved, is no record of the file at its new destination, unless the
        // project deleted the file and no entry stands at the new one, so
        // that the deletion moves with it.
        // sokf-implements: CLI-2_AC-80
        let old = self.existing.and_then(|e| e.files.get(rel)).filter(|f| {
            match held(self.root, self.name, self.fetched, rel, f) {
                Held::Same => true,
                Held::Moved => {
                    let carried = current == Dest::Absent
                        && f.shadowed_by.is_none()
                        // The sync's own deletions vacate the recorded
                        // destination of a moved file, so the working tree
                        // decides whether the project deleted it.
                        && RepoPath::parse(&f.path)
                            .is_some_and(|p| read_dest(self.root, &p) == Dest::Absent);
                    if carried {
                        self.carried.insert(rel.to_string());
                    }
                    carried
                }
                Held::Dropped => false,
            }
        });
        let new = old.is_none_or(|f| f.shadowed_by.is_some())
            && !self.own.takeover.contains_key(&dest)
            && current == Dest::Absent;
        let above = new.then(|| entry_above(self.root, &dest, &gone)).flatten();
        Target {
            rel,
            upstream,
            dest,
            current,
            old,
            executable: rel.starts_with(FILES) && self.fetched.pack.executable.contains(rel),
            above,
        }
    }

    /// `shadowed` plans the pack's file `file`, whose destination the pack
    /// that `shadow` names owns. It records the file and its base copy, and
    /// notes a change of the pack that the owner's version may need.
    // sokf-implements: CLI-2_AC-40
    pub(super) fn shadowed(&mut self, file: &Target, shadow: &Shadow) {
        let (name, dest, old, upstream) = (self.name, &file.dest, file.old, file.upstream);
        let changed = old.is_some_and(|f| self.changes(&f.base, name, file.rel, dest, upstream));
        let wrote = old.is_some_and(|f| f.shadowed_by.is_none());
        // A shadowed record holds no acceptance, so a higher pack that
        // takes the file over drops it.
        // sokf-implements: CLI-2_AC-49
        if let Some(a) = old.and_then(|f| f.accepted.as_ref()).filter(|_| wrote) {
            self.out.notes.push(format!(
                "note: the pack {} now owns {dest}, and the acceptance of its edit is dropped; {}",
                shadow.owner,
                again(shadow.editable && !a.deleted)
            ));
        }
        // sokf-implements: CLI-2_AC-41
        let action = changed.then(|| {
            self.out.notes.push(format!(
                "note: the pack {name} changed {dest}, which the pack {} overrides; check whether its version needs the change too",
                shadow.owner
            ));
            FileAction::Overridden
        });
        if let Some(action) = action {
            self.out.actions.push((dest.clone(), action));
        }
        self.out.entry.files.insert(
            file.rel.to_string(),
            LockedFile {
                path: dest.as_str().to_string(),
                base: hash_of(upstream),
                executable: file.executable,
                shadowed_by: Some(shadow.owner.clone()),
                changed,
                // The owner's record decides whether the file is editable, and
                // this record keeps the pack's own declaration for a sync that
                // reads the pack from the lock.
                editable: self.fetched.pack.manifest.is_editable(file.rel),
                conflicted: false,
                accepted: None,
            },
        );
        self.write_base(file.rel, upstream);
    }

    /// `owned` plans the pack's file `file`, whose destination the pack
    /// owns. It decides the action on the file and on its executable bit,
    /// notes a conflict that an earlier sync left, follows the file's
    /// acceptance, and records the file and its base copy.
    pub(super) fn owned(&mut self, file: Target) -> Result<()> {
        let (name, rel, upstream, executable) =
            (self.name, file.rel, file.upstream, file.executable);
        let dest = &file.dest;
        // A record of a version that another pack shadowed until now is no
        // record of the file, which holds another pack's version or none.
        let recorded = file.old.filter(|f| f.shadowed_by.is_none());
        let takeover = self.own.takeover.get(dest);
        let mode = self.planned_mode(&file, recorded, takeover);
        self.note_conflict(&file, recorded, takeover);
        let editable = self.fetched.pack.manifest.is_editable(rel);
        let was =
            self.editable_acceptance(&file, recorded.and_then(|f| f.accepted.clone()), editable);
        let held = was.as_ref().is_some_and(|a| {
            let bit = mode::carried_bit(rel, &file.current, &dest.to_fs(self.root));
            a.holds(&file.current, bit, dest, self.meaning)
        });
        // An acceptance belongs to the owner's record, so a lower pack that
        // takes a file back from a higher pack, which no longer supplies it
        // or is removed, drops the higher pack's.
        if let Some(t) = takeover.filter(|t| t.accepted) {
            self.out.notes.push(format!(
                "note: the pack {name} now owns {dest}, and the acceptance of its edit is dropped; {}",
                again(editable && !t.deletion)
            ));
        }
        let action = self.content_action(&file, recorded, takeover)?;
        let written = self.out.writes.last().is_some_and(|(p, _)| p == dest);
        // sokf-implements: CLI-2_AC-30
        let action = match mode {
            Some(m) if written || m != file_is_executable(&dest.to_fs(self.root)) => {
                self.out.modes.push((dest.clone(), m));
                action.or(Some(FileAction::Updated))
            }
            _ => action,
        };
        if let Some(action) = action {
            self.out.actions.push((dest.clone(), action));
        }
        let unreadable = matches!(file.current, Dest::Unreadable(_));
        let kept = if unreadable {
            self.kept_base(&file, recorded, takeover)
        } else {
            None
        };
        let after = if written {
            self.out.writes.last().map(|(_, b)| b.clone())
        } else {
            file.current.into_bytes()
        };
        // The sync leaves an entry that isn't a readable regular file
        // untouched, so its acceptance stays as it was, and doesn't hold
        // until a file stands there, whose next sync follows it.
        let accepted = if unreadable {
            was
        } else {
            // Only a file with an acceptance asks the meaning of the change.
            was.and_then(|a| {
                let (holds_new, changed) =
                    self.news(dest, rel, upstream, recorded, after.as_deref());
                follow_acceptance(
                    a,
                    &Followed {
                        dest,
                        action,
                        held,
                        after: after.as_deref(),
                        holds_new,
                        bit: mode,
                        executable,
                        changed,
                        binary_kept: self.binary_kept.contains(dest),
                    },
                    self.meaning,
                    &mut self.out.notes,
                )
            })
        };
        let was_editable = takeover.map_or(recorded.is_some_and(|f| f.editable), |t| t.editable);
        // A deleted file needed an acceptance already, and an entry that
        // isn't a readable regular file can't take one.
        if was_editable && !editable && accepted.is_none() && action != Some(FileAction::Conflicted)
        {
            let differs = after.as_deref().is_some_and(|b| {
                !self.means(dest, b, upstream) || mode.is_some_and(|m| m != executable)
            });
            self.note_no_longer_editable(dest, differs, takeover.is_some());
        }
        let (base, recorded_bit) = kept.unwrap_or_else(|| {
            self.write_base(rel, upstream);
            (hash_of(upstream), executable)
        });
        self.out.entry.files.insert(
            rel.to_string(),
            LockedFile {
                path: dest.as_str().to_string(),
                base,
                executable: recorded_bit,
                shadowed_by: None,
                changed: false,
                editable,
                conflicted: action == Some(FileAction::Conflicted),
                accepted,
            },
        );
        Ok(())
    }

    /// `note_conflict` notes the pack's file `file` where it still holds a
    /// conflict that an earlier sync left, as the record of the pack that
    /// owned it until this sync, `recorded` or `takeover`, reads one.
    // sokf-implements: CLI-1_AC-47
    fn note_conflict(
        &mut self,
        file: &Target,
        recorded: Option<&LockedFile>,
        takeover: Option<&Takeover>,
    ) {
        let left = match (recorded, takeover) {
            (Some(f), _) => f.left_conflict().then(|| base_path(self.name, file.rel)),
            (None, Some(t)) => t.conflicted.then(|| base_path(&t.from, &t.rel)),
            (None, None) => return,
        };
        if file
            .current
            .bytes()
            .is_some_and(|c| holds_conflict(self.root, &file.dest, c, left.as_ref()))
        {
            self.conflicted.push(file.dest.to_string());
        }
    }

    /// `editable_acceptance` returns the acceptance `was` of the pack's file
    /// `file`, or none where the pack declares the file editable, as
    /// `editable` says, and `was` accepts its edit, or a deletion while a
    /// file stands there again, because the edit then needs no acceptance,
    /// with a note. An accepted deletion of an absent file stays, because a
    /// deletion still needs one.
    // sokf-implements: CLI-2_AC-82
    fn editable_acceptance(
        &mut self,
        file: &Target,
        was: Option<Accepted>,
        editable: bool,
    ) -> Option<Accepted> {
        let dest = &file.dest;
        let stands = matches!(file.current, Dest::File(_));
        match was {
            Some(a) if editable && (!a.deleted || stands) => {
                self.out.notes.push(format!(
                    "note: {dest} is an editable file of the pack {} and needs no acceptance; its acceptance is dropped",
                    self.name
                ));
                None
            }
            was => was,
        }
    }

    /// `note_no_longer_editable` notes that the file `dest`, which the lock
    /// recorded as editable and which its owner no longer declares so, needs
    /// an acceptance of its edit, where a file stands there that `differs`
    /// from its base. Where the pack takes the file over from another pack,
    /// as `takeover` says, the note names the pack that now owns it.
    // sokf-implements: CLI-2_AC-83
    fn note_no_longer_editable(&mut self, dest: &RepoPath, differs: bool, takeover: bool) {
        if !differs {
            return;
        }
        let why = if takeover {
            format!(
                "the pack {} now owns {dest} and doesn't declare it editable",
                self.name
            )
        } else {
            format!("{dest} is no longer an editable file of its pack")
        };
        self.out.notes.push(format!(
            "note: {why}, and its edit now needs an acceptance; accept it with `pakf accept` if the project keeps the edit"
        ));
    }

    /// `kept_base` returns the base and the executable bit that the
    /// pack's record of its file `file`, `recorded`, keeps while an entry
    /// that isn't a readable regular file stands at its destination and the
    /// pack changed its version, so that the pack's change merges once a
    /// regular file stands there. The base copy then stays too. It returns
    /// none for a file that the pack takes over as `takeover`, whose record
    /// takes the pack's version as its base, as a deleted file's does,
    /// because a base copy of the pack holds only the pack's own version.
    // sokf-implements: CLI-2_AC-62
    // sokf-implements: CLI-2_AC-84
    fn kept_base(
        &self,
        file: &Target,
        recorded: Option<&LockedFile>,
        takeover: Option<&Takeover>,
    ) -> Option<(String, bool)> {
        if takeover.is_some() {
            return None;
        }
        recorded
            .filter(|f| {
                self.changes(&f.base, self.name, file.rel, &file.dest, file.upstream)
                    || bit_changed(file, f.executable)
            })
            .map(|f| (f.base.clone(), f.executable))
    }

    /// `planned_mode` returns the executable bit that the pack's file `file`
    /// of `files/` has after the sync, or `None` for another file or when no
    /// file is there after it. A file that the pack takes over as `takeover`
    /// takes the pack's bit while its bit equals the old owner's, and keeps
    /// its own bit otherwise.
    // sokf-implements: CLI-1_AC-26
    fn planned_mode(
        &self,
        file: &Target,
        recorded: Option<&LockedFile>,
        takeover: Option<&Takeover>,
    ) -> Option<bool> {
        // A platform without the bit neither compares nor sets it, and the
        // sync sets no bit through an entry that isn't a readable regular
        // file, such as a symbolic link.
        // Nor does it set one on a new file that it doesn't write.
        if matches!(file.current, Dest::Unreadable(_)) || file.above.is_some() {
            return None;
        }
        carries_mode(file.rel)
            .then(|| match takeover {
                Some(t) if file.current.bytes().is_some() => {
                    let bit = file_is_executable(&file.dest.to_fs(self.root));
                    Some(if bit == t.executable {
                        file.executable
                    } else {
                        bit
                    })
                }
                _ => file_mode(
                    recorded,
                    file.current.bytes().is_some(),
                    file.executable,
                    &file.dest,
                    self.root,
                ),
            })
            .flatten()
    }

    /// `content_action` decides what the sync does to the content of the
    /// pack's file `file`, which the lock records as `recorded`, or which
    /// the pack takes over as `takeover`, and pushes the file's write. It
    /// lists a file that lands on an existing file that the lock doesn't
    /// record, and that differs from the pack's version, unless
    /// `--keep-existing` keeps it as an edit of the pack's version.
    // sokf-implements: CLI-2_AC-13
    fn content_action(
        &mut self,
        file: &Target,
        recorded: Option<&LockedFile>,
        takeover: Option<&Takeover>,
    ) -> Result<Option<FileAction>> {
        let (name, rel, upstream, dest) = (self.name, file.rel, file.upstream, &file.dest);
        let current = match &file.current {
            Dest::Unreadable(_) => return Ok(self.unreadable_action(file, recorded, takeover)),
            Dest::Absent => None,
            Dest::File(b) => Some(b),
        };
        Ok(match (recorded, current) {
            // A change of owner reads the four cases against the old owner's
            // base copy.
            // sokf-implements: CLI-2_AC-42
            (_, Some(cur)) if let Some(t) = takeover => {
                if self.holds_old(&t.base, &t.from, &t.rel, cur) {
                    return Ok(self.update_unedited(dest, cur, upstream));
                }
                let edited = self.edited(&t.base, &t.from, &t.rel, dest, cur);
                let changes = self.changes(&t.base, &t.from, &t.rel, dest, upstream);
                if !edited && changes {
                    self.update_unedited(dest, cur, upstream)
                } else if !changes || self.means(dest, cur, upstream) {
                    // A file that moves unchanged, or only re-laid, keeps
                    // its edit, as a merge would, with no write.
                    None
                } else {
                    let base_copy = base_path(&t.from, &t.rel);
                    let base = fs::read(base_copy.to_fs(self.root)).map_err(|_| {
                        self.fail(format_args!(
                            "{dest}: the base copy {base_copy} of the pack {} is missing; restore the base copy from git",
                            t.from
                        ))
                    })?;
                    Some(self.merge_into(rel, dest, cur, &base, upstream, true)?)
                }
            }
            // A deletion is an edit, and it outlives a change of owner too.
            // sokf-implements: CLI-2_AC-18
            (Some(_), None) => Some(FileAction::Deleted),
            (None, None) if takeover.is_some() => Some(FileAction::Deleted),
            // A new file that can't be written below an entry is an existing
            // file that differs from the pack's version, which reads as
            // deleted once it is kept.
            // sokf-implements: CLI-2_AC-63
            // sokf-implements: CLI-2_AC-64
            (None, None) if let Some(above) = &file.above => {
                if !self.request.keep_existing {
                    self.below.push(format!("{dest} below {above}"));
                }
                Some(FileAction::KeptExisting)
            }
            (None, None) => {
                self.out.writes.push((dest.clone(), upstream.to_vec()));
                Some(FileAction::Added)
            }
            // A file that already holds the pack's version, such as the
            // copy that `pakf patch` sent to the pack, or that means it in
            // the project's layout, is no edit.
            (None, Some(cur)) => {
                let same = self.means(dest, cur, upstream)
                    && (!carries_mode(rel)
                        || file_is_executable(&dest.to_fs(self.root)) == file.executable);
                if !same && !self.request.keep_existing {
                    self.unrecorded.push(dest.to_string());
                }
                (!same).then_some(FileAction::KeptExisting)
            }
            // The four cases of a sync: the copy against its base, and the
            // pack's new version against it (the decision "The four cases of
            // a sync" of design-pack). A file and the pack's version that
            // differ only in their line endings hold the same text (the
            // decision "The committed bytes" of design-pack).
            // sokf-implements: CLI-2_AC-84
            // sokf-implements: CLI-2_AC-85
            // sokf-implements: CLI-2_AC-86
            (Some(f), Some(cur)) => {
                // A copy that holds its old base takes the pack's new bytes,
                // also where they only re-lay it, so that it holds its base
                // on every machine, before any comparison parses a text.
                if self.holds_old(&f.base, name, rel, cur) {
                    return Ok(self.update_unedited(dest, cur, upstream));
                }
                let edited = self.edited(&f.base, name, rel, dest, cur);
                let changes = self.changes(&f.base, name, rel, dest, upstream);
                if !edited && changes {
                    self.update_unedited(dest, cur, upstream)
                } else if !changes || self.means(dest, cur, upstream) {
                    None
                } else {
                    // sokf-implements: CLI-2_AC-19
                    let base_copy = base_path(name, rel);
                    let base = fs::read(base_copy.to_fs(self.root)).map_err(|_| {
                        self.fail(format_args!(
                            "{dest}: the base copy {base_copy} is missing; restore it from git, or run `pakf remove {name}` and then `pakf add --keep-existing`, which records the file as an edit of the pack's new version"
                        ))
                    })?;
                    Some(self.merge_into(rel, dest, cur, &base, upstream, false)?)
                }
            }
        })
    }

    /// `unreadable_action` decides what the sync does to an entry that isn't
    /// a readable regular file at the destination of the pack's file
    /// `file`. It leaves the entry untouched, as an edit that no merge
    /// reaches, and reports it as kept when the pack's version changed
    /// against the base or the bit of the record `recorded`, or of the
    /// takeover `takeover`. Without either, the entry is an existing file that
    /// differs from the pack's version, as `content_action` says.
    // sokf-implements: CLI-2_AC-60
    // sokf-implements: CLI-2_AC-61
    fn unreadable_action(
        &mut self,
        file: &Target,
        recorded: Option<&LockedFile>,
        takeover: Option<&Takeover>,
    ) -> Option<FileAction> {
        let (dest, upstream) = (&file.dest, file.upstream);
        match (takeover, recorded) {
            (Some(t), _) => (self.changes(&t.base, &t.from, &t.rel, dest, upstream)
                || bit_changed(file, t.executable))
            .then_some(FileAction::Kept),
            (None, Some(f)) => (self.changes(&f.base, self.name, file.rel, dest, upstream)
                || bit_changed(file, f.executable))
            .then_some(FileAction::Kept),
            (None, None) => {
                if !self.request.keep_existing {
                    self.unrecorded.push(file.dest.to_string());
                }
                Some(FileAction::KeptExisting)
            }
        }
    }

    /// `update_unedited` replaces the unedited file `cur` at `dest` with the
    /// pack's version `upstream`, in the file's line endings, and returns
    /// `None` when the file already holds that version. A binary file takes
    /// the pack's bytes unchanged, as `keep_ending` says.
    // sokf-implements: CLI-2_AC-26
    fn update_unedited(
        &mut self,
        dest: &RepoPath,
        cur: &[u8],
        upstream: &[u8],
    ) -> Option<FileAction> {
        if same_text(cur, upstream) {
            None
        } else {
            self.out
                .writes
                .push((dest.clone(), keep_ending(cur, upstream.to_vec())));
            Some(FileAction::Updated)
        }
    }

    /// `merge_into` merges the pack's version `upstream` of its file `rel`
    /// into the edited file `cur` at `dest` against the base `base`, and
    /// pushes the merged file in the file's line endings. It returns
    /// `FileAction::Merged` or `FileAction::Conflicted`, or the error of
    /// `git merge-file`. It merges no binary file: where the file, its base,
    /// or the pack's version is binary, it keeps the file, whose record
    /// takes the pack's version as its base, and gives the file's line the
    /// reason, with the base copy that holds the pack's version. The reason
    /// names a change of the file's owner where `takeover` says so.
    // sokf-implements: CLI-2_AC-27
    fn merge_into(
        &mut self,
        rel: &str,
        dest: &RepoPath,
        cur: &[u8],
        base: &[u8],
        upstream: &[u8],
        takeover: bool,
    ) -> Result<FileAction> {
        if is_binary(cur) || is_binary(base) || is_binary(upstream) {
            let changed = if takeover {
                "the project changed the binary file, and its new owner's version differs"
            } else {
                "the project and the pack both changed the binary file"
            };
            self.out.reasons.insert(
                dest.clone(),
                format!(
                    "{changed}; copy {} over it to take the pack's version",
                    base_path(self.name, rel)
                ),
            );
            self.binary_kept.insert(dest.clone());
            return Ok(FileAction::Kept);
        }
        let merge = git::merge_file(
            &normalise(cur),
            &normalise(base),
            &normalise(upstream),
            dest.as_str(),
        )?;
        self.out
            .writes
            .push((dest.clone(), keep_ending(cur, merge.text)));
        Ok(if merge.conflicted {
            FileAction::Conflicted
        } else {
            FileAction::Merged
        })
    }

    /// `write_base` writes the base copy of the pack's file `rel`, unless
    /// the copy already holds the pack's version `upstream`.
    // sokf-implements: CLI-2_AC-6
    fn write_base(&mut self, rel: &str, upstream: &[u8]) {
        let base = base_path(self.name, rel);
        if !fs::read(base.to_fs(self.root)).is_ok_and(|b| same_text(&b, upstream)) {
            self.out.writes.push((base, upstream.to_vec()));
        }
    }

    /// `remove_dropped` deletes the base copy of each file that the lock
    /// records and the pack no longer holds, and deletes the file too unless
    /// the user edited it, as `dropped` says. A file that the remaps of the
    /// entry moved to another destination goes from its recorded
    /// destination in the same way, and keeps its base copy, which its new
    /// record holds, with a note for an acceptance that the move drops. A
    /// file whose destination changes only in the case of its letters stays,
    /// because a filesystem that ignores the case holds one file at both.
    pub(super) fn remove_dropped(&mut self) {
        let Some(e) = self.existing else {
            return;
        };
        for (rel, f) in &e.files {
            match held(self.root, self.name, self.fetched, rel, f) {
                Held::Same => continue,
                Held::Moved => {
                    let now = self.out.entry.files.get(rel).map(|n| n.path.as_str());
                    if f.accepted.is_some()
                        && f.shadowed_by.is_none()
                        && !self.carried.contains(rel)
                    {
                        self.out.notes.push(format!(
                            "note: the paths of the entry move {} to {}, and the acceptance of its edit is dropped; accept the file again at its new path if the project keeps the edit",
                            f.path,
                            now.unwrap_or_default()
                        ));
                    }
                }
                Held::Dropped => self.out.deletes.push(base_path(self.name, rel)),
            }
            if let Some((dest, action)) = super::dropped::dropped(
                self.root,
                self.meaning,
                self.name,
                rel,
                f,
                &self.request.owned_dests,
            ) {
                if action == FileAction::Removed {
                    self.out.deletes.push(dest.clone());
                } else if f.still_conflicted(self.root, self.name, rel) {
                    // sokf-implements: CLI-1_AC-48
                    self.out.still_conflicted.push(dest.clone());
                }
                self.out.actions.push((dest, action));
            }
        }
    }
}

/// `again` returns the end of the note of an acceptance that a change of
/// the file's owner drops: that the new owner's declaration of the file as
/// editable makes the acceptance needless, as `editable` says, or else to
/// accept the file again.
fn again(editable: bool) -> &'static str {
    if editable {
        "the file is editable and needs no acceptance"
    } else {
        "accept it again if the project keeps the edit"
    }
}

/// `bit_changed` returns whether the pack's version of `file` changes the
/// executable bit `recorded`, which only a file that carries the bit does.
fn bit_changed(file: &Target, recorded: bool) -> bool {
    carries_mode(file.rel) && recorded != file.executable
}

/// `file_mode` returns the executable bit that the file `dest` of `files/`
/// has after the sync, or `None` when no file is there after it. A new file
/// takes the pack's bit, `executable`. A file whose bit the lock records
/// takes the pack's bit while its bit equals the recorded one, and keeps
/// its own bit otherwise, because a change of the bit is an edit. A file
/// that the lock doesn't record keeps its bit.
fn file_mode(
    recorded: Option<&LockedFile>,
    exists: bool,
    executable: bool,
    dest: &RepoPath,
    root: &RepoRoot,
) -> Option<bool> {
    let current = file_is_executable(&dest.to_fs(root));
    match (recorded, exists) {
        (Some(_), false) => None,
        (None, false) => Some(executable),
        (Some(f), true) if f.executable == current => Some(executable),
        (_, true) => Some(current),
    }
}
