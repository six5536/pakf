//! This module decides which pack owns each destination, given the order
//! of the packs. The highest pack that supplies a destination owns it.
//! Every other pack's version is shadowed: sync keeps its base copy and
//! doesn't write it. A destination whose owner changes passes to its new
//! owner by a merge (the decision "The rules of owning a file" of
//! design-pack).

use std::collections::BTreeMap;

use super::lock::Lock;
use crate::RepoPath;

/// A `Supply` is what one pack supplies: each destination.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Supply {
    /// Each destination.
    pub files: Vec<RepoPath>,
    /// Each destination of a file that the pack declares editable.
    pub editable: Vec<RepoPath>,
}

/// A `Shadow` says that another pack owns a destination of a pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shadow {
    /// The pack that owns it.
    pub owner: String,
    /// Whether the pack that owns it declares the file editable.
    pub editable: bool,
}

/// A `Takeover` says that a pack owns a destination that another pack
/// owned until this sync, so the file holds that pack's version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Takeover {
    /// The pack that owned the destination.
    pub from: String,
    /// The path of the file in that pack.
    pub rel: String,
    /// The hash of that pack's base.
    pub base: String,
    /// Whether that pack's version was executable.
    pub executable: bool,
    /// Whether that pack declared the file editable.
    pub editable: bool,
    /// Whether that pack's last sync left a conflict in the file, as
    /// `LockedFile::left_conflict` says.
    pub conflicted: bool,
    /// Whether that pack's record holds an acceptance that the sync drops
    /// and no longer supplies the destination, so that the note of the drop
    /// falls to the pack that takes the file over.
    pub accepted: bool,
    /// Whether that acceptance accepts a deletion, which still needs an
    /// acceptance where the file is editable.
    pub deletion: bool,
}

/// An `Ownership` is what one pack owns of what it supplies.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ownership {
    /// Each destination of the pack that another pack owns.
    pub shadowed: BTreeMap<RepoPath, Shadow>,
    /// Each destination that the pack takes over from another pack.
    pub takeover: BTreeMap<RepoPath, Takeover>,
}

/// `owners` returns the ownership of each pack of `supplies`, given
/// `order`, the packs from the lowest to the highest, and `old`, the lock
/// before the sync, which says which pack's version each file holds.
// sokf-implements: CLI-2_AC-38
pub fn owners(
    order: &[String],
    supplies: &BTreeMap<String, Supply>,
    old: &Lock,
) -> BTreeMap<String, Ownership> {
    let mut dest_owner: BTreeMap<&RepoPath, &str> = BTreeMap::new();
    for name in order {
        let Some(supply) = supplies.get(name) else {
            continue;
        };
        for dest in &supply.files {
            dest_owner.insert(dest, name);
        }
    }
    let mut out = BTreeMap::new();
    for (name, supply) in supplies {
        let mut own = Ownership::default();
        for dest in &supply.files {
            let owner = dest_owner.get(dest).copied().unwrap_or(name);
            if owner != name {
                own.shadowed.insert(
                    dest.clone(),
                    Shadow {
                        owner: owner.to_string(),
                        editable: supplies
                            .get(owner)
                            .is_some_and(|s| s.editable.contains(dest)),
                    },
                );
            } else if let Some((from, rel)) = old.owner_of(dest.as_str())
                && from != name
            {
                let f = &old.packs[from].files[rel];
                // A pack that still supplies the destination is shadowed by
                // this sync, and its own plan gives the note.
                let supplied = supplies.get(from).is_some_and(|s| s.files.contains(dest));
                own.takeover.insert(
                    dest.clone(),
                    Takeover {
                        from: from.to_string(),
                        rel: rel.to_string(),
                        base: f.base.clone(),
                        executable: f.executable,
                        editable: f.editable,
                        conflicted: f.left_conflict(),
                        accepted: f.accepted.is_some() && !supplied,
                        deletion: f.accepted.as_ref().is_some_and(|a| a.deleted),
                    },
                );
            }
        }
        out.insert(name.clone(), own);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::{LockedFile, LockedPack};

    fn path(p: &str) -> RepoPath {
        RepoPath::parse(p).unwrap()
    }

    fn supply(files: &[&str]) -> Supply {
        Supply {
            files: files.iter().map(|d| path(d)).collect(),
            editable: Vec::new(),
        }
    }

    fn lock(owner: &str, dest: &str) -> Lock {
        let mut lock = Lock::default();
        let mut pack = LockedPack::default();
        pack.files.insert(
            "files/rustfmt.toml".into(),
            LockedFile {
                path: dest.into(),
                base: crate::pack::hash_of(b"x"),
                executable: false,
                shadowed_by: None,
                changed: false,
                editable: false,
                conflicted: false,
                accepted: None,
            },
        );
        lock.packs.insert(owner.into(), pack);
        lock
    }

    // sokf-tests: CLI-2_AC-38
    #[test]
    fn the_highest_pack_owns_a_destination() {
        let order = ["base".to_string(), "rust".to_string()];
        let supplies = BTreeMap::from([
            (
                "base".to_string(),
                supply(&["/k/schemas/issue.md", "/rustfmt.toml", "/deny.toml"]),
            ),
            (
                "rust".to_string(),
                supply(&["/k/schemas/issue-strict.md", "/rustfmt.toml"]),
            ),
        ]);
        let old = lock("base", "/rustfmt.toml");
        let own = owners(&order, &supplies, &old);
        let base = &own["base"];
        assert_eq!(
            base.shadowed[&path("/rustfmt.toml")],
            Shadow {
                owner: "rust".into(),
                editable: false,
            }
        );
        // Two schemas of one type at two paths are two files.
        assert!(!base.shadowed.contains_key(&path("/k/schemas/issue.md")));
        assert!(!base.shadowed.contains_key(&path("/deny.toml")));
        let rust = &own["rust"];
        assert!(rust.shadowed.is_empty());
        // `rust` takes over the file that `base` wrote before.
        assert_eq!(rust.takeover[&path("/rustfmt.toml")].from, "base");
        assert!(
            !rust
                .takeover
                .contains_key(&path("/k/schemas/issue-strict.md"))
        );
    }
}
