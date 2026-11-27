//! This module reads the table `[compare]` of `/.pakf/config.toml`, which
//! sets how pakf compares a pack's files by their meaning through
//! `same-same` (the decision "The configuration of the comparison" of
//! design-pack): whether pakf asks the library to detect the project's
//! formatter, and the rules that name, for the files of their patterns, the
//! named things that count as an edit, and the formatter or the command
//! that lays the files out.

use std::{path::Path, time::Duration};

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use same_same::{KnownFormatter, Named, NamedSet, Settings};
use serde::de::DeserializeOwned;

use super::file::config_path;
use crate::{Error, RepoPath, RepoRoot, Result, source::read_optional};

/// `TIMEOUT` is the time that one run of a configured formatter, a
/// configured command, or the detected formatter takes at most: 10
/// seconds.
// sokf-implements: CONF-1_AC-14
pub const TIMEOUT: Duration = Duration::from_secs(10);

// sokf:begin compare
/// `CompareTable` holds the table `[compare]` of `/.pakf/config.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareTable {
    /// Whether pakf asks same-same to detect the project's formatter at the
    /// repository root, for a file of a type that same-same doesn't know and
    /// that no rule's formatter or command covers, in the commands that
    /// report. Default: false.
    pub detect: bool,
    /// The rules, in their order: the last rule whose patterns match a file
    /// decides each of its keys for the file.
    pub rule: Vec<CompareRule>,
}

/// `CompareRule` holds one `[[compare.rule]]` entry: the files of its
/// patterns, and the named things that count as an edit of them, the
/// formatter or the command that lays them out, or both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareRule {
    /// The files, as patterns in the syntax of `.gitignore`, read from the
    /// repository root against each file's destination, such as `*.css`.
    pub files: Vec<String>,
    /// The stable names of the named things of same-same that count as an
    /// edit of the files, such as `["import-order"]`. Default, where no
    /// rule that matches a file sets it: `["yaml11-value"]`.
    pub edits: Option<Vec<String>>,
    /// A formatter that same-same knows, which lays the files out: `oxfmt`,
    /// `prettier`, `biome`, or `dprint`.
    pub formatter: Option<String>,
    /// The formatter's configuration file, by its path from the repository
    /// root, such as `.prettierrc.json`.
    pub config: Option<String>,
    /// Whose settings the formatter runs with: `internal-with-overrides`,
    /// the default, `as-written`, or `internal-only`.
    pub settings: Option<String>,
    /// Prettier's plugins, each the name of a package of the project, such
    /// as `prettier-plugin-sort-json`.
    pub plugins: Option<Vec<String>>,
    /// Any other program that lays the files out, with its arguments, run
    /// exactly as written, such as `["stylelint", "--fix", "{files}"]`.
    pub command: Option<Vec<String>>,
    /// Whether the formatter or the command comes before same-same's own
    /// comparison by meaning for a file of a type that the library knows.
    /// Default: false. The key is `override`.
    pub overrides: Option<bool>,
}
// sokf:end compare

/// `RULE_KEYS` are the keys of a `[[compare.rule]]` entry.
const RULE_KEYS: &[&str] = &[
    "files",
    "edits",
    "formatter",
    "config",
    "settings",
    "plugins",
    "command",
    "override",
];

/// `key` returns the value of the key `name` of the table `t` as a `T`,
/// none where the table lacks it, or why its value is of the wrong type.
fn key<T: DeserializeOwned>(t: &toml::Table, name: &str) -> std::result::Result<Option<T>, String> {
    t.get(name)
        .map(|v| {
            v.clone()
                .try_into()
                .map_err(|e: toml::de::Error| format!("`{name}`: {}", e.message()))
        })
        .transpose()
}

/// `unknown` returns why the table `t` is refused where it holds a key
/// other than `keys`, or none.
fn unknown(t: &toml::Table, keys: &[&str]) -> std::result::Result<(), String> {
    match t.keys().find(|k| !keys.contains(&k.as_str())) {
        Some(k) => Err(format!(
            "unknown key `{k}`, {}",
            expected(keys.iter().copied())
        )),
        None => Ok(()),
    }
}

/// `expected` returns the names that a value can take, each in backticks,
/// as serde words them: expected one name, expected two names joined by
/// `or`, or expected one of three or more names.
fn expected<T: std::fmt::Display>(names: impl Iterator<Item = T>) -> String {
    let names: Vec<String> = names.map(|n| format!("`{n}`")).collect();
    match names.as_slice() {
        [one] => format!("expected {one}"),
        [a, b] => format!("expected {a} or {b}"),
        _ => format!("expected one of {}", names.join(", ")),
    }
}

/// `rule_of` reads one `[[compare.rule]]` entry, or why it is refused.
fn rule_of(v: &toml::Value) -> std::result::Result<CompareRule, String> {
    let t = v.as_table().ok_or("the rule is not a table")?;
    unknown(t, RULE_KEYS)?;
    Ok(CompareRule {
        files: key(t, "files")?.ok_or("the rule lacks `files`")?,
        edits: key(t, "edits")?,
        formatter: key(t, "formatter")?,
        config: key(t, "config")?,
        settings: key(t, "settings")?,
        plugins: key(t, "plugins")?,
        command: key(t, "command")?,
        overrides: key(t, "override")?,
    })
}

/// A `Compare` is the checked configuration of the comparison: whether
/// detection is on, and the rules in their order. By default, detection is
/// off and there is no rule.
#[derive(Debug, Clone, Default)]
pub struct Compare {
    /// Whether pakf asks same-same to detect the project's formatter.
    pub detect: bool,
    /// The rules, in their order.
    pub rules: Vec<Rule>,
}

/// A `Rule` is one checked rule of `[compare]`.
#[derive(Debug, Clone)]
pub struct Rule {
    /// The patterns as written, which pakf gives same-same with braces
    /// escaped as `/.pakf/ignore` reads them.
    pub files: Vec<String>,
    /// The matcher of the patterns, from the repository root.
    matcher: Gitignore,
    /// The named things that count as an edit, where the rule sets `edits`.
    pub edits: Option<NamedSet>,
    /// The formatter or the command of the rule, where it sets one.
    pub normaliser: Option<LayOut>,
}

/// A `LayOut` is what a rule lays its files out with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayOut {
    /// A formatter that same-same knows.
    Formatter {
        /// The formatter.
        formatter: KnownFormatter,
        /// Its configuration file, from the repository root.
        config: Option<String>,
        /// Whose settings it runs with.
        settings: Settings,
        /// Prettier's plugins.
        plugins: Vec<String>,
        /// Whether it comes before the comparison by meaning.
        overrides: bool,
    },
    /// Any other program, run as written.
    Command {
        /// The program and its arguments.
        argv: Vec<String>,
        /// Whether it comes before the comparison by meaning.
        overrides: bool,
    },
}

impl Compare {
    /// `laid_out_by` returns, for the file at `dest`, the number of the last
    /// rule, from 1, whose patterns match it and that names a formatter or
    /// a command, with the name of that formatter or of the command's
    /// program as the rule writes it.
    pub fn laid_out_by(&self, dest: &RepoPath) -> Option<(usize, String)> {
        self.rules
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, r)| r.matches(dest))
            .find_map(|(i, r)| Some((i + 1, r.program()?)))
    }

    /// `edits` returns the named things that count as an edit of the file
    /// at `dest`: those of the last rule whose patterns match `dest` and
    /// that sets `edits`, or else how a YAML 1.1 reader takes a value.
    // sokf-implements: CONF-1_AC-8
    // sokf-implements: CONF-1_AC-12
    pub fn edits(&self, dest: &RepoPath) -> NamedSet {
        self.rules
            .iter()
            .rev()
            .filter(|r| r.matches(dest))
            .find_map(|r| r.edits)
            .unwrap_or_else(|| NamedSet::of(Named::Yaml11Value))
    }
}

impl Rule {
    /// `matches` returns whether the patterns of the rule match `dest` or a
    /// directory above it, as git reads a pattern of `.gitignore`.
    pub fn matches(&self, dest: &RepoPath) -> bool {
        *dest != RepoPath::root()
            && self
                .matcher
                .matched_path_or_any_parents(dest.relative(), false)
                .is_ignore()
    }

    /// `program` returns the name of the rule's formatter, or of its
    /// command's program as the rule writes it, or none where the rule
    /// names neither.
    pub fn program(&self) -> Option<String> {
        match self.normaliser.as_ref()? {
            LayOut::Formatter { formatter, .. } => Some(formatter.name().to_string()),
            LayOut::Command { argv, .. } => argv.first().cloned(),
        }
    }
}

/// `read_compare` reads the table `[compare]` of `/.pakf/config.toml`, and
/// the default configuration where the file or the table is absent. It
/// refuses a key that the contract doesn't define, a value of the wrong
/// type, a rule without `files` or with neither `edits`, `formatter`, nor
/// `command`, both `formatter` and `command`, `override` without either,
/// `config`, `settings`, or `plugins` without `formatter`, `plugins` for a
/// formatter other than Prettier, a formatter or a way of its settings that
/// same-same doesn't name, an empty `command`, a pattern that doesn't
/// parse, and a named thing in `edits` that same-same doesn't name or
/// `line-endings`, which pakf reads before the library.
// sokf-implements: CONF-1_AC-7
// sokf-implements: CONF-1_AC-11
pub fn read_compare(root: &RepoRoot) -> Result<Compare> {
    let path = config_path();
    let Some(text) = read_optional(&path.to_fs(root))? else {
        return Ok(Compare::default());
    };
    let value: toml::Table =
        toml::from_str(&text).map_err(|e| Error::config(path.clone(), e.message()))?;
    let Some(table) = value.get("compare") else {
        return Ok(Compare::default());
    };
    let refuse = |m: String| Error::config(path.clone(), m);
    let Some(table) = table.as_table() else {
        return Err(refuse("`compare` is not a table".into()));
    };
    let rules = match table.get("rule") {
        None => Vec::new(),
        Some(toml::Value::Array(rules)) => rules.clone(),
        Some(_) => return Err(refuse("`compare.rule` is not a list of tables".into())),
    };
    unknown(table, &["detect", "rule"]).map_err(|why| refuse(format!("[compare]: {why}")))?;
    let rules = rules
        .iter()
        .enumerate()
        .map(|(i, r)| {
            rule_of(r).map_err(|why| refuse(format!("[[compare.rule]] number {}: {why}", i + 1)))
        })
        .collect::<Result<Vec<_>>>()?;
    let table = CompareTable {
        detect: key(table, "detect")
            .map_err(|why| refuse(format!("[compare]: {why}")))?
            .unwrap_or(false),
        rule: rules,
    };
    let mut out = Compare {
        detect: table.detect,
        rules: Vec::new(),
    };
    for (i, rule) in table.rule.into_iter().enumerate() {
        let rule = checked(root.as_path(), rule)
            .map_err(|why| refuse(format!("[[compare.rule]] number {}: {why}", i + 1)))?;
        out.rules.push(rule);
    }
    Ok(out)
}

/// `checked` returns the rule `r`, read from the repository root `root`, or
/// why it is refused.
fn checked(root: &Path, r: CompareRule) -> std::result::Result<Rule, String> {
    if r.edits.is_none() && r.formatter.is_none() && r.command.is_none() {
        return Err("the rule holds none of `edits`, `formatter`, and `command`".into());
    }
    if r.formatter.is_some() && r.command.is_some() {
        return Err("the rule holds both `formatter` and `command`; give each its own rule".into());
    }
    if r.formatter.is_none() {
        for (key, set) in [
            ("config", r.config.is_some()),
            ("settings", r.settings.is_some()),
            ("plugins", r.plugins.is_some()),
        ] {
            if set {
                return Err(format!("`{key}` needs `formatter`"));
            }
        }
        if r.command.is_none() && r.overrides.is_some() {
            return Err("`override` needs `formatter` or `command`".into());
        }
    }
    let mut builder = GitignoreBuilder::new(root);
    for pattern in &r.files {
        builder
            .add_line(None, &crate::pack::literal_braces(pattern))
            .map_err(|e| format!("the pattern `{pattern}` of `files` doesn't parse: {e}"))?;
    }
    let matcher = builder.build().map_err(|e| e.to_string())?;
    let edits = r.edits.as_ref().map(|names| named(names)).transpose()?;
    let overrides = r.overrides.unwrap_or(false);
    let normaliser = match (&r.formatter, &r.command) {
        (Some(name), _) => {
            let formatter = KnownFormatter::from_name(name).ok_or_else(|| {
                format!(
                    "`formatter` names `{name}`, {}",
                    expected(KnownFormatter::ALL.iter().map(|f| f.name()))
                )
            })?;
            let settings = match &r.settings {
                None => Settings::default(),
                Some(s) => Settings::from_name(s).ok_or_else(|| {
                    format!("`settings` names `{s}`, {}", expected(Settings::ALL.iter()))
                })?,
            };
            if r.plugins.is_some() && formatter != KnownFormatter::Prettier {
                return Err(format!(
                    "`plugins` needs `formatter = \"prettier\"`, and the rule names `{name}`"
                ));
            }
            Some(LayOut::Formatter {
                formatter,
                config: r.config.as_deref().map(repository_path).transpose()?,
                settings,
                plugins: r.plugins.clone().unwrap_or_default(),
                overrides,
            })
        }
        (None, Some(argv)) => {
            if argv.first().is_none_or(String::is_empty) {
                return Err("`command` names no program".into());
            }
            command_line(&argv[1..])?;
            Some(LayOut::Command {
                argv: argv.clone(),
                overrides,
            })
        }
        (None, None) => None,
    };
    Ok(Rule {
        files: r.files,
        matcher,
        edits,
        normaliser,
    })
}

/// `command_line` returns why the arguments `args` of a rule's `command`
/// are refused, as same-same would refuse to run them: `{files}` inside
/// another argument, both `{file}` and `{files}`, or `{settings}`, for which
/// pakf gives no settings file.
fn command_line(args: &[String]) -> std::result::Result<(), String> {
    if args.iter().any(|a| a != "{files}" && a.contains("{files}")) {
        return Err(
            "`{files}` of `command` stands inside another argument; give it as an argument of its own"
                .into(),
        );
    }
    let file = args.iter().any(|a| a.contains("{file}"));
    if file && args.iter().any(|a| a == "{files}") {
        return Err("`command` holds both `{file}` and `{files}`; give one of them".into());
    }
    if args.iter().any(|a| a.contains("{settings}")) {
        return Err("`command` names `{settings}`, which pakf doesn't fill; name the command's own configuration file instead".into());
    }
    Ok(())
}

/// `repository_path` returns the path `config` of a formatter's
/// configuration file from the repository root, without a leading `/`, as
/// pakf writes a path of the repository, or why it is refused: a path with a
/// `..` component or a drive, which leaves the repository.
fn repository_path(config: &str) -> std::result::Result<String, String> {
    let path = config.trim_start_matches('/');
    let leaves = path.is_empty()
        || Path::new(path).is_absolute()
        || path.contains(':')
        || path.split(['/', '\\']).any(|c| c == "..");
    if leaves {
        return Err(format!(
            "`config` names `{config}`, which leaves the repository; give its path from the repository root"
        ));
    }
    Ok(path.to_string())
}

/// `named` returns the set of the named things of `names`, or why one is
/// refused: a name that same-same doesn't give, or `line-endings`, which
/// pakf reads before the library, so that the library never reports it.
fn named(names: &[String]) -> std::result::Result<NamedSet, String> {
    let mut set = NamedSet::new();
    for name in names {
        match Named::from_name(name) {
            Some(n) if n != Named::LineEndings => set.insert(n),
            _ => {
                return Err(format!(
                    "`edits` names `{name}`, {}",
                    expected(
                        Named::ALL
                            .iter()
                            .filter(|n| **n != Named::LineEndings)
                            .map(|n| n.name())
                    )
                ));
            }
        }
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `read` returns the configuration of the text `text`.
    fn read(text: &str) -> Result<Compare> {
        static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = crate::test_support::fresh_dir(&format!("compare-{n}"));
        std::fs::create_dir_all(dir.join(".pakf")).unwrap();
        std::fs::write(dir.join(".pakf/config.toml"), text).unwrap();
        let out = read_compare(&RepoRoot::new(dir.clone()));
        std::fs::remove_dir_all(&dir).unwrap();
        out
    }

    fn p(s: &str) -> RepoPath {
        RepoPath::parse(s).unwrap()
    }

    // sokf-tests: CONF-1_AC-7
    // sokf-tests: CONF-1_AC-12
    #[test]
    fn the_last_rule_that_matches_decides_each_key() {
        let c = read(
            r#"
[[pack]]
name = "@acme/p"
source = "x"

[compare]
detect = false

[[compare.rule]]
files = ["*.ts"]
edits = ["import-order"]

[[compare.rule]]
files = ["/src/"]
command = ["fmt", "{file}"]

[[compare.rule]]
files = ["/src/keep.ts"]
edits = []
"#,
        )
        .unwrap();
        assert!(!c.detect);
        assert_eq!(c.edits(&p("/lib/a.ts")), NamedSet::of(Named::ImportOrder));
        // The command of the second rule doesn't decide `edits`.
        assert_eq!(c.edits(&p("/src/a.ts")), NamedSet::of(Named::ImportOrder));
        assert_eq!(c.edits(&p("/src/keep.ts")), NamedSet::new());
        // A file that no rule with `edits` matches counts how a YAML 1.1
        // reader takes a value.
        assert_eq!(c.edits(&p("/a.yaml")), NamedSet::of(Named::Yaml11Value));
        assert!(c.rules[1].matches(&p("/src/deep/a.css")));
        assert!(!c.rules[1].matches(&p("/lib/src.css")));
        assert_eq!(
            c.rules[1].normaliser,
            Some(LayOut::Command {
                argv: vec!["fmt".into(), "{file}".into()],
                overrides: false
            })
        );
        // Detection is off unless the project turns it on.
        let none = read("[[pack]]\nname = \"@acme/p\"\nsource = \"x\"\n").unwrap();
        assert!(!none.detect && none.rules.is_empty());
        assert!(read("[compare]\ndetect = true\n").unwrap().detect);
    }

    // sokf-tests: CONF-1_AC-13
    #[test]
    fn a_formatter_rule_names_its_formatter_and_settings() {
        let c = read(
            "[[compare.rule]]\nfiles = [\"*.css\"]\nformatter = \"prettier\"\nconfig = \"/.prettierrc.json\"\nsettings = \"as-written\"\nplugins = [\"prettier-plugin-x\"]\noverride = true\n",
        )
        .unwrap();
        assert_eq!(
            c.rules[0].normaliser,
            Some(LayOut::Formatter {
                formatter: KnownFormatter::Prettier,
                config: Some(".prettierrc.json".into()),
                settings: Settings::AsWritten,
                plugins: vec!["prettier-plugin-x".into()],
                overrides: true,
            })
        );
        let c = read("[[compare.rule]]\nfiles = [\"*.css\"]\nformatter = \"biome\"\n").unwrap();
        assert!(matches!(
            c.rules[0].normaliser,
            Some(LayOut::Formatter {
                settings: Settings::InternalWithOverrides,
                overrides: false,
                ..
            })
        ));
    }

    // sokf-tests: CONF-1_AC-11
    #[test]
    fn a_broken_table_is_refused_with_its_key() {
        for (text, says) in [
            ("[compare]\nlayout = 1\n", "layout"),
            ("[compare]\ndetect = \"yes\"\n", "detect"),
            ("compare = 1\n", "not a table"),
            ("[compare]\nrule = 1\n", "not a list"),
            ("[[compare.rule]]\nedits = []\n", "files"),
            ("[[compare.rule]]\nfiles = [\"*\"]\n", "none of"),
            (
                "[[compare.rule]]\nfiles = [\"*\"]\nedits = []\nwhat = 1\n",
                "what",
            ),
            (
                "[[compare.rule]]\nfiles = [\"*\"]\nedits = []\noverride = true\n",
                "`override` needs",
            ),
            (
                "[[compare.rule]]\nfiles = [\"*\"]\nedits = [\"line-endings\"]\n",
                "line-endings",
            ),
            (
                "[[compare.rule]]\nfiles = [\"*\"]\nedits = [\"key-orders\"]\n",
                "key-orders",
            ),
            (
                "[[compare.rule]]\nfiles = [\"*\"]\ncommand = [\"x\"]\nformatter = \"oxfmt\"\n",
                "both",
            ),
            (
                "[[compare.rule]]\nfiles = [\"*\"]\ncommand = [\"x\"]\nconfig = \"a.json\"\n",
                "`config` needs",
            ),
            (
                "[[compare.rule]]\nfiles = [\"*\"]\nformatter = \"rustfmt\"\n",
                "rustfmt",
            ),
            (
                "[[compare.rule]]\nfiles = [\"*\"]\nformatter = \"oxfmt\"\nconfig = \"../x.json\"\n",
                "../x.json",
            ),
            (
                "[[compare.rule]]\nfiles = [\"*\"]\nformatter = \"oxfmt\"\nsettings = \"mine\"\n",
                "mine",
            ),
            (
                "[[compare.rule]]\nfiles = [\"*\"]\nformatter = \"oxfmt\"\nplugins = [\"a\"]\n",
                "`plugins` needs `formatter = \"prettier\"`, and the rule names `oxfmt`",
            ),
            (
                "[[compare.rule]]\nfiles = [\"*\"]\ncommand = []\n",
                "no program",
            ),
            (
                "[[compare.rule]]\nfiles = [\"*\"]\ncommand = [\"fmt\", \"--in={files}\"]\n",
                "inside another argument",
            ),
            (
                "[[compare.rule]]\nfiles = [\"*\"]\ncommand = [\"fmt\", \"{file}\", \"{files}\"]\n",
                "both",
            ),
            (
                "[[compare.rule]]\nfiles = [\"*\"]\ncommand = [\"fmt\", \"-c\", \"{settings}\", \"{files}\"]\n",
                "{settings}",
            ),
            (
                "[[compare.rule]]\nfiles = [\"*\"]\ncommand = \"fmt {file}\"\n",
                "command",
            ),
        ] {
            let e = read(text).unwrap_err().to_string();
            assert!(e.contains(says), "{text}: {e}");
            assert!(e.starts_with("/.pakf/config.toml: "), "{e}");
        }
        // A value of a known set names the set.
        for (text, says) in [
            (
                "[compare]\nfoo = 1\n",
                "unknown key `foo`, expected `detect` or `rule`",
            ),
            (
                "[[compare.rule]]\nfiles = [\"*\"]\nformatter = \"rustfmt\"\n",
                "`formatter` names `rustfmt`, expected one of `oxfmt`, `prettier`, `biome`, `dprint`",
            ),
            (
                "[[compare.rule]]\nfiles = [\"*\"]\nformatter = \"oxfmt\"\nsettings = \"mine\"\n",
                "`settings` names `mine`, expected one of `as-written`, `internal-with-overrides`, `internal-only`",
            ),
            (
                "[[compare.rule]]\nfiles = [\"*\"]\nedits = [\"line-endings\"]\n",
                "`edits` names `line-endings`, expected one of `key-order`,",
            ),
        ] {
            let e = read(text).unwrap_err().to_string();
            assert!(e.contains(says), "{text}: {e}");
        }
        let e = read(
            "[[compare.rule]]\nfiles = [\"*\"]\nedits = []\n[[compare.rule]]\nfiles = [\"*\"]\n",
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("number 2"), "{e}");
    }
}
