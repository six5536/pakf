//! This module compares the texts of a pack's files by their meaning,
//! through the shared library `same-same`, so that a formatter's change of
//! layout is no edit (the decision "Comparing by meaning through same-same"
//! of design-pack). Each command opens one `Meaning`, which reads `[compare]`
//! of `/.pakf/config.toml`, and builds the library's comparer only at its
//! first comparison, so that a command whose files hold their bases parses
//! no text and starts no program. pakf reads each text first as the module
//! `eol` does, with each CRLF as LF, and asks the library only where the
//! bytes still differ. A command that writes the project's files or the lock
//! compares by the library's own reading alone, so that every machine writes
//! the same (the decision "One answer on every machine" of design-pack).

use std::{
    cell::{OnceCell, RefCell},
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    str::FromStr,
    time::Instant,
};

use same_same::{
    Cache, Command, Comparer, ConfiguredFormatter, Detail, Detect, Fingerprint, Format, Input,
    Method, NamedSet, NormaliserId, Note, Unavailable, Verdict,
};

use super::eol::{is_binary, normalise, same_text};
use crate::{
    RepoPath, RepoRoot, Result,
    config::{self, Compare, LayOut, TIMEOUT},
};

/// `CACHE_DIR` is the folder, relative to the repository root, of pakf's
/// caches, which its own `.gitignore` keeps out of git.
pub const CACHE_DIR: &str = ".pakf/cache";

/// `FORMATS` are the formats that pakf compares by meaning, and reads in a
/// fence of Markdown: a fixed list, so that the rules tags of the lock's
/// fingerprints change only where the library's rules change, and not where
/// a later release of the library adds a format.
const FORMATS: [Format; 10] = [
    Format::Markdown,
    Format::Json,
    Format::Jsonc,
    Format::Toml,
    Format::Yaml,
    Format::JavaScript,
    Format::Jsx,
    Format::TypeScript,
    Format::Tsx,
    Format::TypeScriptDeclaration,
];

/// `LIBRARY_CACHE` is the folder of the library's cache in `CACHE_DIR`.
const LIBRARY_CACHE: &str = "same-same";

/// A `For` is what a command does with the answers of its comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum For {
    /// It reports them, as `pakf status`, `pakf ls`, `pakf diff` and
    /// `pakf patch` do: the formatters and the commands of the rules, and
    /// detection where `[compare]` turns it on, compare a file too, so that a
    /// machine that runs them is spared a report.
    Reports,
    /// It writes by them the project's files or the lock, as `pakf sync`,
    /// `pakf add`, `pakf remove` and `pakf accept` do: only the library's own
    /// reading compares a file, with the `edits` of the rules, so that every
    /// machine writes the same files and the same lock, and the command
    /// starts no program.
    Writes,
}

/// A `Meaning` is the comparison of the texts of one command: the
/// configuration of `[compare]`, the library's comparers, and its cache.
pub struct Meaning {
    /// The repository root, where detection looks for the project's
    /// formatter and a rule's formatter finds its program and its
    /// configuration file.
    root: PathBuf,
    /// The configuration.
    compare: Compare,
    /// What the command does with the answers.
    purpose: For,
    /// Whether the cache lives in `CACHE_DIR`, or in memory alone.
    folder: bool,
    /// The cache, opened at the first comparison.
    cache: OnceCell<Cache>,
    /// The comparer of a command that reports, with the rules' formatters
    /// and commands and with detection where it is on.
    full: OnceCell<Comparer>,
    /// The comparer of a command that writes and of the fingerprints of the
    /// lock, without any formatter, command, or detection, so that the
    /// files and the lock are the same on every machine.
    plain: OnceCell<Comparer>,
    /// The comparer of a command that reports, for a file whose rule's
    /// formatter or command ran past its limit earlier in the command: with
    /// detection where it is on, and without the rules.
    detected: OnceCell<Comparer>,
    /// The comparer of a command that reports, for a file whose rule's
    /// formatter or command hasn't run past its limit, once the detected
    /// formatter has: with the rules, and without detection.
    undetected: OnceCell<Comparer>,
    /// The name and the settings by which the library knows the formatter
    /// or the command of each rule, in the order of the rules, or none for
    /// a rule that names neither.
    programs: OnceCell<Vec<Option<Known>>>,
    /// The patterns of the file names that the detected formatter takes,
    /// such as `*.css`, or none where detection finds no formatter.
    detected_names: OnceCell<Vec<String>>,
    /// The files on which each formatter or command of the project's
    /// configuration was missing, failed or ran past its limit, by the
    /// warning that names the formatter and what happened.
    faults: RefCell<BTreeMap<String, BTreeSet<String>>>,
    /// Each formatter or command that ran past its limit in the command, by
    /// the number of its rule, or none for the detected formatter, with the
    /// warning of `faults` that names it, as `gather` remembers it.
    hung: RefCell<BTreeMap<Option<usize>, String>>,
}

/// A `Known` is the name and the settings by which the library knows a
/// formatter or a command, as the identity of each of its notes gives them.
type Known = (String, Option<String>);

/// A `Reading` is pakf's reading of the library's answer for two texts, as
/// the decision "Comparing by meaning through same-same" of design-pack
/// gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Reading {
    /// The texts mean the same, apart from named things that the
    /// configuration doesn't count as an edit.
    Same,
    /// They differ in meaning, or in a named thing that counts as an edit.
    Edited,
    /// They were compared by their bytes, which differ.
    Bytes,
}

impl Meaning {
    /// `open` returns the comparison of a command in the repository `root`
    /// that does `purpose` with its answers, with the configuration that
    /// `[compare]` of `/.pakf/config.toml` gives, which it refuses as
    /// `read_compare` says.
    // sokf-implements: CONF-1_AC-11
    pub fn open(root: &RepoRoot, purpose: For) -> Result<Meaning> {
        Ok(Meaning::new(
            root.as_path().to_path_buf(),
            config::read_compare(root)?,
            purpose,
            true,
        ))
    }

    /// `in_memory` returns a comparison for `purpose` with the
    /// configuration `compare`, from the repository root `root`, with its
    /// cache in memory alone.
    pub fn in_memory(root: &Path, compare: Compare, purpose: For) -> Meaning {
        Meaning::new(root.to_path_buf(), compare, purpose, false)
    }

    fn new(root: PathBuf, compare: Compare, purpose: For, folder: bool) -> Meaning {
        Meaning {
            root,
            compare,
            purpose,
            folder,
            cache: OnceCell::new(),
            full: OnceCell::new(),
            plain: OnceCell::new(),
            detected: OnceCell::new(),
            undetected: OnceCell::new(),
            programs: OnceCell::new(),
            detected_names: OnceCell::new(),
            faults: RefCell::new(BTreeMap::new()),
            hung: RefCell::new(BTreeMap::new()),
        }
    }

    /// `unedited` returns whether the copy `copy` of the file at `dest`
    /// means the same as its base copy, which lies at `base_copy`, as the
    /// test of an edit reads it: the same, or the same apart from named
    /// things that the configuration doesn't count as an edit. A base copy
    /// that can't be read leaves the file compared by its bytes.
    // sokf-implements: CLI-1_AC-34
    // sokf-implements: CLI-1_AC-35
    pub fn unedited(&self, dest: &RepoPath, copy: &[u8], base_copy: &Path) -> bool {
        fs::read(base_copy).is_ok_and(|base| self.read(dest, copy, &base) == Reading::Same)
    }

    /// `same` returns whether the texts `a` and `b` of the file at `dest`
    /// mean the same, apart from named things that the configuration
    /// doesn't count as an edit, for the sync's tests of a change.
    pub fn same(&self, dest: &RepoPath, a: &[u8], b: &[u8]) -> bool {
        self.read(dest, a, b) == Reading::Same
    }

    /// `same_as_file` returns whether the text `a` of the file at `dest`
    /// means the same as the file at `path`, as `same` says, and false
    /// where that file can't be read.
    pub fn same_as_file(&self, dest: &RepoPath, a: &[u8], path: &Path) -> bool {
        fs::read(path).is_ok_and(|b| self.same(dest, a, &b))
    }

    /// `read` returns the reading of the texts `a` and `b` of the file at
    /// `dest`. Texts that hold the same bytes with each CRLF read as LF are
    /// the same, with no call of the library. A binary text, and a text
    /// that holds a conflict, compare by their bytes without the library.
    /// The library reads both texts under the name of the file's
    /// destination, through the formatters, the commands and the detection
    /// of `[compare]` only for a command that reports.
    // sokf-implements: CLI-1_AC-36
    // sokf-implements: CLI-1_AC-39
    // sokf-implements: CLI-11_AC-1
    fn read(&self, dest: &RepoPath, a: &[u8], b: &[u8]) -> Reading {
        if same_text(a, b) {
            return Reading::Same;
        }
        if is_binary(a) || is_binary(b) || holds_conflict(dest, a) || holds_conflict(dest, b) {
            return Reading::Bytes;
        }
        let (a, b) = (normalise(a), normalise(b));
        let name = dest.as_str();
        let (comparer, skipped) = match self.purpose {
            For::Reports => self.reporting(dest),
            For::Writes => (self.plain(), Vec::new()),
        };
        let started = Instant::now();
        let answer = comparer.compare(
            &Input::bytes(&a, name),
            &Input::bytes(&b, name),
            Detail::Verdict,
        );
        // A hang that the library's cache kept for a text costs no time, and
        // a program that ran past its limit in this comparison took it all.
        // sokf-implements: CONF-1_AC-15
        self.gather(dest, &answer.notes, started.elapsed() >= TIMEOUT);
        for (program, fault) in skipped {
            if self.would_lay_out(dest, program, &answer.method, &answer.notes) {
                self.faults
                    .borrow_mut()
                    .entry(fault)
                    .or_default()
                    .insert(dest.to_string());
            }
        }
        match answer.verdict {
            Verdict::Same => Reading::Same,
            Verdict::SameApartFrom(named) if !counts(named, self.compare.edits(dest)) => {
                Reading::Same
            }
            Verdict::SameApartFrom(_) | Verdict::Different => Reading::Edited,
            _ => Reading::Bytes,
        }
    }

    /// `reporting` returns the comparer of a command that reports for the
    /// file at `dest`: the full one, or, where the formatter or the command
    /// of the file's rule, or the detected formatter, already ran past its
    /// limit in the command, one without that program, which answers as the
    /// library does once the program has given no answer, so that one
    /// program that hangs costs its limit in one comparison of the command
    /// alone. It also returns each program that the comparer leaves out, by
    /// the number of its rule, or none for the detected formatter, with the
    /// warning that names it.
    // sokf-implements: CONF-1_AC-15
    fn reporting(&self, dest: &RepoPath) -> (&Comparer, Vec<(Option<usize>, String)>) {
        let hung = self.hung.borrow();
        let rule = self
            .compare
            .laid_out_by(dest)
            .and_then(|(n, _)| Some((Some(n), hung.get(&Some(n))?.clone())));
        let detection = hung.get(&None).map(|fault| (None, fault.clone()));
        // The library tries the rule's program, and the detected formatter
        // once that program has given no answer, so a comparer without
        // one of them answers as the library does after it hung.
        let comparer = match (&rule, &detection) {
            (None, None) => self.full(),
            (None, Some(_)) => self.undetected(),
            (Some(_), None) if self.compare.detect => self.detected(),
            (Some(_), _) => self.plain(),
        };
        (comparer, rule.into_iter().chain(detection).collect())
    }

    /// `would_lay_out` returns whether the program `program` that a
    /// comparison of the file at `dest` left out, the formatter or the
    /// command of the rule by its number or else the detected formatter,
    /// would have been given the file, by the method `method` and the notes
    /// `notes` of the library's answer without it: a text that the library
    /// reads, and that it didn't read by meaning or whose rule puts the
    /// program first; for the detected formatter, one that no rule's
    /// program laid out either, of a name that the formatter takes.
    fn would_lay_out(
        &self,
        dest: &RepoPath,
        program: Option<usize>,
        method: &Method,
        notes: &[Note],
    ) -> bool {
        let read = !notes
            .iter()
            .any(|note| matches!(note, Note::NotUtf8 { .. } | Note::Binary { .. }));
        match program {
            Some(rule) => {
                read && (!matches!(method, Method::Meaning { .. }) || self.overrides(rule))
            }
            None => read && *method == Method::Bytes && self.detected_takes(dest),
        }
    }

    /// `overrides` returns whether the rule number `rule`, from 1, puts its
    /// formatter or command before the comparison by meaning.
    fn overrides(&self, rule: usize) -> bool {
        self.compare
            .rules
            .get(rule - 1)
            .and_then(|r| r.normaliser.as_ref())
            .is_some_and(|l| match l {
                LayOut::Formatter { overrides, .. } | LayOut::Command { overrides, .. } => {
                    *overrides
                }
            })
    }

    /// `detected_takes` returns whether the detected formatter takes the
    /// file at `dest`, by the ending of its name.
    fn detected_takes(&self, dest: &RepoPath) -> bool {
        let names = self.detected_names.get_or_init(|| {
            same_same::detected(&self.root)
                .map(|found| found.names)
                .unwrap_or_default()
        });
        let file = dest.as_str().rsplit('/').next().unwrap_or_default();
        file.rsplit_once('.').is_some_and(|(_, ending)| {
            names
                .iter()
                .any(|name| name.strip_prefix("*.") == Some(ending))
        })
    }

    /// `program_of` returns the rule, by its number from 1 and the name of
    /// its program, whose formatter or command gave a note with the
    /// identity `id` on the file at `dest`: the last rule that matches the
    /// file and whose program the library knows by that name and those
    /// settings, or none for the detected formatter. Without detection, a
    /// note that names no rule's program is the program's of the rule that
    /// lays the file out.
    fn program_of(&self, dest: &RepoPath, id: &NormaliserId) -> Option<(usize, String)> {
        let programs = self.programs.get_or_init(|| {
            let mut known = self.full().normalisers().into_iter();
            self.compare
                .rules
                .iter()
                .map(|r| {
                    r.normaliser.as_ref()?;
                    known.next().map(|(id, _)| (id.name, id.settings))
                })
                .collect()
        });
        let named = self
            .compare
            .rules
            .iter()
            .zip(programs)
            .enumerate()
            .rev()
            .filter(|(_, (rule, _))| rule.matches(dest))
            .find_map(|(i, (rule, program))| {
                let (name, settings) = program.as_ref()?;
                if *name == id.name && *settings == id.settings {
                    Some((i + 1, rule.program()?))
                } else {
                    None
                }
            });
        named.or_else(|| {
            if self.compare.detect {
                None
            } else {
                self.compare.laid_out_by(dest)
            }
        })
    }

    /// `gather` records each formatter or command of the project's
    /// configuration that the notes `notes` of the file at `dest` report as
    /// missing, failed, or past its limit, for the warnings of `finish`, by
    /// the identity that each note gives. Where the comparison took the
    /// limit, `waited`, it remembers in `hung` a program that ran past its
    /// limit, or whose samples failed, as they do where it hangs on every
    /// text.
    // sokf-implements: CLI-1_AC-44
    fn gather(&self, dest: &RepoPath, notes: &[Note], waited: bool) {
        for note in notes {
            let Note::NormaliserUnavailable { id, why } = note else {
                continue;
            };
            let rule = self.program_of(dest, id);
            let who = match &rule {
                Some((n, name)) => format!("`{name}` of [[compare.rule]] number {n}"),
                None => format!("the detected formatter `{}`", id.name),
            };
            let (what, files) = match why {
                Unavailable::Missing { .. } => ("is missing".to_string(), false),
                Unavailable::DoesntStart { why } => (format!("doesn't start: {why}"), false),
                Unavailable::Hung { limit } => {
                    (format!("ran past {} seconds on", limit.as_secs()), true)
                }
                _ => ("failed on".to_string(), true),
            };
            let fault = format!("{who} {what}");
            if waited && matches!(why, Unavailable::Hung { .. } | Unavailable::SampleFailed) {
                self.hung
                    .borrow_mut()
                    .insert(rule.map(|(n, _)| n), fault.clone());
            }
            let mut faults = self.faults.borrow_mut();
            let seen = faults.entry(fault).or_default();
            if files {
                seen.insert(dest.to_string());
            }
        }
    }

    /// `fingerprint` returns the fingerprint that the lock records for an
    /// accepted version `bytes` of the file at `dest`, in its text form:
    /// that of its meaning, with each CRLF read as LF, by the library's
    /// own comparison, never through a formatter or a command, or the
    /// plain hash of a binary file.
    // sokf-implements: CONF-2_AC-6
    pub fn fingerprint(&self, dest: &RepoPath, bytes: &[u8]) -> String {
        self.fingerprint_of(dest, bytes).to_string()
    }

    fn fingerprint_of(&self, dest: &RepoPath, bytes: &[u8]) -> Fingerprint {
        // A file that isn't UTF-8 and holds no NUL byte reads its CRLF as
        // LF too, as `normalise` reads it.
        if is_binary(bytes) {
            return Fingerprint::of_bytes(&normalise(bytes));
        }
        let text = normalise(bytes);
        self.plain()
            .fingerprint(&Input::bytes(&text, dest.as_str()))
    }

    /// `holds_fingerprint` returns whether the version `bytes` of the file
    /// at `dest` means what the fingerprint `recorded` records: the same,
    /// or the same apart from named things that the configuration doesn't
    /// count as an edit. A value of another form, such as an `fnv1a64:`
    /// hash of an older lock, holds no version.
    // sokf-implements: CLI-1_AC-27
    pub fn holds_fingerprint(&self, dest: &RepoPath, bytes: &[u8], recorded: &str) -> bool {
        let Ok(recorded) = Fingerprint::from_str(recorded) else {
            return false;
        };
        match recorded.compare(&self.fingerprint_of(dest, bytes)) {
            Verdict::Same => true,
            Verdict::SameApartFrom(named) => !counts(named, self.compare.edits(dest)),
            _ => false,
        }
    }

    /// `finish` writes the cache's new entries, where it can, and returns a
    /// warning for each formatter or command of the project's configuration
    /// that was missing, failed, or ran past its limit, with the files where
    /// it failed or ran past its limit.
    pub fn finish(&self) -> Vec<String> {
        if let Some(cache) = self.cache.get() {
            // A cache that can't be written costs only time.
            let _ = cache.flush();
        }
        self.faults
            .borrow()
            .iter()
            .map(|(fault, files)| {
                if files.is_empty() {
                    fault.clone()
                } else {
                    let files: Vec<&str> = files.iter().map(String::as_str).collect();
                    format!("{fault} {}", and_list(&files))
                }
            })
            .collect()
    }

    /// `settle` finishes the comparison, as `finish` does, with the result
    /// of the command that used it: the warnings go into the report through
    /// `put`, or with the error, so that a command that fails after it
    /// compared still prints them.
    // sokf-implements: CLI-1_AC-45
    pub(crate) fn settle<T>(
        &self,
        result: Result<T>,
        put: impl FnOnce(&mut T, Vec<String>),
    ) -> Result<T> {
        let warnings = self.finish();
        match result {
            Ok(mut report) => {
                put(&mut report, warnings);
                Ok(report)
            }
            Err(e) => Err(e.warned(warnings)),
        }
    }

    /// `cache` returns the cache of the command: the library's folder in
    /// `/.pakf/cache/same-same/`, beside the `.gitignore` that keeps
    /// `/.pakf/cache/` out of git, or the memory where the folder or its
    /// `.gitignore` can't be written or a link stands on the way, as
    /// `cache_folder` says.
    // sokf-implements: CLI-1_AC-38
    fn cache(&self) -> &Cache {
        self.cache.get_or_init(
            || match self.folder.then(|| cache_folder(&self.root)).flatten() {
                Some(dir) => Cache::folder(&dir),
                None => Cache::memory(),
            },
        )
    }

    /// `full` returns the comparer of a command that reports: the formats
    /// of `FORMATS`, the formatters and the commands of the rules in their
    /// order, and the detection of the project's formatter where
    /// `detect = true` turns it on, because a clone's program runs only by
    /// the project's stated choice.
    fn full(&self) -> &Comparer {
        self.full
            .get_or_init(|| self.build(true, self.compare.detect))
    }

    /// `detected` returns the comparer of `reporting` for a file whose
    /// rule's formatter or command ran past its limit: the formats of
    /// `FORMATS` and the detection of the project's formatter.
    fn detected(&self) -> &Comparer {
        self.detected.get_or_init(|| self.build(false, true))
    }

    /// `undetected` returns the comparer of `reporting` for a file whose
    /// rule's formatter or command hasn't run past its limit, once the
    /// detected formatter has: the formats of `FORMATS` and the rules.
    fn undetected(&self) -> &Comparer {
        self.undetected.get_or_init(|| self.build(true, false))
    }

    /// `build` returns a comparer of the formats of `FORMATS`, with the
    /// formatters and the commands of the rules in their order where
    /// `rules` says so, and the detection of the project's formatter where
    /// `detect` says so.
    // sokf-implements: CONF-1_AC-9
    // sokf-implements: CONF-1_AC-10
    // sokf-implements: CONF-1_AC-13
    // sokf-implements: CONF-1_AC-14
    fn build(&self, rules: bool, detect: bool) -> Comparer {
        let mut builder = Comparer::builder()
            .formats(FORMATS)
            .cache(self.cache().clone());
        for rule in self.compare.rules.iter().filter(|_| rules) {
            let files: Vec<String> = rule
                .files
                .iter()
                .map(|p| super::literal_braces(p))
                .collect();
            builder = match &rule.normaliser {
                Some(LayOut::Formatter {
                    formatter,
                    config,
                    settings,
                    plugins,
                    overrides,
                }) => builder.formatter(ConfiguredFormatter {
                    formatter: *formatter,
                    files,
                    overrides: *overrides,
                    root: self.root.clone(),
                    program: None,
                    config: config.as_ref().map(PathBuf::from),
                    settings: *settings,
                    plugins: plugins.clone(),
                    timeout: TIMEOUT,
                }),
                Some(LayOut::Command { argv, overrides }) => builder.command(Command {
                    argv: from_root(&self.root, argv),
                    files,
                    overrides: *overrides,
                    timeout: TIMEOUT,
                    version_argv: None,
                    settings: None,
                }),
                None => builder,
            };
        }
        if detect {
            builder = builder.detect(Detect {
                root: self.root.clone(),
                timeout: TIMEOUT,
            });
        }
        builder
            .build()
            .expect("pakf compiles every format of the library")
    }

    /// `plain` returns the comparer of a command that writes and of the
    /// lock's fingerprints: the formats of `FORMATS`, and no formatter,
    /// command, or detection.
    fn plain(&self) -> &Comparer {
        self.plain.get_or_init(|| self.build(false, false))
    }
}

/// `cache_folder` returns the library's folder of the cache in the
/// repository `root`, `/.pakf/cache/same-same/`, which it creates with the
/// `.gitignore` of `/.pakf/cache/` where they are absent, or none where
/// `/.pakf`, `/.pakf/cache/`, its `.gitignore`, the library's folder, or an
/// entry below that folder is a symbolic link or not of its kind, or where
/// one can't be written, so that a command writes nothing through a link
/// that the repository holds.
// sokf-implements: CLI-1_AC-38
// sokf-implements: CLI-1_AC-42
fn cache_folder(root: &Path) -> Option<PathBuf> {
    let pakf = root.join(".pakf");
    if fs::symlink_metadata(&pakf).is_err() {
        fs::create_dir(&pakf).ok()?;
    }
    if !is_real_dir(&pakf) {
        return None;
    }
    let dir = root.join(CACHE_DIR);
    if !is_real_dir(&dir) {
        // A link or a file at the path makes the creation fail.
        fs::create_dir(&dir).ok()?;
    }
    let ignore = dir.join(".gitignore");
    match fs::symlink_metadata(&ignore) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => return None,
        Err(_) => {
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&ignore)
                .and_then(|mut file| file.write_all(b"*\n"))
                .ok()?;
        }
    }
    let library = dir.join(LIBRARY_CACHE);
    if !is_real_dir(&library) {
        fs::create_dir(&library).ok()?;
    }
    (!holds_link(&library, 3)).then_some(library)
}

/// `is_real_dir` returns whether a directory, and no link, stands at `path`.
fn is_real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir())
}

/// `holds_link` returns whether a symbolic link stands below the directory
/// `dir`, down to `depth` levels, or an entry there can't be read.
fn holds_link(dir: &Path, depth: usize) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return true;
    };
    entries.into_iter().any(|entry| {
        let Ok(entry) = entry else {
            return true;
        };
        let Ok(kind) = entry.file_type() else {
            return true;
        };
        kind.is_symlink() || (kind.is_dir() && depth > 1 && holds_link(&entry.path(), depth - 1))
    })
}

/// `from_root` returns the command line `argv` of a rule with its program
/// read from the repository root `root` where its name holds a `/` or a
/// `\` and isn't absolute, as a rule's `config` is read, so that a command
/// runs the same program from every directory.
// sokf-implements: CONF-1_AC-9
fn from_root(root: &Path, argv: &[String]) -> Vec<String> {
    let mut argv = argv.to_vec();
    if let Some(program) = argv.first_mut()
        && program.contains(['/', '\\'])
        && Path::new(program.as_str()).is_relative()
    {
        *program = root.join(program.as_str()).to_string_lossy().into_owned();
    }
    argv
}

/// `counts` returns whether the named things `named` hold one of `edits`,
/// the named things that count as an edit.
fn counts(named: NamedSet, edits: NamedSet) -> bool {
    named.iter().any(|n| edits.contains(n))
}

/// `and_list` returns `items` joined with commas and a last `and`.
fn and_list(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// `holds_conflict` returns whether `bytes`, the text of the file at `dest`,
/// hold a conflict that a merge left: a line that begins with `<<<<<<< `,
/// other than an example in a fence of Markdown, or the line that opens a
/// conflict of pakf's merge anywhere, because the comparison has no record
/// to tell where a sync left one, and a reading by bytes at most reports an
/// edit, where a reading by meaning could hide the conflict.
fn holds_conflict(dest: &RepoPath, bytes: &[u8]) -> bool {
    !crate::conflicts::conflict_lines(dest, &String::from_utf8_lossy(bytes), Some("")).is_empty()
}

#[cfg(test)]
mod tests;
