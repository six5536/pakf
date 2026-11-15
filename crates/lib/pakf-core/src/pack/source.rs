//! This module reads a pack from its source, a directory of a git clone. It
//! also resolves a source that is a relative path (the decision "A relative
//! source" of design-pack).

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};

use super::{
    manifest::{MANIFEST, PackManifest, parse_manifest},
    remap::Remap,
};
use crate::{Error, Result};

/// A `Source` is where a pack comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A git repository, by the URL or path that `git clone` reads.
    Git(String),
}

impl Source {
    /// `parse` reads a source: a git URL or path, as `git clone` reads it.
    pub fn parse(s: &str) -> Source {
        Source::Git(s.to_string())
    }

    /// `label` returns the source as the lock records it.
    pub fn label(&self) -> String {
        match self {
            Source::Git(url) => url.clone(),
        }
    }
}

/// `is_relative` reports whether the git source `url` is a relative path:
/// git reads it as a local path, because it holds no `:` or a `/` comes
/// before its first `:`, and it neither begins with `/` nor is absolute on
/// the platform. A URL and an scp-like address, such as
/// `git@example.com:o/r.git`, hold a `:` before any `/`.
fn is_relative(url: &str) -> bool {
    let local = match (url.find(':'), url.find('/')) {
        (None, _) => true,
        (Some(colon), Some(slash)) => slash < colon,
        (Some(_), None) => false,
    };
    local && !url.starts_with('/') && !Path::new(url).is_absolute()
}

/// `is_local` reports whether the git source `url` is a local directory: a
/// relative path, or an absolute path that git reads as a local path. A URL,
/// `file://` among them, and an scp-like address aren't, because they name
/// a repository that pakf only clones.
// sokf-implements: CLI-9_AC-12
pub fn is_local(url: &str) -> bool {
    is_relative(url) || url.starts_with('/') || Path::new(url).is_absolute()
}

/// `from_root` returns the location that git reads for the git source
/// `url`: a relative path joined to the repository root `root`, so that it
/// names the same repository from every directory, and any other source as
/// it is. It joins the root as the disk spells it, as `recorded` reads it,
/// because Windows resolves a `..` without the disk, so that a root reached
/// through a junction would otherwise give `../` another parent.
// sokf-implements: CLI-2_AC-52
pub fn from_root(url: &str, root: &Path) -> String {
    if is_relative(url) {
        let root = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        root.join(url).to_string_lossy().into_owned()
    } else {
        url.to_string()
    }
}

/// `recorded` returns the git source that `pakf add` records for the git
/// source `url` typed on its command line in the directory `cwd`. A
/// relative path resolves from `cwd` and becomes the path from the
/// repository root `root`, with the typed path normalised without reading
/// the disk, with `/` between its segments, and beginning with `./` unless
/// it begins with `../`. A path on another drive or share than the root
/// keeps its absolute path, with `/` between its segments. Either way, the
/// recorded source holds a `/`, so it never reads as a short name. Any
/// other source is recorded as it is typed.
// sokf-implements: CLI-3_AC-4
pub fn recorded(url: &str, root: &Path, cwd: &Path) -> String {
    if !is_relative(url) {
        return url.to_string();
    }
    // The root and the current directory take the form that the disk gives
    // them, so that another case of letters or a substituted drive on
    // Windows names the same directory, and a relative `cwd` resolves from
    // the root. Both take the full form of `fs::canonicalize`, which on
    // Windows is the verbatim `\\?\` form, so that their prefixes compare
    // alike, or, when either can't be read, both keep their form without
    // the disk. The typed path joins them segment by segment, because a
    // verbatim path reads no `/` as a separator.
    let dir = root.join(cwd);
    let (base, from) = match (fs::canonicalize(root), fs::canonicalize(&dir)) {
        (Ok(base), Ok(from)) => (base, from),
        _ => (
            normalised(PathBuf::new(), root),
            normalised(PathBuf::new(), &dir),
        ),
    };
    let target = normalised(from, Path::new(url));
    let parts: Vec<Component> = target.components().collect();
    let base: Vec<Component> = base.components().collect();
    let common = parts.iter().zip(&base).take_while(|(a, b)| a == b).count();
    if common == 0 {
        return dunce::simplified(&target)
            .to_string_lossy()
            .replace('\\', "/");
    }
    let rest: Vec<String> = parts[common..]
        .iter()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let up = base.len() - common;
    let lead = if up == 0 {
        "./".to_string()
    } else {
        "../".repeat(up)
    };
    format!("{lead}{}", rest.join("/"))
}

/// `normalised` returns `base` followed by each segment of `path`, without
/// a `.` segment, and with each `..` segment taking away the segment before
/// it, without reading the disk, as `cargo add --path` reads a path. A `..`
/// at the root stays at the root.
fn normalised(mut base: PathBuf, path: &Path) -> PathBuf {
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                base.pop();
            }
            other => base.push(other),
        }
    }
    base
}

/// A `PackFiles` is a pack as pakf reads it: its manifest and each other
/// file, by its path in the pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackFiles {
    /// The manifest.
    pub manifest: PackManifest,
    /// Each file of the pack other than the manifest, by its path in the
    /// pack, with `/` between the segments.
    pub files: BTreeMap<String, Vec<u8>>,
    /// The path in the pack of each file of `files/` that is executable.
    pub executable: BTreeSet<String>,
}

/// `read_dir` reads the pack in `dir`: its manifest and every file under it,
/// except the directory `.git`. It refuses a pack that holds a symbolic
/// link, because a link in a cloned repository can point at any file of the
/// machine, and sync would copy that file into the project. It refuses a
/// pack that holds `schemas/`, `documents/`, or `formats.yaml` at its root,
/// which pakf no longer reads.
// sokf-implements: CONF-3_AC-1
// sokf-implements: CLI-2_AC-17
// sokf-implements: CONF-3_AC-2
pub fn read_dir(dir: &Path, label: &str) -> Result<PackFiles> {
    if !dir.is_dir() {
        return Err(Error::Pack(format!("{label} holds no {MANIFEST}")));
    }
    let mut files = BTreeMap::new();
    collect(dir, "", label, &mut files)?;
    let text = files
        .remove(MANIFEST)
        .ok_or_else(|| Error::Pack(format!("{label} holds no {MANIFEST}")))?;
    let text = String::from_utf8(text)
        .map_err(|_| Error::Pack(format!("{label}/{MANIFEST} is not UTF-8")))?;
    let manifest = parse_manifest(&text, &format!("{label}/{MANIFEST}"))?;
    // sokf-implements: CLI-2_AC-79
    let old = old_layout(&files);
    if !old.is_empty() {
        return Err(Error::Pack(format!(
            "{}: the pack holds {} at its root, which pakf no longer reads; a pack is one tree of files, so move each file under files/ at the path where it lands, such as schemas/issue.md to files/knowledge/schemas/issue.md",
            manifest.name,
            old.join(" and ")
        )));
    }
    // The caller reads the executable bits from git's tree.
    Ok(PackFiles {
        manifest,
        files,
        executable: BTreeSet::new(),
    })
}

/// `collect` adds each file under `dir` to `out`, by its path under the pack
/// with the prefix `prefix`. It reads the type of each entry without
/// following a symbolic link, and refuses the first link that it meets.
fn collect(
    dir: &Path,
    prefix: &str,
    label: &str,
    out: &mut BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    let entries = fs::read_dir(dir).map_err(|e| Error::io(dir, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(dir, e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let path = entry.path();
        let rel = format!("{prefix}{name}");
        let kind = fs::symlink_metadata(&path)
            .map_err(|e| Error::io(&path, e))?
            .file_type();
        if kind.is_symlink() {
            return Err(Error::Pack(format!(
                "{label}: {rel} is a symbolic link, and a pack from git holds only files and directories"
            )));
        }
        if kind.is_dir() {
            collect(&path, &format!("{rel}/"), label, out)?;
        } else {
            let bytes = fs::read(&path).map_err(|e| Error::io(&path, e))?;
            out.insert(rel, bytes);
        }
    }
    Ok(())
}

/// `clone_dir` returns the directory `path` of the clone `worktree`, and
/// refuses a path with a `..` segment or a symbolic link on the way, so the
/// directory lies inside the clone.
// sokf-implements: CLI-2_AC-17
pub fn clone_dir(worktree: &Path, path: &str) -> Result<std::path::PathBuf> {
    let mut dir = worktree.to_path_buf();
    for segment in segments(path) {
        if segment == ".." {
            return Err(Error::Pack(format!(
                "--path {path}: the path leaves the repository"
            )));
        }
        dir.push(segment);
        let Ok(meta) = fs::symlink_metadata(&dir) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            return Err(Error::Pack(format!(
                "--path {path}: {segment} is a symbolic link, so the path can leave the repository"
            )));
        }
    }
    Ok(dir)
}

/// `segments` returns the segments of the path `path` of a pack's directory
/// in its source, without an empty segment or a `.`.
pub fn segments(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter(|s| !s.is_empty() && *s != ".")
}

/// `FILES` is the directory of a pack whose files land in the project, each
/// at its path from the repository root after the remaps of its entry.
pub const FILES: &str = "files/";

/// `OLD_LAYOUT` lists the parts of a pack's root that pakf no longer reads,
/// because a pack is one tree of files under `files/`.
const OLD_LAYOUT: [&str; 3] = ["schemas/", "documents/", "formats.yaml"];

/// `old_layout` returns the parts of the pack's root, of `OLD_LAYOUT`, that
/// the files `files`, by their paths in the pack, hold: a directory that
/// holds a file, or the file itself.
fn old_layout(files: &BTreeMap<String, Vec<u8>>) -> Vec<&'static str> {
    OLD_LAYOUT
        .into_iter()
        .filter(|part| {
            files
                .keys()
                .any(|rel| rel == part || (part.ends_with('/') && rel.starts_with(part)))
        })
        .collect()
}

/// `destination` returns where sync writes the file `rel` of the pack
/// `name`: a file under `files/` goes to its path from the repository root
/// after the remaps `remap` of the pack's entry, and `instructions.md` to
/// the same name under `/.pakf/packs/<NAME>/`. Any other file has no
/// destination. The planner checks where a file of `files/` lands.
// sokf-implements: CLI-2_AC-22
// sokf-implements: CONF-3_AC-10
pub fn destination(name: &str, remap: &Remap, rel: &str) -> Option<String> {
    if let Some(rest) = rel.strip_prefix(FILES) {
        return Some(remap.forward(&format!("/{rest}")));
    }
    // sokf-implements: CLI-2_AC-16
    (rel == "instructions.md").then(|| super::lock::instructions_of(name).as_str().to_string())
}

/// `in_files` returns the path in a pack's `files/` of the file whose path
/// from the repository root is `path`.
pub fn in_files(path: &str) -> String {
    format!("{FILES}{}", path.trim_start_matches('/'))
}

/// `origin` is the inverse of `destination`: it returns the path in the
/// pack `name`, whose entry remaps its paths by `remap`, of the project's
/// file `dest`, or the reason that the file has no place in the pack. The
/// copy of the pack's `instructions.md` goes to the pack's root, and any
/// other file to `files/`, at its path after the inverse of the remaps. It
/// reads nothing but the path and the remaps.
// sokf-implements: CLI-9_AC-3
// sokf-implements: CLI-9_AC-29
pub fn origin(name: &str, remap: &Remap, dest: &str) -> std::result::Result<String, String> {
    if dest == super::lock::instructions_of(name).as_str() {
        return Ok("instructions.md".into());
    }
    remap.backward(dest).map(|path| in_files(&path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_source_is_a_git_source() {
        for s in [
            "pakf",
            "builtin:pakf",
            "@pakf/pakf",
            "git@example.com:o/r.git",
            "../pack",
        ] {
            assert_eq!(Source::parse(s), Source::Git(s.into()));
            assert_eq!(Source::parse(s).label(), s);
        }
    }

    #[test]
    fn a_relative_path_is_a_local_path_that_isnt_absolute() {
        for url in ["./", "../common", "packs/core", "./a:b", "../x:y/z"] {
            assert!(is_relative(url), "{url}");
        }
        for url in [
            "/srv/packs",
            "https://example.com/o/r.git",
            "file:///srv/packs",
            "git@example.com:o/r.git",
            "example.com:o/r",
        ] {
            assert!(!is_relative(url), "{url}");
        }
    }

    // sokf-tests: CLI-9_AC-12
    #[test]
    fn a_local_directory_is_a_path_and_never_a_url() {
        for url in ["./", "../common", "packs/core", "/srv/packs"] {
            assert!(is_local(url), "{url}");
        }
        for url in [
            "https://example.com/o/r.git",
            "file:///srv/packs",
            "git@example.com:o/r.git",
            "example.com:o/r",
        ] {
            assert!(!is_local(url), "{url}");
        }
    }

    // sokf-tests: CLI-2_AC-52
    #[cfg(unix)]
    #[test]
    fn a_relative_source_resolves_from_the_root() {
        let root = Path::new("/repo");
        assert_eq!(from_root("./", root), "/repo/./");
        assert_eq!(from_root("../common", root), "/repo/../common");
        assert_eq!(from_root("packs/core", root), "/repo/packs/core");
        for url in [
            "/srv/packs",
            "https://example.com/o/r.git",
            "git@example.com:o/r.git",
        ] {
            assert_eq!(from_root(url, root), url);
        }
    }

    // sokf-tests: CLI-3_AC-4
    #[cfg(unix)]
    #[test]
    fn a_typed_relative_source_is_recorded_from_the_root() {
        let root = Path::new("/repo");
        let sub = Path::new("/repo/knowledge");
        for (typed, cwd, want) in [
            ("./", root, "./"),
            ("packs/core/", root, "./packs/core"),
            ("./packs/./core", root, "./packs/core"),
            ("../common", root, "../common"),
            ("./", sub, "./knowledge"),
            ("../", sub, "./"),
            ("../common", sub, "./common"),
            ("../../common", sub, "../common"),
            ("../../", sub, "../"),
            ("./a:b", sub, "./knowledge/a:b"),
            // A current directory that isn't absolute resolves from the root.
            ("../common", Path::new(""), "../common"),
            ("../common", Path::new("knowledge"), "./common"),
        ] {
            assert_eq!(
                recorded(typed, root, cwd),
                want,
                "{typed} in {}",
                cwd.display()
            );
        }
        for typed in [
            "/srv/packs",
            "https://example.com/o/r.git",
            "git@example.com:o/r.git",
        ] {
            assert_eq!(recorded(typed, root, sub), typed);
        }
    }

    // sokf-tests: CLI-3_AC-4
    #[cfg(windows)]
    #[test]
    fn a_typed_source_on_windows_is_recorded_from_the_root() {
        let root = Path::new(r"C:\repo");
        let sub = Path::new(r"C:\repo\k\sub");
        for (typed, want) in [
            ("./", "./k/sub"),
            ("../", "./k"),
            ("../../common", "./common"),
            ("../../../common", "../common"),
            ("./a/../b/", "./k/sub/b"),
        ] {
            assert_eq!(recorded(typed, root, sub), want, "{typed}");
        }
    }

    // sokf-tests: CLI-3_AC-4
    #[cfg(windows)]
    #[test]
    fn a_typed_source_on_another_drive_keeps_its_absolute_path() {
        let root = Path::new(r"C:\repo");
        assert_eq!(
            recorded("./common", root, Path::new(r"D:\work")),
            "D:/work/common"
        );
        let share = recorded("./common", root, Path::new(r"\\srv\share\work"));
        assert_eq!(share, "//srv/share/work/common");
        assert!(matches!(Source::parse(&share), Source::Git(_)));
    }

    // sokf-tests: CLI-2_AC-22
    // sokf-tests: CONF-3_AC-10
    #[test]
    fn each_file_lands_at_its_destination() {
        let none = Remap::default();
        let d = |rel| destination("m", &none, rel);
        assert_eq!(
            d("files/knowledge/schemas/issue.md").unwrap(),
            "/knowledge/schemas/issue.md"
        );
        assert_eq!(
            d("instructions.md").unwrap(),
            "/.pakf/packs/m/instructions.md"
        );
        assert_eq!(d("README.md"), None);
        assert_eq!(d("files/rustfmt.toml").unwrap(), "/rustfmt.toml");
        assert_eq!(d("files/docs/index.md").unwrap(), "/docs/index.md");
        let remap = Remap::new(&BTreeMap::from([("/knowledge/".into(), "/docs/".into())])).unwrap();
        assert_eq!(
            destination("m", &remap, "files/knowledge/schemas/issue.md").unwrap(),
            "/docs/schemas/issue.md"
        );
        assert_eq!(
            destination("m", &remap, "instructions.md").unwrap(),
            "/.pakf/packs/m/instructions.md"
        );
    }

    // sokf-tests: CLI-9_AC-3
    // sokf-tests: CLI-9_AC-29
    #[test]
    fn each_file_maps_back_to_its_place_in_the_pack() {
        let remap = Remap::new(&BTreeMap::from([("/knowledge/".into(), "/docs/".into())])).unwrap();
        let o = |dest| origin("m", &remap, dest);
        for (dest, rel) in [
            (
                "/docs/schemas/runbook.md",
                "files/knowledge/schemas/runbook.md",
            ),
            ("/docs/index.md", "files/knowledge/index.md"),
            ("/.pakf/packs/m/instructions.md", "instructions.md"),
            ("/scripts/check.sh", "files/scripts/check.sh"),
            ("/knowledge.toml", "files/knowledge.toml"),
        ] {
            assert_eq!(o(dest).as_deref(), Ok(rel), "{dest}");
            // The inverse of `destination` leads back to the same file.
            assert_eq!(destination("m", &remap, rel).as_deref(), Some(dest));
        }
        // Another pack's copies lie in `/.pakf/`, where sync refuses to
        // land them.
        assert_eq!(
            o("/.pakf/packs/other/instructions.md").as_deref(),
            Ok("files/.pakf/packs/other/instructions.md")
        );
        // The pack's own path of a remapped file lands elsewhere.
        assert!(o("/knowledge/a.md").is_err());
    }

    // sokf-tests: CLI-2_AC-79
    #[test]
    fn a_pack_of_the_old_layout_is_refused() {
        let pack = |rels: &[&str]| -> BTreeMap<String, Vec<u8>> {
            rels.iter().map(|r| (r.to_string(), Vec::new())).collect()
        };
        let files = pack(&[
            "schemas/a.md",
            "files/schemas/b.md",
            "documents/x.md",
            "formats.yaml",
            "files/formats.yaml",
        ]);
        assert_eq!(
            old_layout(&files),
            ["schemas/", "documents/", "formats.yaml"]
        );
        let files = pack(&[
            "files/schemas/b.md",
            "instructions.md",
            "README.md",
            "schemas",
        ]);
        assert_eq!(old_layout(&files), Vec::<&str>::new());
    }
}
