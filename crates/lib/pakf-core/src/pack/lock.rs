//! This module reads and writes `/.pakf/lock.toml`, the record of each
//! schema pack that `pakf sync` copied into the repository.

use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};

use super::{
    dest::{Dest, read_dest},
    eol::is_base,
    meaning::Meaning,
};
use crate::{Error, RepoPath, RepoRoot, Result, source::read_optional};

/// `HEADER` is the comment at the top of the lock.
const HEADER: &str = "# pakf sync writes this file. Do not edit it.\n";

/// `LOCK_VERSION` is the version of the lock's format that this tool reads
/// and writes. A lock without `lock-version` has the version 1, the version
/// 2 adds `editable` to the record of a file, and the version 3 records the
/// values of `same-same` in place of pakf's own `fnv1a64:` hashes. This
/// tool reads a lock of each version alike, where an `fnv1a64:` value of an
/// older lock equals no value that it computes (the decision "Fingerprints
/// in the lock" of design-pack).
pub const LOCK_VERSION: u32 = 3;

/// `lock_path` returns the path of the lock.
pub fn lock_path() -> RepoPath {
    RepoPath::parse("/.pakf/lock.toml").expect("literal")
}

/// `PACKS_DIR` is the directory, relative to the repository root, that holds
/// a directory for each pack: the copy of its instructions and its base
/// copies.
pub const PACKS_DIR: &str = ".pakf/packs";

/// `pack_dir` returns the directory of the pack `name` under `/.pakf/packs/`:
/// `/.pakf/packs/@<scope>/<name>/` for the id `@<scope>/<name>`.
// sokf-implements: CLI-1_AC-29
pub fn pack_dir(name: &str) -> RepoPath {
    RepoPath::parse(&format!("/{PACKS_DIR}/{name}")).expect("a pack name is an id or a slug")
}

/// `base_path` returns the path of the base copy of the file `rel` of the
/// pack `name`.
pub fn base_path(name: &str, rel: &str) -> RepoPath {
    in_pack_dir(name, &format!("base/{rel}"))
}

/// `instructions_of` returns the path of the copy of the instructions
/// fragment of the pack `name`.
pub fn instructions_of(name: &str) -> RepoPath {
    in_pack_dir(name, "instructions.md")
}

/// `in_pack_dir` returns the path `rel` under the directory of the pack
/// `name`.
fn in_pack_dir(name: &str, rel: &str) -> RepoPath {
    RepoPath::parse(&format!("{}/{rel}", pack_dir(name))).expect("a pack path is relative")
}

/// `ordered` returns the names of the packs of `lock` in the order of the
/// packs: the packs that the configuration declares, sorted by their
/// dependencies and their entries, then any other pack by its name.
/// Dependencies that form a cycle leave the declared packs in the order of
/// their entries, and a configuration that doesn't parse leaves every pack
/// in the order of its name, because a sync refuses both before it writes.
pub(crate) fn ordered(root: &RepoRoot, lock: &Lock) -> Vec<String> {
    let mut list: Vec<String> = Vec::new();
    for n in crate::config::declared_order(root) {
        // Two entries of one name lead to one record, which comes once.
        if lock.packs.contains_key(&n) && !list.contains(&n) {
            list.push(n);
        }
    }
    let depends = lock
        .packs
        .iter()
        .map(|(n, p)| (n.clone(), p.depends.keys().cloned().collect()))
        .collect();
    let mut out = super::order::precedence(&list, &depends).unwrap_or(list);
    for name in lock.packs.keys() {
        if !out.contains(name) {
            out.push(name.clone());
        }
    }
    out
}

/// A `Lock` is the content of `/.pakf/lock.toml`: one entry for each pack,
/// by name.
// sokf-implements: CONF-2_AC-1
// sokf:begin lock
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lock {
    /// The version of the lock's format.
    #[serde(rename = "lock-version", default = "first_version")]
    pub lock_version: u32,
    /// The packs, by id.
    #[serde(default)]
    pub packs: BTreeMap<String, LockedPack>,
}

/// A `LockedPack` is the record of one pack.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedPack {
    /// The source: a git URL or path. A relative path resolves from the
    /// repository root.
    pub source: String,
    /// The branch, tag, or commit of a git source.
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    /// The pack's directory in the source, when it isn't the source's root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The resolved commit.
    pub commit: String,
    /// The version that the pack's manifest gives.
    pub version: String,
    /// The remaps of the paths of the pack's files that the `paths` of its
    /// entry gave at the last sync, from each prefix to its mapped prefix.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub paths: BTreeMap<String, String>,
    /// The packs that the pack depends on, each with its oldest version.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub depends: BTreeMap<String, String>,
    /// The pack's files, by their paths in the pack.
    #[serde(default)]
    pub files: BTreeMap<String, LockedFile>,
}

/// A `LockedFile` is the record of one file of a pack.
// sokf-implements: CONF-2_AC-2
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedFile {
    /// The file's path in the repository.
    pub path: String,
    /// The plain hash that `same-same` gives of the file's base, the pack's
    /// version of the file at the last sync, in its text form
    /// `ss1:bytes:<HEX>`.
    pub base: String,
    /// Whether the pack's version of a file of `files/` is executable.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub executable: bool,
    /// The pack higher in the order that owns the file's destination, so
    /// that the file holds that pack's version and not this one.
    #[serde(
        rename = "shadowed-by",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub shadowed_by: Option<String>,
    /// Whether the last sync changed this pack's version of a shadowed file.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub changed: bool,
    /// Whether the pack declares the file editable, so that its edit needs
    /// no acceptance while the pack owns the file.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub editable: bool,
    /// Whether the last sync left a conflict in the file, so that the line
    /// that opens it counts inside a fenced code block too.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub conflicted: bool,
    /// The edit or the deletion of the file that a maintainer accepted with
    /// `pakf accept`, so that `pakf status --check` passes on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted: Option<Accepted>,
}

/// An `Accepted` is the record of an accepted edit: the version of the file
/// that the project keeps on purpose, or its deletion, with the reason.
// sokf-implements: CONF-2_AC-6
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Accepted {
    /// The fingerprint that `same-same` gives of the accepted version's
    /// meaning, with each CRLF of a text read as LF, or its plain hash where
    /// the library doesn't compare it by meaning, in the library's text
    /// form, or none for an accepted deletion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    /// Whether the accepted version of a file of `files/` is executable.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub executable: bool,
    /// Whether the project deleted the file on purpose.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
    /// Why the project keeps the edit.
    pub note: String,
}
// sokf:end lock

impl LockedFile {
    /// `left_conflict` returns whether the last sync left a conflict in the
    /// file, as the record says, or the record comes from a lock older than
    /// the version 3, whose base is an `fnv1a64:` hash, because such a lock
    /// records no conflict.
    pub fn left_conflict(&self) -> bool {
        self.conflicted || self.base.parse::<same_same::Fingerprint>().is_err()
    }

    /// `holds_conflict` returns whether `bytes`, the text of the file `rel`
    /// of the pack `name`, whose record this is, hold a conflict, as the
    /// lock module's free function `holds_conflict` reads one with the
    /// record and the base copy.
    pub fn holds_conflict(&self, root: &RepoRoot, name: &str, rel: &str, bytes: &[u8]) -> bool {
        RepoPath::parse(&self.path).is_some_and(|dest| {
            let left = self.left_conflict().then(|| base_path(name, rel));
            holds_conflict(root, &dest, bytes, left.as_ref())
        })
    }

    /// `still_conflicted` returns whether a regular file stands at the
    /// file's path and holds a conflict, as `holds_conflict` reads one, so
    /// that a command that drops the record and keeps the file can say so.
    pub fn still_conflicted(&self, root: &RepoRoot, name: &str, rel: &str) -> bool {
        RepoPath::parse(&self.path).is_some_and(|dest| match read_dest(root, &dest) {
            Dest::File(bytes) => self.holds_conflict(root, name, rel, &bytes),
            _ => false,
        })
    }

    /// `holds_base` returns whether what stands at the file's path, `dest`,
    /// holds the file's base: a regular file whose text is the base, with
    /// each CRLF read as LF, or means the same as the base copy at
    /// `base_copy`, as `Meaning::unedited` reads it, and, for a file of
    /// `files/`, whose executable bit `bit` is the bit that the record
    /// holds. `bit` is none for any
    /// other file and on a platform without the bit, where no command
    /// compares it. An absent file and an entry that isn't a readable
    /// regular file hold no base. The test of an edit that `pakf status`,
    /// `pakf ls`, `pakf accept`, and `pakf remove` share, and sync's removal
    /// of a file that its pack no longer holds, ask this function, so that
    /// they read the same file alike with one comparison, which the `For` of
    /// the command's `Meaning` picks: a command that writes reads by the
    /// library's own reading alone, and one that reports through the rules'
    /// formatters and commands too. Sync's update of a file that its pack
    /// still holds reads the text against the base apart from the bit, which
    /// its plan of the bit decides on its own.
    // sokf-implements: CLI-1_AC-22
    // sokf-implements: CLI-1_AC-25
    pub fn holds_base(
        &self,
        dest: &Dest,
        bit: Option<bool>,
        base_copy: &Path,
        meaning: &Meaning,
    ) -> bool {
        let text = match dest {
            Dest::File(b) => {
                is_base(b, &self.base)
                    || RepoPath::parse(&self.path)
                        .is_some_and(|path| meaning.unedited(&path, b, base_copy))
            }
            _ => false,
        };
        text && bit.is_none_or(|x| x == self.executable)
    }
}

impl Accepted {
    /// `of` returns the acceptance of the file at `dest` that holds `bytes`,
    /// or of a deleted file when `bytes` is none, with the executable bit
    /// `bit` of a file of `files/`, and the fingerprint of the accepted
    /// version that `meaning` gives.
    pub fn of(
        bytes: Option<&[u8]>,
        bit: bool,
        note: &str,
        dest: &RepoPath,
        meaning: &Meaning,
    ) -> Accepted {
        Accepted {
            hash: bytes.map(|b| meaning.fingerprint(dest, b)),
            executable: bytes.is_some() && bit,
            deleted: bytes.is_none(),
            note: note.to_string(),
        }
    }

    /// `of_older_lock` returns whether the acceptance records a hash that
    /// no fingerprint of `same-same` reads, such as an `fnv1a64:` hash that
    /// pakf 0.2 recorded, which holds no version.
    pub fn of_older_lock(&self) -> bool {
        self.hash
            .as_deref()
            .is_some_and(|h| h.parse::<same_same::Fingerprint>().is_err())
    }

    /// `holds` returns whether what stands at the file's path `path`,
    /// `dest`, holds the accepted version: a regular file that means what
    /// the accepted fingerprint records, as `Meaning::holds_fingerprint`
    /// reads it, and, for a file of `files/`, the accepted executable bit
    /// `bit`, which
    /// is none for any other file, or no entry for an accepted deletion. An
    /// entry that isn't a readable regular file holds no version, so it
    /// holds neither an accepted edit nor an accepted deletion.
    // sokf-implements: CLI-1_AC-27
    // sokf-implements: CLI-4_AC-14
    pub fn holds(
        &self,
        dest: &Dest,
        bit: Option<bool>,
        path: &RepoPath,
        meaning: &Meaning,
    ) -> bool {
        match (dest, &self.hash) {
            (Dest::Absent, _) => self.deleted,
            (Dest::Unreadable(_), _) | (Dest::File(_), None) => false,
            (Dest::File(b), Some(h)) => {
                meaning.holds_fingerprint(path, b, h) && bit.is_none_or(|x| x == self.executable)
            }
        }
    }
}

impl Default for Lock {
    fn default() -> Self {
        Lock {
            lock_version: LOCK_VERSION,
            packs: BTreeMap::new(),
        }
    }
}

/// `first_version` is the version of a lock without `lock-version`.
fn first_version() -> u32 {
    1
}

impl Lock {
    /// `owner_of` returns the pack whose version of the destination `dest`
    /// the file holds, which is the one pack whose record of it isn't
    /// shadowed, and the file's path in that pack.
    pub fn owner_of(&self, dest: &str) -> Option<(&str, &str)> {
        self.packs.iter().find_map(|(name, pack)| {
            pack.files
                .iter()
                .find(|(_, f)| f.path == dest && f.shadowed_by.is_none())
                .map(|(rel, _)| (name.as_str(), rel.as_str()))
        })
    }
}

/// `holds_conflict` returns whether `bytes`, the text of the file at
/// `dest`, read with each invalid sequence replaced, hold a conflict, as
/// `conflict_lines` reads one. Where a sync left a conflict in the file,
/// `left` is the base copy that holds the pack's version, and an absent
/// base copy holds no example. Every command that reports or refuses a
/// conflict of a file of a pack asks this function, or
/// `LockedFile::holds_conflict`, so that the commands agree; only the
/// comparison by meaning, which has no record, reads the text itself.
// sokf-implements: CLI-1_AC-47
pub fn holds_conflict(
    root: &RepoRoot,
    dest: &RepoPath,
    bytes: &[u8],
    left: Option<&RepoPath>,
) -> bool {
    let text = String::from_utf8_lossy(bytes);
    // The base copy is read only for a text that holds the opening line of
    // pakf's merge, which no other text needs, and by the one reading of a
    // destination, so that a copy below a link out of the repository holds
    // no example on any machine.
    let version = left
        .filter(|_| text.lines().any(|l| crate::conflicts::is_opening(dest, l)))
        .map(|copy| match read_dest(root, copy) {
            Dest::File(b) => String::from_utf8_lossy(&b).into_owned(),
            _ => String::new(),
        });
    !crate::conflicts::conflict_lines(dest, &text, version.as_deref()).is_empty()
}

/// `hash_of` returns the hash of `bytes` as the lock records it: the plain
/// hash of `same-same`, in its text form, which keeps where each CRLF
/// stood, so that two bytes that differ hash apart.
pub fn hash_of(bytes: &[u8]) -> String {
    same_same::Fingerprint::of_bytes(bytes).to_string()
}

/// `read_lock` reads the lock, and returns an empty lock when the file is
/// absent. It refuses a lock that doesn't parse, and a pack whose key isn't
/// an id.
// sokf-implements: CONF-2_AC-3
pub fn read_lock(root: &RepoRoot) -> Result<Lock> {
    let path = lock_path();
    let Some(text) = read_optional(&path.to_fs(root))? else {
        return Ok(Lock::default());
    };
    // The version is read first, so that a lock of a newer format gives an
    // error that says to upgrade, and not one about an unknown key.
    // sokf-implements: CONF-2_AC-5
    let table: toml::Table =
        toml::from_str(&text).map_err(|e| Error::config(path.clone(), e.message()))?;
    if let Some(v) = table.get("lock-version").and_then(toml::Value::as_integer)
        && v > i64::from(LOCK_VERSION)
    {
        return Err(Error::config(
            path,
            format!(
                "the lock has the format {v}, and this pakf reads the format {LOCK_VERSION} at most; upgrade pakf"
            ),
        ));
    }
    let mut lock: Lock =
        toml::from_str(&text).map_err(|e| Error::config(path.clone(), e.message()))?;
    for name in lock.packs.keys() {
        if !super::manifest::is_pack_id(name) {
            return Err(Error::config(
                path,
                format!(
                    "the pack name `{name}` is not the id of a pack, of the form `@scope/name`"
                ),
            ));
        }
    }
    // A lock that this tool writes again takes its format.
    lock.lock_version = LOCK_VERSION;
    Ok(lock)
}

/// `lock_text` returns the text of the lock, with its header and the
/// version of its format.
// sokf-implements: CONF-2_AC-4
pub fn lock_text(lock: &Lock) -> Result<String> {
    let lock = Lock {
        lock_version: LOCK_VERSION,
        ..lock.clone()
    };
    let body = toml::to_string(&lock).map_err(|e| Error::Internal(e.to_string()))?;
    Ok(format!("{HEADER}{body}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `meaning` returns a comparison without a configuration, detection,
    /// or a cache on disk.
    fn meaning() -> Meaning {
        Meaning::in_memory(
            Path::new("."),
            crate::config::Compare {
                detect: false,
                rules: Vec::new(),
            },
            crate::pack::For::Reports,
        )
    }

    fn at() -> RepoPath {
        RepoPath::parse("/a.txt").unwrap()
    }

    #[test]
    fn a_lock_round_trips_through_its_text() {
        let mut lock = Lock::default();
        let mut pack = LockedPack {
            source: "https://example.com/method.git".into(),
            git_ref: Some("main".into()),
            path: None,
            commit: "0123abcd".into(),
            version: "1.0.0".into(),
            paths: BTreeMap::from([("/knowledge/".into(), "/docs/".into())]),
            depends: BTreeMap::new(),
            files: BTreeMap::new(),
        };
        pack.files.insert(
            "files/knowledge/schemas/issue.md".into(),
            LockedFile {
                path: "/docs/schemas/issue.md".into(),
                base: hash_of(b"x"),
                executable: false,
                shadowed_by: None,
                changed: false,
                editable: false,
                conflicted: false,
                accepted: None,
            },
        );
        pack.files.insert(
            "files/.gitignore".into(),
            LockedFile {
                path: "/.gitignore".into(),
                base: hash_of(b"y"),
                executable: false,
                shadowed_by: None,
                changed: false,
                editable: true,
                conflicted: false,
                accepted: Some(Accepted::of(
                    Some(b"y\nz\n"),
                    false,
                    "the project's binary",
                    &at(),
                    &meaning(),
                )),
            },
        );
        lock.packs.insert("@acme/method".into(), pack);
        let text = lock_text(&lock).unwrap();
        assert!(text.starts_with(HEADER), "{text}");
        assert!(text.contains("[packs.\"@acme/method\"]"), "{text}");
        assert!(text.contains("lock-version = 3\n"), "{text}");
        assert!(text.contains("editable = true\n"), "{text}");
        assert!(text.contains("note = \"the project's binary\""), "{text}");
        assert!(!text.contains("deleted"), "{text}");
        assert!(!text.contains("executable"), "{text}");
        assert!(text.contains("ref = \"main\""), "{text}");
        assert!(!text.contains("path = \"\""), "{text}");
        assert!(text.contains("\"/knowledge/\" = \"/docs/\""), "{text}");
        let back: Lock = toml::from_str(&text).unwrap();
        assert_eq!(back, lock);
        assert_eq!(
            lock.owner_of("/docs/schemas/issue.md"),
            Some(("@acme/method", "files/knowledge/schemas/issue.md"))
        );
        assert_eq!(lock.owner_of("/docs/schemas/other.md"), None);
    }

    // sokf-tests: CLI-1_AC-27
    // sokf-tests: CLI-4_AC-14
    #[test]
    fn a_file_holds_its_accepted_version_with_either_line_ending() {
        let (m, at) = (meaning(), at());
        let holds = |a: &Accepted, d: &Dest, bit| a.holds(d, bit, &at, &m);
        let a = Accepted::of(Some(b"a\nb\n"), true, "why", &at, &m);
        assert!(holds(&a, &file(b"a\nb\n"), Some(true)));
        assert!(holds(&a, &file(b"a\r\nb\r\n"), Some(true)));
        assert!(holds(&a, &file(b"a\nb\n"), None));
        assert!(!holds(&a, &file(b"a\nb\n"), Some(false)));
        assert!(!holds(&a, &file(b"a\nc\n"), Some(true)));
        assert!(!holds(&a, &Dest::Absent, None));
        let crlf = Accepted::of(Some(b"a\r\nb\r\n"), false, "why", &at, &m);
        assert!(holds(&crlf, &file(b"a\nb\n"), None));
        let gone = Accepted::of(None, true, "unused", &at, &m);
        assert!(gone.deleted && !gone.executable && gone.hash.is_none());
        assert!(holds(&gone, &Dest::Absent, None));
        assert!(!holds(&gone, &file(b""), None));
        // An entry that isn't a readable regular file holds neither.
        let unreadable = Dest::Unreadable("a directory stands at its path".into());
        assert!(!holds(&a, &unreadable, None));
        assert!(!holds(&gone, &unreadable, None));
        // An acceptance of an older lock holds no version.
        let old = Accepted {
            hash: Some("fnv1a64:a8c7f832281a39c5".into()),
            ..a
        };
        assert!(!holds(&old, &file(b"a\nb\n"), Some(true)));
    }

    // sokf-tests: CLI-1_AC-22
    // sokf-tests: CLI-1_AC-25
    #[test]
    fn a_file_holds_its_base_in_its_text_and_its_bit() {
        let record = |executable| LockedFile {
            path: "/bin/tool".into(),
            base: hash_of(b"a\nb\n"),
            executable,
            shadowed_by: None,
            changed: false,
            editable: false,
            conflicted: false,
            accepted: None,
        };
        let (plain, script) = (record(false), record(true));
        let m = meaning();
        let none = Path::new("/nonexistent/base");
        let holds = |f: &LockedFile, d: &Dest, bit| f.holds_base(d, bit, none, &m);
        assert!(holds(&plain, &file(b"a\nb\n"), Some(false)));
        assert!(holds(&plain, &file(b"a\r\nb\r\n"), Some(false)));
        assert!(holds(&script, &file(b"a\nb\n"), Some(true)));
        // A file that carries no bit holds its base by its text alone.
        assert!(holds(&plain, &file(b"a\nb\n"), None));
        assert!(holds(&script, &file(b"a\nb\n"), None));
        // A change of the bit, either way, is an edit.
        assert!(!holds(&plain, &file(b"a\nb\n"), Some(true)));
        assert!(!holds(&script, &file(b"a\nb\n"), Some(false)));
        assert!(!holds(&plain, &file(b"a\nc\n"), Some(false)));
        assert!(!holds(&plain, &Dest::Absent, None));
        let unreadable = Dest::Unreadable("a directory stands at its path".into());
        assert!(!holds(&plain, &unreadable, None));
        // A base of an older lock holds no text, and the base copy settles
        // it.
        let old = LockedFile {
            base: "fnv1a64:a8c7f832281a39c5".into(),
            ..record(false)
        };
        assert!(!holds(&old, &file(b"a\nb\n"), None));
        let dir = crate::test_support::fresh_dir("lock-old-base");
        std::fs::write(dir.join("base"), b"a\nb\n").unwrap();
        assert!(old.holds_base(&file(b"a\r\nb\r\n"), None, &dir.join("base"), &m));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `file` returns a regular file that holds `bytes`.
    fn file(bytes: &[u8]) -> Dest {
        Dest::File(bytes.to_vec())
    }

    // sokf-tests: CONF-2_AC-4
    #[test]
    fn a_lock_names_each_pack_by_its_id_and_takes_the_format_of_the_tool() {
        let dir = crate::test_support::fresh_dir("lock-v1");
        let root = RepoRoot::new(dir.clone());
        std::fs::create_dir_all(dir.join(".pakf")).unwrap();
        std::fs::write(
            dir.join(".pakf/lock.toml"),
            "[packs.\"@acme/p\"]\nsource = \"x\"\ncommit = \"1\"\nversion = \"1\"\n",
        )
        .unwrap();
        let lock = read_lock(&root).unwrap();
        assert!(lock.packs.contains_key("@acme/p"));
        // A lock of the version 1, which pakf 0.1 wrote, reads, and is
        // written again as the version 3.
        assert!(lock_text(&lock).unwrap().contains("lock-version = 3\n"));
        std::fs::write(
            dir.join(".pakf/lock.toml"),
            "lock-version = 1\n[packs.\"@acme/p\"]\nsource = \"x\"\ncommit = \"1\"\nversion = \"1\"\n",
        )
        .unwrap();
        assert!(read_lock(&root).unwrap().packs.contains_key("@acme/p"));
        // A lock of the version 2, which pakf 0.2 wrote, reads with its
        // `fnv1a64:` values.
        std::fs::write(
            dir.join(".pakf/lock.toml"),
            "lock-version = 2\n[packs.\"@acme/p\"]\nsource = \"x\"\ncommit = \"1\"\nversion = \"1\"\n[packs.\"@acme/p\".files.\"files/a\"]\npath = \"/a\"\nbase = \"fnv1a64:cbf29ce484222325\"\n[packs.\"@acme/p\".files.\"files/a\".accepted]\nhash = \"fnv1a64:cbf29ce484222325\"\nnote = \"n\"\n",
        )
        .unwrap();
        let old = read_lock(&root).unwrap();
        assert_eq!(
            old.packs["@acme/p"].files["files/a"].base,
            "fnv1a64:cbf29ce484222325"
        );
        assert!(lock_text(&old).unwrap().contains("lock-version = 3\n"));
        std::fs::write(dir.join(".pakf/lock.toml"), "lock-version = 4\n").unwrap();
        let e = read_lock(&root).unwrap_err().to_string();
        assert!(e.contains("upgrade pakf"), "{e}");
        // A key that isn't an id is refused, a bare slug among them.
        for key in ["p", "\"acme/p\""] {
            std::fs::write(
                dir.join(".pakf/lock.toml"),
                format!("[packs.{key}]\nsource = \"x\"\ncommit = \"1\"\nversion = \"1\"\n"),
            )
            .unwrap();
            let e = read_lock(&root).unwrap_err().to_string();
            assert!(e.contains("`@scope/name`"), "{e}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // sokf-tests: CLI-1_AC-29
    #[test]
    fn the_paths_of_a_pack_lie_under_its_directory() {
        assert_eq!(pack_dir("@pakf/pakf").as_str(), "/.pakf/packs/@pakf/pakf");
        assert_eq!(
            base_path("@pakf/pakf", "files/knowledge/schemas/issue.md").as_str(),
            "/.pakf/packs/@pakf/pakf/base/files/knowledge/schemas/issue.md"
        );
        assert_eq!(
            instructions_of("@pakf/pakf").as_str(),
            "/.pakf/packs/@pakf/pakf/instructions.md"
        );
        assert!(hash_of(b"").starts_with("ss1:bytes:"), "{}", hash_of(b""));
        assert_ne!(hash_of(b"a\r\n"), hash_of(b"a\n"));
    }

    proptest::proptest! {
        // sokf-tests: CONF_P-1
        #[test]
        fn the_text_of_a_lock_reads_back_as_the_same_lock(
            packs in proptest::collection::btree_map(
                ("[a-z][a-z0-9]{0,4}", "[a-z][a-z0-9]{0,4}"),
                (
                    "[ -~]{1,12}",
                    proptest::option::of("[a-z]{1,6}"),
                    proptest::collection::btree_map(
                        "files/[a-z]{1,6}",
                        ("/[a-z]{1,6}", proptest::prelude::any::<bool>(), proptest::prelude::any::<bool>(), proptest::prelude::any::<bool>(), proptest::option::of("[ -~]{0,10}")),
                        0..4,
                    ),
                ),
                0..4,
            ),
        ) {
            let mut lock = Lock::default();
            for ((scope, name), (source, git_ref, files)) in packs {
                let files = files
                    .into_iter()
                    .map(|(rel, (path, executable, editable, conflicted, note))| {
                        let accepted = note.map(|n| Accepted::of(Some(b"x"), executable, &n, &at(), &meaning()));
                        (rel, LockedFile {
                            path,
                            base: hash_of(b"base"),
                            executable,
                            shadowed_by: None,
                            changed: false,
                            editable,
                            conflicted,
                            accepted,
                        })
                    })
                    .collect();
                lock.packs.insert(format!("@{scope}/{name}"), LockedPack {
                    source,
                    git_ref,
                    commit: "0123abcd".into(),
                    version: "1.0.0".into(),
                    files,
                    ..LockedPack::default()
                });
            }
            let text = lock_text(&lock).unwrap();
            let back: Lock = toml::from_str(&text).unwrap();
            proptest::prop_assert_eq!(back, lock);
        }
    }
}
