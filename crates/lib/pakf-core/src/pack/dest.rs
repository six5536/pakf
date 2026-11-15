//! This module reads what stands at the destination of a file of a pack,
//! for every command of the packs that asks, so that sync, `pakf accept`,
//! `pakf status`, `pakf ls`, `pakf diff`, `pakf patch`, and the harness's
//! read of a copy of a pack's `instructions.md` read a destination by one
//! rule (the decision "One reading of a destination" of design-pack).
//! It also finds the entry above a destination that stands in the way of a
//! write.

use std::{fs, io, path::Path};

use crate::{RepoPath, RepoRoot};

/// A `Dest` is what stands at the destination of a file of a pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dest {
    /// No entry stands at the path, or a file stands at a directory above
    /// it.
    Absent,
    /// An entry that isn't a readable regular file stands at the path, such
    /// as a directory, a symbolic link, a FIFO, or a file without read
    /// permission, with why it doesn't read.
    Unreadable(String),
    /// A regular file with these bytes stands at the path.
    File(Vec<u8>),
}

impl Dest {
    /// `bytes` returns the bytes of a regular file, and none otherwise.
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Dest::File(b) => Some(b),
            _ => None,
        }
    }

    /// `into_bytes` returns the bytes of a regular file, and none
    /// otherwise.
    pub fn into_bytes(self) -> Option<Vec<u8>> {
        match self {
            Dest::File(b) => Some(b),
            _ => None,
        }
    }
}

/// `is_absent` returns whether no entry stands at `path`, or a file stands
/// at a directory above it. Any other failure to read the entry's type
/// leaves the path present, because only those two say that nothing stands
/// there.
// sokf-implements: CLI-2_AC-57
pub fn is_absent(path: &Path) -> bool {
    matches!(fs::symlink_metadata(path), Err(e) if absence(&e))
}

/// `absence` returns whether the error `e` of a read of a path says that
/// nothing stands at it.
fn absence(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    )
}

/// `read_dest` reads what stands at `path`. It reads the bytes of a regular
/// file only, and follows no symbolic link at the path, so that a FIFO
/// blocks no command and a link reads as an edit whatever it points at.
// sokf-implements: CLI-2_AC-57
// sokf-implements: CLI-2_AC-58
// sokf-implements: CLI-2_AC-59
pub fn read_dest(path: &Path) -> Dest {
    let kind = match fs::symlink_metadata(path) {
        Ok(meta) => meta.file_type(),
        Err(e) if absence(&e) => return Dest::Absent,
        Err(e) => return Dest::Unreadable(e.to_string()),
    };
    if kind.is_symlink() {
        return Dest::Unreadable("a symbolic link stands at its path".into());
    }
    if kind.is_dir() {
        return Dest::Unreadable("a directory stands at its path".into());
    }
    if !kind.is_file() {
        return Dest::Unreadable("an entry that isn't a regular file stands at its path".into());
    }
    match fs::read(path) {
        Ok(bytes) => Dest::File(bytes),
        Err(e) => Dest::Unreadable(e.to_string()),
    }
}

/// `entry_above` returns the highest directory above `path` at which an
/// entry stands that is neither a directory nor a link to one, such as a
/// file, so that no file can be written at `path`. An entry for which
/// `gone` holds reads as absent, because the command deletes it before it
/// writes. It returns none when nothing stands in the way, or when the
/// entry's type doesn't read for another reason, which no command
/// foresees.
// sokf-implements: CLI-2_AC-63
// sokf-implements: CLI-2_AC-66
pub fn entry_above(
    root: &RepoRoot,
    path: &RepoPath,
    gone: &impl Fn(&RepoPath) -> bool,
) -> Option<RepoPath> {
    let mut dirs: Vec<RepoPath> = std::iter::successors(path.parent(), RepoPath::parent)
        .take_while(|d| *d != RepoPath::root())
        .collect();
    dirs.reverse();
    for dir in dirs {
        if gone(&dir) {
            return None;
        }
        let fs_dir = dir.to_fs(root);
        let directory = match fs::symlink_metadata(&fs_dir) {
            Ok(meta) if meta.file_type().is_symlink() => {
                fs::metadata(&fs_dir).is_ok_and(|m| m.is_dir())
            }
            Ok(meta) => meta.is_dir(),
            Err(_) => return None,
        };
        if !directory {
            return Some(dir);
        }
    }
    None
}

/// `empties` returns whether a directory, and no link, stands at `path`
/// that the deletions of a command leave absent: each entry below it, at
/// any depth, is a file for which `gone` holds or a directory that holds an
/// entry, because a deletion removes each directory that it leaves empty,
/// and no other.
// sokf-implements: CLI-2_AC-65
pub fn empties(root: &RepoRoot, path: &RepoPath, gone: &impl Fn(&RepoPath) -> bool) -> bool {
    let fs_path = path.to_fs(root);
    if !fs::symlink_metadata(&fs_path).is_ok_and(|m| m.is_dir()) {
        return false;
    }
    let Ok(entries) = fs::read_dir(&fs_path) else {
        return false;
    };
    let mut any = false;
    for entry in entries {
        let Some(child) = entry
            .ok()
            .and_then(|e| e.file_name().to_str().and_then(|n| path.join(n)))
        else {
            return false;
        };
        if !(gone(&child) || empties(root, &child, gone)) {
            return false;
        }
        any = true;
    }
    any
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_empties_when_each_file_below_it_goes() {
        let dir = super::super::TempDir::new().unwrap();
        let root = RepoRoot::new(dir.path().to_path_buf());
        let at = |s: &str| RepoPath::parse(s).unwrap();
        fs::create_dir_all(dir.path().join("a/b")).unwrap();
        fs::write(dir.path().join("a/b/c"), "x").unwrap();
        fs::write(dir.path().join("a/d"), "x").unwrap();
        let both = |p: &RepoPath| *p == at("/a/b/c") || *p == at("/a/d");
        assert!(empties(&root, &at("/a"), &both));
        assert!(!empties(&root, &at("/a"), &|p: &RepoPath| *p == at("/a/d")));
        // An empty directory below stays, so the directory does too.
        fs::create_dir(dir.path().join("a/e")).unwrap();
        assert!(!empties(&root, &at("/a"), &both));
        assert!(!empties(&root, &at("/a/d"), &both));
        assert!(!empties(&root, &at("/missing"), &both));
    }

    #[test]
    fn an_entry_above_is_the_highest_that_isnt_a_directory() {
        let dir = super::super::TempDir::new().unwrap();
        let root = RepoRoot::new(dir.path().to_path_buf());
        let at = |s: &str| RepoPath::parse(s).unwrap();
        fs::create_dir_all(dir.path().join("a/b")).unwrap();
        fs::write(dir.path().join("a/b/c"), "x").unwrap();
        fs::write(dir.path().join("top"), "x").unwrap();
        assert_eq!(
            entry_above(&root, &at("/a/b/c/d/e"), &|_| false),
            Some(at("/a/b/c"))
        );
        assert_eq!(
            entry_above(&root, &at("/top/x"), &|_| false),
            Some(at("/top"))
        );
        assert_eq!(entry_above(&root, &at("/a/b/new"), &|_| false), None);
        assert_eq!(entry_above(&root, &at("/missing/x/y"), &|_| false), None);
        assert_eq!(entry_above(&root, &at("/top"), &|_| false), None);
        // An entry that the command deletes first is out of the way.
        assert_eq!(
            entry_above(&root, &at("/top/x"), &|p: &RepoPath| *p == at("/top")),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_above_is_in_the_way_unless_it_leads_to_a_directory() {
        use std::os::unix::fs::symlink;
        let dir = super::super::TempDir::new().unwrap();
        let root = RepoRoot::new(dir.path().to_path_buf());
        let at = |s: &str| RepoPath::parse(s).unwrap();
        fs::create_dir(dir.path().join("real")).unwrap();
        fs::write(dir.path().join("file"), "x").unwrap();
        symlink("real", dir.path().join("to-dir")).unwrap();
        symlink("file", dir.path().join("to-file")).unwrap();
        symlink("nowhere", dir.path().join("dangling")).unwrap();
        assert_eq!(entry_above(&root, &at("/to-dir/x"), &|_| false), None);
        assert_eq!(
            entry_above(&root, &at("/to-file/x"), &|_| false),
            Some(at("/to-file"))
        );
        assert_eq!(
            entry_above(&root, &at("/dangling/x"), &|_| false),
            Some(at("/dangling"))
        );
    }

    #[test]
    fn a_path_reads_by_what_stands_at_it() {
        let dir = super::super::TempDir::new().unwrap();
        let file = dir.path().join("file");
        fs::write(&file, "text\n").unwrap();
        assert_eq!(read_dest(&file), Dest::File(b"text\n".to_vec()));
        assert!(!is_absent(&file));
        let missing = dir.path().join("missing");
        assert_eq!(read_dest(&missing), Dest::Absent);
        assert!(is_absent(&missing));
        // A file at a directory above the path stands for no entry at it.
        let below = file.join("below");
        assert_eq!(read_dest(&below), Dest::Absent);
        assert!(is_absent(&below));
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        assert!(matches!(read_dest(&sub), Dest::Unreadable(_)));
        assert!(!is_absent(&sub));
    }

    #[cfg(unix)]
    #[test]
    fn a_link_reads_as_unreadable_whatever_it_points_at() {
        use std::os::unix::fs::symlink;
        let dir = super::super::TempDir::new().unwrap();
        let file = dir.path().join("file");
        fs::write(&file, "text\n").unwrap();
        for (name, target) in [
            ("to-file", file.as_path()),
            ("dangling", Path::new("nowhere")),
        ] {
            let link = dir.path().join(name);
            symlink(target, &link).unwrap();
            assert!(matches!(read_dest(&link), Dest::Unreadable(_)), "{name}");
            assert!(!is_absent(&link), "{name}");
        }
    }
}
