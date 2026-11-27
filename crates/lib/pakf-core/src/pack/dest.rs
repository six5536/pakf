//! This module reads what stands at the destination of a file of a pack,
//! for every command of the packs that asks, so that sync, `pakf accept`,
//! `pakf status`, `pakf ls`, `pakf diff`, `pakf patch`, and the harness's
//! read of a copy of a pack's `instructions.md` read a destination by one
//! rule (the decision "One reading of a destination" of design-pack).
//! It also finds the entry above a destination that stands in the way of a
//! write, and the symbolic link on the way to a path that leads out of the
//! repository.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::Path,
};

use crate::{Error, RepoPath, RepoRoot, Result};

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

/// `read_dest` reads what stands at the repository file `path`. It reads
/// the bytes of a regular file only, and follows no symbolic link at the
/// path, so that a FIFO blocks no command and a link reads as an edit
/// whatever it points at. A path below a link that leads out of the
/// repository, as `link_out` finds it, reads as an entry that isn't a
/// readable regular file, whatever stands at its target, so that no command
/// reads a file of the machine through it.
// sokf-implements: CLI-2_AC-57
// sokf-implements: CLI-2_AC-58
// sokf-implements: CLI-2_AC-59
// sokf-implements: CLI-1_AC-46
pub fn read_dest(root: &RepoRoot, path: &RepoPath) -> Dest {
    if let Some(out) = link_out(root, path, false) {
        return Dest::Unreadable(out.why(path));
    }
    read_entry(&path.to_fs(root))
}

/// `read_entry` reads what stands at `path` on disk, as `read_dest` says,
/// without a look at the links above it.
fn read_entry(path: &Path) -> Dest {
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

/// `Leads` says where a symbolic link that leads out of the repository
/// leads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Leads {
    /// Outside the repository root.
    Outside,
    /// Into a directory `.git`, among git's own files.
    Git,
    /// To no entry.
    Nowhere,
}

/// A `LinkOut` is a symbolic link on the way to a path that leads out of the
/// repository.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct LinkOut {
    /// The link, from the repository root.
    pub(crate) link: RepoPath,
    /// Where it leads.
    pub(crate) leads: Leads,
}

impl LinkOut {
    /// `to` says where the link leads.
    fn to(&self) -> &'static str {
        match self.leads {
            Leads::Outside => "leads outside the repository",
            Leads::Git => "leads into git's own files",
            Leads::Nowhere => "leads to no file",
        }
    }

    /// `shown` names the link, for a file at `path` or below it, and where it
    /// leads, as a message prints it after a verb such as "through".
    fn shown(&self, path: &RepoPath) -> String {
        if self.link == *path {
            format!("a symbolic link that {}", self.to())
        } else {
            format!("the symbolic link {}, which {}", self.link, self.to())
        }
    }

    /// `why` says why the file `path`, at the link or below it, can't be
    /// read.
    pub(crate) fn why(&self, path: &RepoPath) -> String {
        if self.link == *path {
            format!("it is {}", self.shown(path))
        } else {
            format!("it lies below {}", self.shown(path))
        }
    }
}

/// `link_out` returns the first symbolic link on the way to `path`, at a
/// directory above it or, where `at_path` says, at `path` itself, whose
/// canonical target lies outside the canonical repository root or in a
/// directory `.git`, or, at `path` itself, that leads to no entry, because
/// a write follows a link at its path and creates a missing target. It
/// returns none where no such link stands on the way, and stops at an entry
/// that is absent or isn't a directory, below which nothing stands. A link
/// whose target lies elsewhere in the root counts as the directory that it
/// leads to. It reads the disk and not git, so a link that no commit holds
/// counts too.
// sokf-implements: CLI-2_AC-89
// sokf-implements: CLI-2_AC-90
pub(crate) fn link_out(root: &RepoRoot, path: &RepoPath, at_path: bool) -> Option<LinkOut> {
    let mut way: Vec<RepoPath> = std::iter::successors(path.parent(), RepoPath::parent)
        .take_while(|d| *d != RepoPath::root())
        .collect();
    way.reverse();
    if at_path && *path != RepoPath::root() {
        way.push(path.clone());
    }
    let mut base = None;
    for at in way {
        let fs_at = at.to_fs(root);
        let meta = fs::symlink_metadata(&fs_at).ok()?;
        if !meta.file_type().is_symlink() {
            if meta.is_dir() {
                continue;
            }
            return None;
        }
        // Both paths take one form, the verbatim one on Windows, so that a
        // long path compares with a short one.
        let base = base.get_or_insert_with(|| {
            fs::canonicalize(root.as_path()).unwrap_or_else(|_| root.as_path().to_path_buf())
        });
        let leads = match fs::canonicalize(&fs_at) {
            // A link above the path that leads to nothing stands in the
            // way, as `entry_above` says, so nothing is written below it.
            Err(_) if at != *path => return None,
            Err(_) => Leads::Nowhere,
            Ok(target) => match target.strip_prefix(&*base) {
                Err(_) => Leads::Outside,
                Ok(rest)
                    if rest
                        .components()
                        .any(|c| c.as_os_str().eq_ignore_ascii_case(".git")) =>
                {
                    Leads::Git
                }
                Ok(_) => continue,
            },
        };
        return Some(LinkOut { link: at, leads });
    }
    None
}

/// `refuse_links_out` refuses the repository files `paths`, each with
/// whether its own path counts, as `link_out` says, where a link that leads
/// out of the repository stands on the way to one, and names each such
/// file with its link, the files of one link together, before the command
/// writes any file.
// sokf-implements: CLI-2_AC-89
// sokf-implements: CLI-2_AC-90
pub(crate) fn refuse_links_out(
    root: &RepoRoot,
    paths: impl IntoIterator<Item = (RepoPath, bool)>,
) -> Result<()> {
    let mut links: BTreeMap<LinkOut, BTreeSet<RepoPath>> = BTreeMap::new();
    for (path, at_path) in paths {
        if let Some(out) = link_out(root, &path, at_path) {
            links.entry(out).or_default().insert(path);
        }
    }
    let action = match links.len() {
        0 => return Ok(()),
        1 => "move the link away",
        _ => "move each link away",
    };
    let lines: Vec<String> = links
        .iter()
        .map(|(out, files)| {
            let named: Vec<&str> = files.iter().map(RepoPath::as_str).collect();
            // A link at the path of the one file is named by that file.
            let shown = match named[..] {
                [_] => files.iter().next().map(|f| out.shown(f)),
                _ => None,
            }
            .unwrap_or_else(|| out.shown(&RepoPath::root()));
            format!("{}: pakf doesn't write through {shown}", named.join(", "))
        })
        .collect();
    Err(Error::Pack(format!(
        "{}; {action}, then run the command again",
        lines.join("; ")
    )))
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
        let root = RepoRoot::new(dir.path().to_path_buf());
        let at = |s: &str| RepoPath::parse(s).unwrap();
        let file = dir.path().join("file");
        fs::write(&file, "text\n").unwrap();
        assert_eq!(
            read_dest(&root, &at("/file")),
            Dest::File(b"text\n".to_vec())
        );
        assert!(!is_absent(&file));
        let missing = dir.path().join("missing");
        assert_eq!(read_dest(&root, &at("/missing")), Dest::Absent);
        assert!(is_absent(&missing));
        // A file at a directory above the path stands for no entry at it.
        let below = file.join("below");
        assert_eq!(read_dest(&root, &at("/file/below")), Dest::Absent);
        assert!(is_absent(&below));
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        assert!(matches!(read_dest(&root, &at("/sub")), Dest::Unreadable(_)));
        assert!(!is_absent(&sub));
    }

    #[cfg(unix)]
    #[test]
    fn a_link_reads_as_unreadable_whatever_it_points_at() {
        use std::os::unix::fs::symlink;
        let dir = super::super::TempDir::new().unwrap();
        let root = RepoRoot::new(dir.path().to_path_buf());
        let file = dir.path().join("file");
        fs::write(&file, "text\n").unwrap();
        for (name, target) in [
            ("to-file", file.as_path()),
            ("dangling", Path::new("nowhere")),
        ] {
            let link = dir.path().join(name);
            symlink(target, &link).unwrap();
            let at = RepoPath::parse(&format!("/{name}")).unwrap();
            assert!(
                matches!(read_dest(&root, &at), Dest::Unreadable(_)),
                "{name}"
            );
            assert!(!is_absent(&link), "{name}");
        }
    }

    /// `linked` returns a repository root with a directory `real`, a file
    /// `file`, a `.git` directory, and a directory outside the root, and
    /// keeps both alive.
    #[cfg(unix)]
    fn linked() -> (super::super::TempDir, super::super::TempDir, RepoRoot) {
        let dir = super::super::TempDir::new().unwrap();
        let outside = super::super::TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("real/sub")).unwrap();
        fs::create_dir_all(dir.path().join(".git/hooks")).unwrap();
        fs::write(dir.path().join("file"), "x").unwrap();
        fs::write(outside.path().join("secret"), "s").unwrap();
        let root = RepoRoot::new(dir.path().to_path_buf());
        (dir, outside, root)
    }

    // sokf-tests: CLI-2_AC-89
    // sokf-tests: CLI-2_AC-90
    #[cfg(unix)]
    #[test]
    fn a_link_out_is_found_on_the_way_to_a_path() {
        use std::os::unix::fs::symlink;
        let (dir, outside, root) = linked();
        let at = |s: &str| RepoPath::parse(s).unwrap();
        let d = dir.path();
        symlink(outside.path(), d.join("out")).unwrap();
        symlink("real", d.join("in")).unwrap();
        symlink("out", d.join("chain")).unwrap();
        symlink("real/../..", d.join("up")).unwrap();
        symlink(".git/hooks", d.join("hooks")).unwrap();
        symlink("in/sub", d.join("deep")).unwrap();
        symlink(outside.path().join("secret"), d.join("secret")).unwrap();
        symlink(outside.path().join("none"), d.join("none")).unwrap();
        symlink("nowhere", d.join("dangling")).unwrap();
        symlink(outside.path(), d.join("real/sub/out")).unwrap();
        let found = |p: &str, at_path: bool| {
            link_out(&root, &at(p), at_path).map(|o| (o.link.as_str().to_string(), o.leads))
        };
        let out = |l: &str, leads| Some((l.to_string(), leads));
        assert_eq!(found("/out/x", false), out("/out", Leads::Outside));
        assert_eq!(found("/out/a/b/x", false), out("/out", Leads::Outside));
        assert_eq!(found("/chain/x", false), out("/chain", Leads::Outside));
        assert_eq!(found("/up/x", false), out("/up", Leads::Outside));
        assert_eq!(found("/hooks/x", false), out("/hooks", Leads::Git));
        // A link inside the root counts as its directory, also on the way
        // to a link out below it.
        assert_eq!(found("/in/x", false), None);
        assert_eq!(found("/deep/x", false), None);
        assert_eq!(
            found("/in/sub/out/x", false),
            out("/in/sub/out", Leads::Outside)
        );
        // A link at the path itself counts only where the write follows it.
        assert_eq!(found("/secret", false), None);
        assert_eq!(found("/secret", true), out("/secret", Leads::Outside));
        assert_eq!(found("/none", true), out("/none", Leads::Nowhere));
        assert_eq!(found("/dangling", true), out("/dangling", Leads::Nowhere));
        // A dangling link above the path, or a file there, stands in the
        // way, and nothing is written below it.
        assert_eq!(found("/dangling/x", false), None);
        assert_eq!(found("/file/x", true), None);
        assert_eq!(found("/real/sub/x", true), None);
        assert_eq!(found("/missing/x", true), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_path_below_a_link_out_reads_as_unreadable() {
        use std::os::unix::fs::symlink;
        let (dir, outside, root) = linked();
        symlink(outside.path(), dir.path().join("out")).unwrap();
        let dest = RepoPath::parse("/out/secret").unwrap();
        assert_eq!(
            read_dest(&root, &dest),
            Dest::Unreadable(
                "it lies below the symbolic link /out, which leads outside the repository".into()
            )
        );
        let err = refuse_links_out(
            &root,
            [
                (dest.clone(), false),
                (RepoPath::parse("/out/other").unwrap(), false),
                (RepoPath::parse("/real/x").unwrap(), false),
            ],
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "pack: /out/other, /out/secret: pakf doesn't write through the symbolic link /out, which leads outside the repository; move the link away, then run the command again"
        );
        assert!(refuse_links_out(&root, [(RepoPath::parse("/real/x").unwrap(), true)]).is_ok());
    }
}
