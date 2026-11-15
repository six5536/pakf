//! This module runs the user's `git`: it fetches a pack with a shallow
//! clone into a temporary directory (the decision "The user's git" of
//! design-pack), merges a file
//! with `git merge-file`, and prints a patch with `git diff --no-index`.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Output,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::{Error, Result};

/// A `TempDir` is a directory under the system's temporary directory that
/// the tool removes when the value is dropped.
#[derive(Debug)]
pub struct TempDir(PathBuf);

/// `NEXT` numbers the temporary directories of one process.
static NEXT: AtomicUsize = AtomicUsize::new(0);

impl TempDir {
    /// `new` creates an empty temporary directory.
    pub fn new() -> Result<Self> {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir =
            std::env::temp_dir().join(format!("pakf-pack-{}-{n}-{nanos:08x}", std::process::id()));
        fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        Ok(TempDir(dir))
    }

    /// `path` returns the directory.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// `git` runs `git` with `args` in `dir` and returns its output. It fails
/// when the program can't start. It builds the command with
/// `crate::git::command`, so that a clone's checkout reads the clone
/// whatever `GIT_DIR` the environment holds, and a source that needs a
/// password fails rather than asking on the terminal.
fn git(dir: Option<&Path>, args: &[&str]) -> Result<Output> {
    git_env(dir, args, &[])
}

/// `git_env` runs `git` as `git` does, with the variables `envs` too.
fn git_env(dir: Option<&Path>, args: &[&str], envs: &[(&str, &std::ffi::OsStr)]) -> Result<Output> {
    let mut cmd = crate::git::command();
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    cmd.envs(envs.iter().copied());
    // git converts no line ending of a file that it checks out or reads,
    // whatever the user's configuration says, such as the
    // `core.autocrlf=true` of git for Windows (the decision "The committed
    // bytes" of design-pack).
    cmd.args(["-c", "core.autocrlf=false", "-c", "core.eol=lf"]);
    cmd.args(args)
        .output()
        .map_err(|e| Error::Pack(format!("cannot run `git`: {e}")))
}

/// `NO_CONVERSION` is the attributes file of a clone. It turns off each
/// conversion that a pack's own `.gitattributes` can ask for at checkout,
/// so that the working tree holds the bytes that the pack committed. The
/// repository's `info/attributes` overrides every `.gitattributes`.
const NO_CONVERSION: &str = "* -text -ident -filter -working-tree-encoding\n";

/// `checkout` checks out `rev` in the clone `repo`, with the bytes of each
/// committed file.
// sokf-implements: CLI-2_AC-20
fn checkout(repo: &Path, rev: &str) -> Result<()> {
    let info = repo.join(".git").join("info");
    fs::create_dir_all(&info).map_err(|e| Error::io(&info, e))?;
    let attributes = info.join("attributes");
    fs::write(&attributes, NO_CONVERSION).map_err(|e| Error::io(&attributes, e))?;
    checked(Some(repo), &["checkout", "--quiet", "--force", rev])?;
    Ok(())
}

/// `checked` runs `git` and fails, with git's own message, when git exits
/// with an error.
fn checked(dir: Option<&Path>, args: &[&str]) -> Result<String> {
    let out = git(dir, args)?;
    if !out.status.success() {
        return Err(Error::Pack(format!(
            "`git {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A `Clone` is a shallow clone of a source, with the ref that it checked
/// out and the commit of that ref.
#[derive(Debug)]
pub struct Clone {
    /// The temporary directory that holds the clone.
    pub dir: TempDir,
    /// The branch, tag, or commit that the clone checked out.
    pub git_ref: String,
    /// The commit that the clone checked out.
    pub commit: String,
}

impl Clone {
    /// `worktree` returns the clone's working tree.
    pub fn worktree(&self) -> PathBuf {
        self.dir.path().join("repo")
    }

    /// `executables` returns the path from the clone's root of each file
    /// that the checked-out commit records as executable. It reads the mode
    /// from git's tree, which records it on every platform, and not from the
    /// working tree, which has no mode on Windows.
    // sokf-implements: CLI-1_AC-26
    pub fn executables(&self) -> Result<BTreeSet<String>> {
        let out = git(Some(&self.worktree()), &["ls-tree", "-r", "-z", "HEAD"])?;
        if !out.status.success() {
            return Err(Error::Pack(format!(
                "`git ls-tree` failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout)
            .split('\0')
            .filter_map(|entry| {
                let (meta, path) = entry.split_once('\t')?;
                meta.starts_with("100755 ").then(|| path.to_string())
            })
            .collect())
    }
}

/// `shallow_clone` fetches `source` at `git_ref` into a temporary directory
/// with a clone of depth 1. Without `git_ref`, it fetches the head of the
/// source's default branch and returns that branch as the ref. A ref that
/// `clone --branch` refuses, such as a commit, is fetched by itself. The
/// clone checks out no file until `checkout` has turned off the pack's
/// conversions.
// sokf-implements: CLI-2_AC-1
// sokf-implements: CLI-2_AC-2
pub fn shallow_clone(source: &str, git_ref: Option<&str>) -> Result<Clone> {
    let dir = TempDir::new()?;
    let repo = dir.path().join("repo");
    let repo_arg = repo.to_string_lossy().into_owned();
    let clone = ["clone", "--quiet", "--no-checkout", "--depth", "1"];
    let git_ref = match git_ref {
        None => {
            checked(None, &[&clone[..], &["--", source, &repo_arg]].concat())?;
            checkout(&repo, "HEAD")?;
            checked(Some(&repo), &["rev-parse", "--abbrev-ref", "HEAD"])?
        }
        Some(r) => {
            let branch = git(
                None,
                &[&clone[..], &["--branch", r, "--", source, &repo_arg]].concat(),
            )?;
            if branch.status.success() {
                checkout(&repo, "HEAD")?;
            } else {
                let _ = fs::remove_dir_all(&repo);
                checked(None, &["init", "--quiet", &repo_arg])?;
                checked(
                    Some(&repo),
                    &["fetch", "--quiet", "--depth", "1", "--", source, r],
                )?;
                checkout(&repo, "FETCH_HEAD")?;
            }
            r.to_string()
        }
    };
    let commit = checked(Some(&repo), &["rev-parse", "HEAD"])?;
    Ok(Clone {
        dir,
        git_ref,
        commit,
    })
}

/// A `Merge` is the result of a three-way merge of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merge {
    /// The merged text, with git's conflict markers where the merge
    /// conflicts.
    pub text: Vec<u8>,
    /// Whether the merge conflicts.
    pub conflicted: bool,
}

/// `merge_file` merges the change from `base` to `upstream` into `current`
/// with `git merge-file`, and labels the sides of a conflict with `label`,
/// `base`, and `upstream`.
// sokf-implements: CLI-2_AC-8
pub fn merge_file(current: &[u8], base: &[u8], upstream: &[u8], label: &str) -> Result<Merge> {
    let dir = TempDir::new()?;
    let write = |name: &str, bytes: &[u8]| -> Result<String> {
        let p = dir.path().join(name);
        fs::write(&p, bytes).map_err(|e| Error::io(&p, e))?;
        Ok(p.to_string_lossy().into_owned())
    };
    let (c, b, u) = (
        write("current", current)?,
        write("base", base)?,
        write("upstream", upstream)?,
    );
    let out = git(
        None,
        &[
            "merge-file",
            "-p",
            "-L",
            label,
            "-L",
            "base",
            "-L",
            "upstream",
            &c,
            &b,
            &u,
        ],
    )?;
    // `git merge-file` exits with the number of conflicts, and with a
    // negative code, which the shell sees as 255, on an error.
    match out.status.code() {
        Some(0) => Ok(Merge {
            text: out.stdout,
            conflicted: false,
        }),
        Some(n) if (1..=127).contains(&n) => Ok(Merge {
            text: out.stdout,
            conflicted: true,
        }),
        _ => Err(Error::Pack(format!(
            "`git merge-file` failed on {label}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))),
    }
}

/// `apply` applies `patch`, whose paths are relative to the pack's directory
/// `dir`, to the working tree that holds `dir`, whole or not at all: it runs
/// `git apply --check` first, so that a patch that doesn't apply changes no
/// file. git reads a patch's paths from the root of the working tree, and
/// leaves out without an error each path outside the directory that it runs
/// in, so `--directory` puts the place of `dir` in its working tree before
/// each path (the decision "A patch applied to a local source" of
/// design-pack).
// sokf-implements: CLI-9_AC-11
// sokf-implements: CLI-9_AC-13
pub fn apply(dir: &Path, patch: &str) -> Result<()> {
    let shown = dir.display();
    if !dir.is_dir() {
        return Err(Error::Pack(format!(
            "--apply: the pack's directory {shown} doesn't exist"
        )));
    }
    let inside = git(Some(dir), &["rev-parse", "--is-inside-work-tree"])?;
    if !inside.status.success() || String::from_utf8_lossy(&inside.stdout).trim() != "true" {
        return Err(Error::Pack(format!(
            "--apply: the pack's directory {shown} lies in no working tree of git"
        )));
    }
    let prefix = checked(Some(dir), &["rev-parse", "--show-prefix"])?;
    let tmp = TempDir::new()?;
    let file = tmp.path().join("pack.patch");
    fs::write(&file, patch).map_err(|e| Error::io(&file, e))?;
    let file = file.to_string_lossy().into_owned();
    let directory = format!("--directory={prefix}");
    let mut args = vec!["apply"];
    if !prefix.is_empty() {
        args.push(&directory);
    }
    let check = applying(dir, &[&args[..], &["--check", &file]].concat())?;
    if !check.status.success() {
        return Err(Error::Pack(format!(
            "--apply: the patch doesn't apply to {shown}, so no file changed: {}",
            String::from_utf8_lossy(&check.stderr).trim()
        )));
    }
    let out = applying(dir, &[&args[..], &[&file]].concat())?;
    if !out.status.success() {
        return Err(Error::Pack(format!(
            "--apply: `git apply` failed in {shown}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(())
}

/// `applying` runs `git apply` with `args` in `dir`, with the user's own
/// settings of line endings, and not those of `git`, which a clone's
/// checkout needs, because the working tree of a local source is the
/// user's, and git converts the patch's LF as the user's settings say.
fn applying(dir: &Path, args: &[&str]) -> Result<Output> {
    crate::git::command()
        .current_dir(dir)
        .args(args)
        .output()
        .map_err(|e| Error::Pack(format!("cannot run `git`: {e}")))
}

/// `diff` returns the unified diff from `base` to `current` of the file
/// `rel` of a pack, with the paths `a/<rel>` and `b/<rel>`, or an empty text
/// when the two are equal. `current` is `None` for a file that the user
/// deleted.
// sokf-implements: CLI-8_AC-1
pub fn diff(rel: &str, base: &[u8], current: Option<&[u8]>) -> Result<String> {
    diff_modes(rel, Some(base), current, None)
}

/// `diff_modes` is `diff` with the executable bit of each side, `(base,
/// current)`, when `modes` gives them, so that the patch carries a change of
/// the bit as git's `old mode` and `new mode` lines, which `git apply`
/// applies. `base` is `None` for a file that the pack doesn't hold yet,
/// which the patch adds with git's `new file mode` line. A platform without
/// the bit writes no mode line. Where either side is binary, the diff is a
/// git binary patch, which `git apply` applies to reproduce the bytes,
/// because git marks each file binary through an attributes file of its
/// own, so that it judges no file without a NUL byte as text.
// sokf-implements: CLI-8_AC-5
// sokf-implements: CLI-9_AC-4
// sokf-implements: CLI-9_AC-1
pub fn diff_modes(
    rel: &str,
    base: Option<&[u8]>,
    current: Option<&[u8]>,
    modes: Option<(bool, bool)>,
) -> Result<String> {
    let dir = TempDir::new()?;
    let write = |side: &str, bytes: &[u8]| -> Result<()> {
        let p = dir.path().join(side).join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        fs::write(&p, bytes).map_err(|e| Error::io(&p, e))
    };
    let side = |name: &str, bytes: Option<&[u8]>| -> Result<String> {
        match bytes {
            Some(bytes) => {
                write(name, bytes)?;
                Ok(format!("{name}/{rel}"))
            }
            None => Ok("/dev/null".to_string()),
        }
    };
    let a = side("a", base)?;
    let b = side("b", current)?;
    // A deleted file has no `b` side, and a new file no `a` side, and the
    // other side still carries its bit, which git writes as the `deleted
    // file mode` or the `new file mode`.
    if let Some((old, new)) = modes {
        let sides = [("a", base.map(|_| old)), ("b", current.map(|_| new))];
        for (side, bit) in sides {
            if let Some(bit) = bit {
                let p = dir.path().join(side).join(rel);
                super::mode::set_executable(&p, bit).map_err(|e| Error::io(&p, e))?;
            }
        }
    }
    // pakf's own attributes file stands in for the user's, so that no
    // attribute or textconv of the user's turns a text file binary or
    // rewrites its content, and marks each file binary where a side is.
    let binary = [base, current].into_iter().flatten().any(super::is_binary);
    let attributes = dir.path().join("attributes");
    let marks = if binary { "* binary\n" } else { "" };
    fs::write(&attributes, marks).map_err(|e| Error::io(&attributes, e))?;
    let mut args = vec![
        "-c".to_string(),
        "core.quotePath=false".to_string(),
        "-c".to_string(),
        format!("core.attributesFile={}", attributes.display()),
    ];
    args.extend(
        [
            "diff",
            "--no-index",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--no-prefix",
        ]
        .map(String::from),
    );
    if binary {
        args.push("--binary".into());
    }
    args.extend(["--".to_string(), a, b]);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    // git reads no system attributes, and no repository above the
    // temporary directory.
    let ceiling = dir.path().parent().unwrap_or(dir.path());
    let out = git_env(
        Some(dir.path()),
        &args,
        &[
            ("GIT_ATTR_NOSYSTEM", "1".as_ref()),
            ("GIT_CEILING_DIRECTORIES", ceiling.as_os_str()),
        ],
    )?;
    // `git diff --no-index` exits with 1 when the files differ.
    match out.status.code() {
        Some(0 | 1) => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        _ => Err(Error::Pack(format!(
            "`git diff` failed on {rel}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_merge_keeps_both_changes_or_marks_the_conflict() {
        let base = b"one\ntwo\nthree\nfour\nfive\n";
        let current = b"one\nTWO\nthree\nfour\nfive\n";
        let upstream = b"one\ntwo\nthree\nfour\nFIVE\n";
        let m = merge_file(current, base, upstream, "/knowledge/a.md").unwrap();
        assert!(!m.conflicted);
        assert_eq!(m.text, b"one\nTWO\nthree\nfour\nFIVE\n");
        let m = merge_file(current, base, b"one\n2\nthree\nfour\nfive\n", "/k/a.md").unwrap();
        assert!(m.conflicted);
        let text = String::from_utf8(m.text).unwrap();
        assert!(text.contains("<<<<<<< /k/a.md\n"), "{text}");
        assert!(text.contains(">>>>>>> upstream\n"), "{text}");
    }

    #[test]
    fn a_diff_names_the_paths_of_the_pack() {
        let d = diff("schemas/a.md", b"x\n", Some(b"y\n")).unwrap();
        assert!(d.contains("--- a/schemas/a.md\n"), "{d}");
        assert!(d.contains("+++ b/schemas/a.md\n"), "{d}");
        assert!(d.contains("-x\n+y\n"), "{d}");
        assert_eq!(diff("schemas/a.md", b"x\n", Some(b"x\n")).unwrap(), "");
    }

    // sokf-tests: CLI-8_AC-5
    #[cfg(unix)]
    #[test]
    fn a_diff_carries_the_executable_bit_of_each_side() {
        let rel = "files/run.sh";
        let d = diff_modes(rel, Some(b"x\n"), Some(b"x\n"), Some((false, true))).unwrap();
        assert!(d.contains("old mode 100644\nnew mode 100755\n"), "{d}");
        let d = diff_modes(rel, Some(b"x\n"), Some(b"y\n"), Some((true, true))).unwrap();
        assert!(d.contains(" 100755\n"), "{d}");
        assert!(!d.contains("old mode"), "{d}");
        let d = diff_modes(rel, Some(b"x\n"), None, Some((true, false))).unwrap();
        assert!(d.contains("deleted file mode 100755\n"), "{d}");
    }

    // sokf-tests: CLI-9_AC-4
    #[cfg(unix)]
    #[test]
    fn a_new_file_carries_its_executable_bit() {
        let rel = "files/scripts/new.sh";
        let d = diff_modes(rel, None, Some(b"x\n"), Some((false, true))).unwrap();
        assert!(d.contains("new file mode 100755\n"), "{d}");
        assert!(
            d.contains("--- /dev/null\n+++ b/files/scripts/new.sh\n"),
            "{d}"
        );
        let d = diff_modes(rel, None, Some(b"x\n"), Some((false, false))).unwrap();
        assert!(d.contains("new file mode 100644\n"), "{d}");
    }

    // sokf-tests: CLI-9_AC-1
    #[test]
    fn a_diff_of_a_binary_file_is_a_git_binary_patch() {
        // A file without a NUL byte that isn't UTF-8 is binary too.
        let d = diff("files/LICENSE", b"MIT\n", Some(b"MIT \xff\n")).unwrap();
        assert!(d.contains("GIT binary patch\n"), "{d}");
        assert!(!d.contains("+MIT"), "{d}");
        let d = diff_modes("files/a.bin", None, Some(b"a\x00"), None).unwrap();
        assert!(d.contains("new file mode"), "{d}");
        assert!(d.contains("GIT binary patch\n"), "{d}");
        assert_eq!(diff("files/a.bin", b"a\x00", Some(b"a\x00")).unwrap(), "");
    }

    #[test]
    fn a_temporary_directory_goes_when_dropped() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        assert!(path.is_dir());
        drop(dir);
        assert!(!path.exists());
    }
}
