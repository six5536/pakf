//! This module reads and writes `/.pakf/lock.toml`, the record of each
//! schema pack that `pakf sync` copied into the repository.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{
    dest::Dest,
    eol::{is_base, normalise},
};
use crate::{Error, RepoPath, RepoRoot, Result, source::read_optional};

/// `HEADER` is the comment at the top of the lock.
const HEADER: &str = "# pakf sync writes this file. Do not edit it.\n";

/// `LOCK_VERSION` is the version of the lock's format that this tool reads
/// and writes. A lock without `lock-version` has the version 1, and the
/// version 2 adds `editable` to the record of a file, so this tool reads a
/// lock of either version.
pub const LOCK_VERSION: u32 = 2;

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
    /// The hash of the file's base: the pack's version of the file at the
    /// last sync, as `fnv1a64:<HEX>`.
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
    /// The hash of the accepted version, with each CRLF of a text read as
    /// LF, as `fnv1a64:<HEX>`, or none for an accepted deletion.
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
    /// `holds_base` returns whether what stands at the file's path, `dest`,
    /// holds the file's base: a regular file whose text is the base, with
    /// each CRLF read as LF, and, for a file of `files/`, whose executable
    /// bit `bit` is the bit that the record holds. `bit` is none for any
    /// other file and on a platform without the bit, where no command
    /// compares it. An absent file and an entry that isn't a readable
    /// regular file hold no base. The test of an edit that `pakf status`,
    /// `pakf ls`, `pakf accept`, and `pakf remove` share, and sync's removal
    /// of a file that its pack no longer holds, ask this function, so that
    /// they read the same file alike. Sync's update of a file that its pack
    /// still holds reads the text against the base apart from the bit, which
    /// its plan of the bit decides on its own.
    // sokf-implements: CLI-1_AC-22
    // sokf-implements: CLI-1_AC-25
    pub fn holds_base(&self, dest: &Dest, bit: Option<bool>) -> bool {
        matches!(dest, Dest::File(b) if is_base(b, &self.base))
            && bit.is_none_or(|x| x == self.executable)
    }
}

impl Accepted {
    /// `of` returns the acceptance of a file that holds `bytes`, or of a
    /// deleted file when `bytes` is none, with the executable bit `bit` of a
    /// file of `files/`.
    pub fn of(bytes: Option<&[u8]>, bit: bool, note: &str) -> Accepted {
        Accepted {
            hash: bytes.map(|b| hash_of(&normalise(b))),
            executable: bytes.is_some() && bit,
            deleted: bytes.is_none(),
            note: note.to_string(),
        }
    }

    /// `holds` returns whether what stands at the file's path, `dest`,
    /// holds the accepted version: a regular file with the accepted text
    /// and, for a file of `files/`, the accepted executable bit `bit`, which
    /// is none for any other file, or no entry for an accepted deletion. An
    /// entry that isn't a readable regular file holds no version, so it
    /// holds neither an accepted edit nor an accepted deletion.
    // sokf-implements: CLI-1_AC-27
    // sokf-implements: CLI-4_AC-14
    pub fn holds(&self, dest: &Dest, bit: Option<bool>) -> bool {
        match (dest, &self.hash) {
            (Dest::Absent, _) => self.deleted,
            (Dest::Unreadable(_), _) | (Dest::File(_), None) => false,
            (Dest::File(b), Some(h)) => is_base(b, h) && bit.is_none_or(|x| x == self.executable),
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

/// `hash_of` returns the hash of `bytes` as the lock records it.
pub fn hash_of(bytes: &[u8]) -> String {
    format!("fnv1a64:{:016x}", crate::source::fnv1a64(bytes))
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
                accepted: Some(Accepted::of(Some(b"y\nz\n"), false, "the project's binary")),
            },
        );
        lock.packs.insert("@acme/method".into(), pack);
        let text = lock_text(&lock).unwrap();
        assert!(text.starts_with(HEADER), "{text}");
        assert!(text.contains("[packs.\"@acme/method\"]"), "{text}");
        assert!(text.contains("lock-version = 2\n"), "{text}");
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
        let a = Accepted::of(Some(b"a\nb\n"), true, "why");
        assert!(a.holds(&file(b"a\nb\n"), Some(true)));
        assert!(a.holds(&file(b"a\r\nb\r\n"), Some(true)));
        assert!(a.holds(&file(b"a\nb\n"), None));
        assert!(!a.holds(&file(b"a\nb\n"), Some(false)));
        assert!(!a.holds(&file(b"a\nc\n"), Some(true)));
        assert!(!a.holds(&Dest::Absent, None));
        let crlf = Accepted::of(Some(b"a\r\nb\r\n"), false, "why");
        assert!(crlf.holds(&file(b"a\nb\n"), None));
        let gone = Accepted::of(None, true, "unused");
        assert!(gone.deleted && !gone.executable && gone.hash.is_none());
        assert!(gone.holds(&Dest::Absent, None));
        assert!(!gone.holds(&file(b""), None));
        // An entry that isn't a readable regular file holds neither.
        let unreadable = Dest::Unreadable("a directory stands at its path".into());
        assert!(!a.holds(&unreadable, None));
        assert!(!gone.holds(&unreadable, None));
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
            accepted: None,
        };
        let (plain, script) = (record(false), record(true));
        assert!(plain.holds_base(&file(b"a\nb\n"), Some(false)));
        assert!(plain.holds_base(&file(b"a\r\nb\r\n"), Some(false)));
        assert!(script.holds_base(&file(b"a\nb\n"), Some(true)));
        // A file that carries no bit holds its base by its text alone.
        assert!(plain.holds_base(&file(b"a\nb\n"), None));
        assert!(script.holds_base(&file(b"a\nb\n"), None));
        // A change of the bit, either way, is an edit.
        assert!(!plain.holds_base(&file(b"a\nb\n"), Some(true)));
        assert!(!script.holds_base(&file(b"a\nb\n"), Some(false)));
        assert!(!plain.holds_base(&file(b"a\nc\n"), Some(false)));
        assert!(!plain.holds_base(&Dest::Absent, None));
        let unreadable = Dest::Unreadable("a directory stands at its path".into());
        assert!(!plain.holds_base(&unreadable, None));
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
        // written again as the version 2.
        assert!(lock_text(&lock).unwrap().contains("lock-version = 2\n"));
        std::fs::write(
            dir.join(".pakf/lock.toml"),
            "lock-version = 1\n[packs.\"@acme/p\"]\nsource = \"x\"\ncommit = \"1\"\nversion = \"1\"\n",
        )
        .unwrap();
        assert!(read_lock(&root).unwrap().packs.contains_key("@acme/p"));
        std::fs::write(dir.join(".pakf/lock.toml"), "lock-version = 3\n").unwrap();
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
        assert_eq!(hash_of(b""), "fnv1a64:cbf29ce484222325");
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
                        ("/[a-z]{1,6}", proptest::prelude::any::<bool>(), proptest::prelude::any::<bool>(), proptest::option::of("[ -~]{0,10}")),
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
                    .map(|(rel, (path, executable, editable, note))| {
                        let accepted = note.map(|n| Accepted::of(Some(b"x"), executable, &n));
                        (rel, LockedFile {
                            path,
                            base: hash_of(b"base"),
                            executable,
                            shadowed_by: None,
                            changed: false,
                            editable,
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
