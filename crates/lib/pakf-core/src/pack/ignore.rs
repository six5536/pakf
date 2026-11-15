//! This module reads the ignores of `pakf patch`: `/.pakf/ignore`, the
//! project's own file, and each file under `/.pakf/ignore.d/`, which a pack
//! ships under its own id, as one list of patterns in the syntax of
//! `.gitignore` (the decision "The ignores of a patch" of design-pack).

use std::{fs, io::ErrorKind, path::Path};

use ignore::gitignore::{Gitignore, GitignoreBuilder};

use super::dest::{Dest, read_dest};
use crate::{Error, RepoPath, RepoRoot, Result};

/// `IGNORE` is the project's own ignore file.
const IGNORE: &str = "/.pakf/ignore";

/// `IGNORE_D` is the folder whose files the packs ship, each under the
/// pack's id.
const IGNORE_D: &str = "/.pakf/ignore.d";

/// An `Ignores` is the list of the patterns of every ignore file.
#[derive(Debug, Clone)]
pub struct Ignores {
    matcher: Gitignore,
}

/// `refuse` returns the refusal of the ignore file `file` for the reason
/// `why`.
fn refuse(file: &RepoPath, why: &str) -> Error {
    Error::Pack(format!(
        "{file}: the ignore file can't be read: {why}; fix it, then run the command again"
    ))
}

impl Ignores {
    /// `read` reads the files under `/.pakf/ignore.d/` in the order of
    /// their paths, and then `/.pakf/ignore`, so that a later pattern, such
    /// as a `!` of the project's own file, decides a path that an earlier
    /// one names. It reads each pattern from the repository root, without
    /// a byte-order mark, and refuses an entry at the path of an ignore file
    /// that isn't a readable regular file, a symbolic link at
    /// `/.pakf/ignore.d` or below it, and a file that doesn't parse. It compares the cases of letters as git does by default
    /// on the platform: apart on Linux, and alike on macOS and Windows.
    // sokf-implements: CONF-5_AC-1
    // sokf-implements: CONF-5_AC-2
    // sokf-implements: CONF-5_AC-3
    pub fn read(root: &RepoRoot) -> Result<Ignores> {
        let mut files = Vec::new();
        let dir = RepoPath::parse(IGNORE_D).expect("literal");
        collect(root, &dir, &mut files)?;
        // The order of the paths compares them name by name.
        files.sort_by(|a, b| a.segments().cmp(b.segments()));
        files.push(RepoPath::parse(IGNORE).expect("literal"));
        let mut builder = GitignoreBuilder::new(root.as_path());
        builder
            .case_insensitive(cfg!(any(windows, target_os = "macos")))
            .map_err(|e| Error::Pack(e.to_string()))?;
        for file in &files {
            let text = match read_dest(&file.to_fs(root)) {
                Dest::File(bytes) => {
                    String::from_utf8(bytes).map_err(|_| refuse(file, "the text isn't UTF-8"))?
                }
                Dest::Unreadable(why) => return Err(refuse(file, &why)),
                Dest::Absent => continue,
            };
            let from = Path::new(file.as_str());
            for line in text.trim_start_matches('\u{feff}').lines() {
                builder
                    .add_line(Some(from.to_path_buf()), &literal_braces(line))
                    .map_err(|e| refuse(file, &e.to_string()))?;
            }
        }
        let matcher = builder.build().map_err(|e| Error::Pack(e.to_string()))?;
        Ok(Ignores { matcher })
    }

    /// `ignored` returns the ignore file whose pattern names `path` or a
    /// directory above it, as git reads them: a directory that the list
    /// ignores holds every path below it, which no `!` takes back, and
    /// otherwise the last pattern that names `path` decides it. `is_dir` says
    /// whether `path` is a directory.
    // sokf-implements: CLI-9_AC-31
    // sokf-implements: CLI-9_AC-32
    pub fn ignored(&self, path: &RepoPath, is_dir: bool) -> Option<RepoPath> {
        let mut above: Vec<RepoPath> = std::iter::successors(path.parent(), RepoPath::parent)
            .take_while(|p| *p != RepoPath::root())
            .collect();
        above.reverse();
        above
            .iter()
            .find_map(|dir| self.ignored_here(dir, true))
            .or_else(|| self.ignored_here(path, is_dir))
    }

    /// `ignored_here` returns the ignore file whose last pattern that names
    /// `path` itself ignores it, without the directories above it, for a
    /// walk that checked them on its way down.
    pub fn ignored_here(&self, path: &RepoPath, is_dir: bool) -> Option<RepoPath> {
        if *path == RepoPath::root() {
            return None;
        }
        let m = self.matcher.matched(path.relative(), is_dir);
        if !m.is_ignore() {
            return None;
        }
        Some(
            m.inner()
                .and_then(|g| g.from())
                .and_then(|p| RepoPath::parse(&p.to_string_lossy()))
                .unwrap_or_else(|| RepoPath::parse(IGNORE).expect("literal")),
        )
    }
}

/// `literal_braces` returns the pattern `line` with each brace that no
/// backslash escapes escaped, because git's patterns have no alternation of
/// `{a,b}`, which the glob of the crate `ignore` reads.
fn literal_braces(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let (mut escaped, mut class) = (false, false);
    for c in line.chars() {
        // A brace in a class of characters is already a letter.
        if !escaped && !class && matches!(c, '{' | '}') {
            out.push('\\');
        }
        if !escaped {
            class = match c {
                '[' => true,
                ']' => false,
                _ => class,
            };
        }
        escaped = !escaped && c == '\\';
        out.push(c);
    }
    out
}

/// `collect` adds each file below the directory `dir` to `files`, at any
/// depth, without following a symbolic link, and refuses a link and a
/// folder that it can't read. An absent folder holds none.
fn collect(root: &RepoRoot, dir: &RepoPath, files: &mut Vec<RepoPath>) -> Result<()> {
    let path = dir.to_fs(root);
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            return Ok(());
        }
        Err(e) => return Err(refuse(dir, &e.to_string())),
    };
    if meta.file_type().is_symlink() {
        return Err(refuse(dir, "the folder is a symbolic link"));
    }
    if !meta.is_dir() {
        return Err(refuse(dir, "the folder isn't a directory"));
    }
    let entries = fs::read_dir(&path).map_err(|e| refuse(dir, &e.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|e| refuse(dir, &e.to_string()))?;
        let name = entry.file_name();
        let Some(child) = name.to_str().and_then(|n| dir.join(n)) else {
            return Err(refuse(dir, "the name of an entry isn't UTF-8"));
        };
        let kind = entry
            .file_type()
            .map_err(|e| refuse(&child, &e.to_string()))?;
        if kind.is_dir() {
            collect(root, &child, files)?;
        } else {
            files.push(child);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> RepoPath {
        RepoPath::parse(s).unwrap()
    }

    // sokf-tests: CONF-5_AC-1
    // sokf-tests: CONF-5_AC-2
    // sokf-tests: CLI-9_AC-31
    // sokf-tests: CLI-9_AC-32
    #[test]
    fn the_ignores_act_as_one_list_with_the_projects_file_last() {
        let dir = crate::test_support::fresh_dir("ignores");
        let root = RepoRoot::new(dir.clone());
        let none = Ignores::read(&root).unwrap();
        assert_eq!(none.ignored(&p("/knowledge/index.md"), false), None);
        fs::create_dir_all(dir.join(".pakf/ignore.d/@sokf/sokf")).unwrap();
        fs::create_dir_all(dir.join(".pakf/ignore.d/@acme/house")).unwrap();
        fs::write(
            dir.join(".pakf/ignore.d/@sokf/sokf/ignore"),
            "**/index.md\n# a comment\n/build/\n",
        )
        .unwrap();
        fs::write(
            dir.join(".pakf/ignore.d/@acme/house/ignore"),
            "!/docs/index.md\n",
        )
        .unwrap();
        fs::write(dir.join(".pakf/ignore"), "!/notes/index.md\n*.log\n").unwrap();
        let ig = Ignores::read(&root).unwrap();
        let sokf = p("/.pakf/ignore.d/@sokf/sokf/ignore");
        assert_eq!(
            ig.ignored(&p("/knowledge/schemas/index.md"), false),
            Some(sokf.clone())
        );
        // A pattern from the root holds a directory and each file below it.
        assert_eq!(
            ig.ignored(&p("/build/out/a.txt"), false),
            Some(sokf.clone())
        );
        assert!(ig.ignored(&p("/build"), true).is_some());
        assert_eq!(ig.ignored(&p("/src/build/a.txt"), false), None);
        // The path order puts `@acme` before `@sokf`, so the later pattern
        // of `@sokf` decides the index of `/docs`.
        assert!(ig.ignored(&p("/docs/index.md"), false).is_some());
        // The project's own file comes last.
        assert_eq!(ig.ignored(&p("/notes/index.md"), false), None);
        assert_eq!(ig.ignored(&p("/a/b.log"), false), Some(p("/.pakf/ignore")));
        assert_eq!(ig.ignored(&p("/src/main.rs"), false), None);
        // The order compares the paths name by name, so `house` comes before
        // `house-x`, whose `!` then decides.
        fs::create_dir_all(dir.join(".pakf/ignore.d/@acme/house-x")).unwrap();
        fs::write(dir.join(".pakf/ignore.d/@acme/house-x/ignore"), "!/b.md\n").unwrap();
        fs::write(dir.join(".pakf/ignore.d/@acme/house/ignore"), "/b.md\n").unwrap();
        let ig = Ignores::read(&root).unwrap();
        assert_eq!(ig.ignored(&p("/b.md"), false), None);
        // A `!` takes back no path below a directory that a pattern
        // ignores, as in git, and a byte-order mark begins no pattern.
        fs::write(
            dir.join(".pakf/ignore"),
            "\u{feff}/vendor/\n!/vendor/keep.txt\n",
        )
        .unwrap();
        let ig = Ignores::read(&root).unwrap();
        assert!(ig.ignored(&p("/vendor/keep.txt"), false).is_some());
        assert!(ig.ignored(&p("/vendor"), true).is_some());
        // A brace is a letter of the name, as in git.
        fs::write(dir.join(".pakf/ignore"), "{draft}.md\nnotes{old\n").unwrap();
        let ig = Ignores::read(&root).unwrap();
        assert!(ig.ignored(&p("/{draft}.md"), false).is_some());
        assert_eq!(ig.ignored(&p("/draft.md"), false), None);
        assert!(ig.ignored(&p("/notes{old"), false).is_some());
        fs::write(dir.join(".pakf/ignore"), "*[{]*\n").unwrap();
        let ig = Ignores::read(&root).unwrap();
        assert!(ig.ignored(&p("/a{b.txt"), false).is_some());
        assert_eq!(ig.ignored(&p("/a\\b.txt"), false), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    // sokf-tests: CONF-5_AC-3
    #[test]
    fn an_ignore_file_that_cant_be_read_is_refused() {
        let dir = crate::test_support::fresh_dir("ignores-bad");
        let root = RepoRoot::new(dir.clone());
        fs::create_dir_all(dir.join(".pakf/ignore")).unwrap();
        let e = Ignores::read(&root).unwrap_err().to_string();
        assert!(
            e.contains("/.pakf/ignore: the ignore file can't be read"),
            "{e}"
        );
        fs::remove_dir_all(dir.join(".pakf/ignore")).unwrap();
        fs::create_dir_all(dir.join(".pakf/ignore.d/@a/b")).unwrap();
        fs::write(dir.join(".pakf/ignore.d/@a/b/ignore"), b"\xff\n").unwrap();
        let e = Ignores::read(&root).unwrap_err().to_string();
        assert!(e.contains("/.pakf/ignore.d/@a/b/ignore: "), "{e}");
        fs::write(dir.join(".pakf/ignore.d/@a/b/ignore"), "[z-a]\n").unwrap();
        let e = Ignores::read(&root).unwrap_err().to_string();
        assert!(e.contains("/.pakf/ignore.d/@a/b/ignore: "), "{e}");
        fs::remove_dir_all(&dir).unwrap();
    }
}
