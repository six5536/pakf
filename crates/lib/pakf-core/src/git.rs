//! This module builds each `git` command that pakf runs, so that every one
//! reads the repository of the directory where it runs, whatever repository
//! the environment names (the concern "Running git" of design-pack).

use std::{
    collections::BTreeSet,
    process::{Command, Stdio},
    sync::OnceLock,
};

use crate::RepoRoot;

/// `LOCAL` holds the variables that point git at a repository, once in each
/// process.
static LOCAL: OnceLock<Vec<String>> = OnceLock::new();

/// `KNOWN` holds the variables that point git at a repository as git 2.51
/// lists them, without those of `CONFIG`, so that a git that can't list
/// them, or a wrapper on the path that refuses the option, still has them
/// cleared. It also holds two variables that the list leaves out and that
/// name the state of the user's repository: `GIT_QUARANTINE_PATH`, which
/// git sets beside `GIT_OBJECT_DIRECTORY` for a hook that receives a push,
/// and in which git refuses to update a ref, such as the branch of a clone,
/// and `GIT_ATTR_SOURCE`, the tree of a `git --attr-source`, which a clone
/// lacks, so that its checkout fails.
const KNOWN: [&str; 15] = [
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
    "GIT_QUARANTINE_PATH",
    "GIT_ATTR_SOURCE",
];

/// `CONFIG` holds the variables of `git rev-parse --local-env-vars` that
/// carry the configuration of the command line, the user's `git -c` and
/// `GIT_CONFIG_KEY_<N>`, which name no repository. git itself keeps them
/// when it runs a command in another repository, such as a submodule.
const CONFIG: [&str; 2] = ["GIT_CONFIG_PARAMETERS", "GIT_CONFIG_COUNT"];

/// `local` returns the variables that point git at a repository, such as
/// `GIT_DIR` and `GIT_INDEX_FILE`: those of `KNOWN`, and each other one
/// that `git rev-parse --local-env-vars` lists, without those of `CONFIG`,
/// so that the list follows the git that runs. The probe itself runs
/// without any variable of git's, so that it reads no repository either.
fn local() -> &'static [String] {
    LOCAL.get_or_init(|| {
        let mut probe = Command::new("git");
        probe
            .args(["rev-parse", "--local-env-vars"])
            .stdin(Stdio::null())
            .stderr(Stdio::null());
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("GIT_") {
                probe.env_remove(key);
            }
        }
        let listed = probe
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
            .unwrap_or_default();
        merged(&listed)
    })
}

/// `merged` returns the variables of `KNOWN`, and each other one of
/// `listed`, the output of `git rev-parse --local-env-vars`, that isn't one
/// of `CONFIG`.
fn merged(listed: &str) -> Vec<String> {
    let mut local: Vec<String> = KNOWN.iter().map(|key| (*key).to_string()).collect();
    for key in listed.lines().map(str::trim) {
        if key.starts_with("GIT_") && !CONFIG.contains(&key) && !local.iter().any(|k| k == key) {
            local.push(key.to_string());
        }
    }
    local
}

/// `command` returns a `git` command without the variables that point git
/// at a repository, such as the `GIT_DIR` that a commit hook exports, but
/// with the user's configuration of the command line. git asks for no user
/// name or password on the terminal, so a command that needs one fails, and
/// the user's credential helpers still answer. Each caller sets the
/// directory and adds what only its own commands need.
// sokf-implements: CLI-1_AC-14
pub(crate) fn command() -> Command {
    let mut cmd = Command::new("git");
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    for key in local() {
        cmd.env_remove(key);
    }
    cmd
}

/// `PATHSPECS` holds the settings of the pathspecs that git refuses beside
/// `GIT_LITERAL_PATHSPECS`.
pub(crate) const PATHSPECS: [&str; 3] = [
    "GIT_GLOB_PATHSPECS",
    "GIT_NOGLOB_PATHSPECS",
    "GIT_ICASE_PATHSPECS",
];

/// `literal` makes `cmd` read each pathspec literally, with
/// `GIT_LITERAL_PATHSPECS`, and clears the other settings of the
/// pathspecs, such as the `GIT_GLOB_PATHSPECS` of a `git --glob-pathspecs`,
/// which git refuses beside it.
pub(crate) fn literal(cmd: &mut Command) -> &mut Command {
    cmd.env("GIT_LITERAL_PATHSPECS", "1");
    for key in PATHSPECS {
        cmd.env_remove(key);
    }
    cmd
}

/// `Ignored` is git's list of the files that it ignores below some
/// directories.
#[derive(Debug)]
pub(crate) struct Ignored {
    /// Each path that git ignores from the repository root, such as
    /// `/target`, with an ignored directory named once, without the `/`
    /// after it.
    pub(crate) paths: BTreeSet<String>,
    /// Each warning of git that it is unable to access a file that it reads,
    /// such as a `.gitignore` without read permission, without git's
    /// `warning: `. git still exits with code 0, and its list lacks each
    /// path that an ignore file among them ignores.
    pub(crate) unreadable: Vec<String>,
}

impl Ignored {
    /// `warning` returns git's warnings that it is unable to access a file,
    /// on one line, or none when git gave none.
    pub(crate) fn warning(&self) -> Option<String> {
        (!self.unreadable.is_empty())
            .then(|| format!("`git ls-files` warned: {}", self.unreadable.join(", and ")))
    }
}

/// `ignored` returns each path that git ignores below the directories
/// `dirs`, through one `git ls-files`, which names an ignored directory
/// once, with its `/`, and names an ignored directory above a directory of
/// `dirs` too, with git's warnings that it is unable to access a file, or
/// git's reason when git can't run or fails, as it does outside a git
/// repository. git runs in the C locale, so that its warning reads the same
/// whatever the user's locale.
pub(crate) fn ignored(root: &RepoRoot, dirs: &[String]) -> std::result::Result<Ignored, String> {
    // Each listed directory is a literal pathspec, so that git walks only
    // what the command lists.
    let specs = dirs
        .iter()
        .map(|d| d.trim_start_matches('/'))
        .map(|d| if d.is_empty() { "." } else { d });
    let mut cmd = command();
    let out = literal(&mut cmd)
        .env("LC_ALL", "C")
        .args([
            "ls-files",
            "-z",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
            "--",
        ])
        .args(specs)
        .current_dir(root.as_path())
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run `git`: {e}"))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if !out.status.success() {
        // git can fail without a word, such as when a signal stops it, and
        // then its status is the reason.
        // git's lines join into one, so that the reason stays one line of
        // a refusal.
        let why = if lines.is_empty() {
            out.status.to_string()
        } else {
            lines.join(" ")
        };
        return Err(format!("`git ls-files` failed: {why}"));
    }
    Ok(Ignored {
        paths: out
            .stdout
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| format!("/{}", String::from_utf8_lossy(p).trim_end_matches('/')))
            .collect(),
        unreadable: unreadable(&lines),
    })
}

/// `unreadable` returns each of git's lines of standard error, `lines`,
/// that warns that git is unable to access a file, without its `warning: `.
/// git gives that warning for a file that it can't read, whatever the
/// error, and leaves out each rule of the file. Every other warning, such
/// as `could not open directory`, which the walk of `pakf patch -r` refuses
/// on its own, stays out.
// sokf-implements: CLI-9_AC-22
fn unreadable(lines: &[&str]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|l| l.strip_prefix("warning: "))
        .filter(|l| l.starts_with("unable to access "))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each warning that git is unable to access a file counts, and no other
    // line of git's standard error does.
    #[test]
    fn only_the_warnings_of_a_file_that_git_cant_read_count() {
        let lines = [
            "warning: unable to access '.gitignore': Permission denied",
            "warning: could not open directory 'locked/': Permission denied",
            "unable to access 'x': not a warning",
            "warning: unable to access '.git/info/exclude': Permission denied",
        ];
        let found = unreadable(&lines);
        assert_eq!(
            found,
            [
                "unable to access '.gitignore': Permission denied",
                "unable to access '.git/info/exclude': Permission denied",
            ]
        );
        let ignored = Ignored {
            paths: BTreeSet::new(),
            unreadable: found,
        };
        assert_eq!(
            ignored.warning().unwrap(),
            "`git ls-files` warned: unable to access '.gitignore': Permission denied, and unable to access '.git/info/exclude': Permission denied"
        );
        let none = Ignored {
            paths: BTreeSet::new(),
            unreadable: Vec::new(),
        };
        assert_eq!(none.warning(), None);
    }

    // The list of the git on the path begins with `KNOWN` and keeps the
    // configuration of the command line.
    #[test]
    fn the_list_of_the_git_on_the_path_holds_the_known_variables() {
        assert!(local().starts_with(&merged("")), "{:?}", local());
        for key in ["GIT_CONFIG_PARAMETERS", "GIT_CONFIG_COUNT"] {
            assert!(!local().iter().any(|k| k == key), "{:?}", local());
        }
    }

    // A git that lists no variable still clears each of `KNOWN`, and one that
    // lists a new variable clears it too, once, while the configuration of
    // the command line stays.
    #[test]
    fn the_list_holds_each_known_variable_and_each_new_one() {
        assert_eq!(merged(""), KNOWN.map(str::to_string).to_vec());
        for key in ["GIT_DIR", "GIT_QUARANTINE_PATH", "GIT_ATTR_SOURCE"] {
            assert!(merged("").iter().any(|k| k == key), "{key}");
        }
        let local =
            merged("GIT_DIR\nGIT_CONFIG_COUNT\nGIT_NEW_REPOSITORY\nGIT_CONFIG_PARAMETERS\n");
        let mut expected = KNOWN.map(str::to_string).to_vec();
        expected.push("GIT_NEW_REPOSITORY".to_string());
        assert_eq!(local, expected);
    }

    #[test]
    fn a_command_clears_each_variable_and_never_prompts() {
        let cmd = command();
        let envs: Vec<_> = cmd.get_envs().collect();
        for key in local() {
            assert!(
                envs.contains(&(std::ffi::OsStr::new(key), None)),
                "{key} in {envs:?}"
            );
        }
        assert!(envs.contains(&(
            std::ffi::OsStr::new("GIT_TERMINAL_PROMPT"),
            Some(std::ffi::OsStr::new("0"))
        )));
    }
}
