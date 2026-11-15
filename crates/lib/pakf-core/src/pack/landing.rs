//! This module checks where the files of a pack land, before the planner
//! plans any of them: the pack's `instructions.md` is text, a file of
//! `files/` lands outside git's and pakf's own files, except the pack's own
//! folder of ignores, and in no other pack's folder of a `*.d/pakf/`
//! directory, and no two files of one pack land at one destination (the
//! decisions "Where a file of a pack lands" and "Binary files" of
//! design-pack).

use std::collections::BTreeMap;

use super::{
    remap::Remap,
    source::{FILES, PackFiles, destination},
};

/// `own_ignores` returns the folder of the pack `name` in `/.pakf/ignore.d/`,
/// the one place of `/.pakf/` where a pack writes, in the case of letters
/// that pakf reads.
pub fn own_ignores(name: &str) -> String {
    format!("/.pakf/ignore.d/{name}")
}

/// `below` returns whether the path `path` lies below the directory `dir`.
pub fn below(dir: &str, path: &str) -> bool {
    path.strip_prefix(dir)
        .is_some_and(|rest| rest.starts_with('/'))
}

/// `is_text` returns whether pakf reads the file `rel` of a pack as text,
/// so that it is never binary: the pack's `instructions.md`, which a
/// harness reads.
pub fn is_text(rel: &str) -> bool {
    rel == "instructions.md"
}

/// `check` returns a line for each file of `pack`, whose entry remaps its
/// paths by `remap`, that sync must refuse, as `refusal` says.
// sokf-implements: CLI-2_AC-23
// sokf-implements: CLI-2_AC-25
pub fn check(pack: &PackFiles, remap: &Remap) -> Vec<String> {
    let name = &pack.manifest.name;
    pack.files
        .iter()
        .filter_map(|(rel, bytes)| {
            // A file without a destination never lands.
            let dest = destination(name, remap, rel)?;
            refusal(name, rel, bytes, &dest)
        })
        .collect()
}

/// `refusal` returns why sync must refuse the file `rel` of the pack
/// `name`, which holds `bytes` and lands at `dest`, or `None` when sync
/// lands it: sync refuses a binary file that pakf reads as text, as
/// `is_text` says, and a file of `files/` whose path holds a segment that
/// leaves its place, that lands at the repository root, in `/.pakf/` other
/// than below `/.pakf/ignore.d/` and the pack's id, in any `.git`
/// directory, or below a folder `pakf/` of a directory whose name ends in
/// `.d` other than below the pack's id. It compares the names of the
/// folders without regard to the case of letters, because macOS and
/// Windows open `.GIT/` as `.git/`, and the id in its own case.
// sokf-implements: CLI-9_AC-5
// sokf-implements: CLI-2_AC-81
pub fn refusal(name: &str, rel: &str, bytes: &[u8], dest: &str) -> Option<String> {
    if is_text(rel) && super::eol::is_binary(bytes) {
        return Some(format!("{rel} is binary, and pakf reads the file as text"));
    }
    let rest = rel.strip_prefix(FILES)?;
    if rest
        .split('/')
        .any(|s| s.is_empty() || s == "." || s == "..")
    {
        return Some(format!(
            "{rel}: the path leaves its place in the repository"
        ));
    }
    if dest == "/" {
        return Some(format!(
            "{rel}: the paths of the pack's entry land the file at the repository root"
        ));
    }
    let segments: Vec<&str> = dest.trim_start_matches('/').split('/').collect();
    let id: Vec<&str> = name.splitn(2, '/').collect();
    // The pack's own folder at `at`: its id, with at least one name below.
    let own_at = |at: usize| segments.len() > at + 2 && segments[at..at + 2] == id[..];
    // sokf-implements: CLI-2_AC-23
    if segments.iter().any(|s| s.eq_ignore_ascii_case(".git")) {
        return Some(format!(
            "{rel} lands at {dest}, among git's own files, which no pack writes"
        ));
    }
    if segments[0].eq_ignore_ascii_case(".pakf") && !below(&own_ignores(name), dest) {
        return Some(format!(
            "{rel} lands at {dest}, among pakf's own files, which a pack writes only below /.pakf/ignore.d/{name}/"
        ));
    }
    let foreign = segments
        .windows(2)
        .enumerate()
        .find_map(|(at, pair)| match pair {
            [dir, pakf]
                if dir.to_ascii_lowercase().ends_with(".d")
                    && pakf.eq_ignore_ascii_case("pakf")
                    && !own_at(at + 2) =>
            {
                Some((at, *dir))
            }
            _ => None,
        });
    if let Some((at, dir)) = foreign {
        let folder = format!("/{}", segments[..at + 2].join("/"));
        return Some(format!(
            "{rel} lands at {dest}, in {folder}/ outside {folder}/{name}/, and a pack writes in a folder `pakf/` of a `{dir}` directory only below its own id"
        ));
    }
    None
}

/// `collisions` returns a line for each file of `pack` that lands at the
/// destination of another file of the same pack, or below it, after the
/// remaps `remap`, compared without regard to the case of letters, because
/// one pack has no one version of such a path to keep, and no file stands
/// at a directory of files.
// sokf-implements: CLI-2_AC-69
pub fn collisions(pack: &PackFiles, remap: &Remap) -> Vec<String> {
    let name = &pack.manifest.name;
    let mut first: BTreeMap<String, &str> = BTreeMap::new();
    let mut out = Vec::new();
    for rel in pack.files.keys() {
        let Some(dest) = destination(name, remap, rel) else {
            continue;
        };
        match first.get(&dest.to_lowercase()) {
            Some(other) => out.push(format!(
                "{rel} lands at {dest}, where the pack's {other} lands too, so the pack has no one version of the file"
            )),
            None => {
                first.insert(dest.to_lowercase(), rel);
            }
        }
    }
    // A file that lands at a directory above another file of the pack.
    for (dest, rel) in &first {
        let below = format!("{dest}/");
        if let Some((_, other)) = first
            .range(below.clone()..)
            .next()
            .filter(|(d, _)| d.starts_with(&below))
        {
            out.push(format!(
                "{rel} lands at {dest}, a directory that holds the pack's {other}, so the pack has no one version of the path"
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::pack::PackManifest;

    fn pack(files: &[(&str, &[u8])]) -> PackFiles {
        PackFiles {
            manifest: PackManifest {
                name: "@m/m".into(),
                version: "1.0.0".into(),
                requires_pakf: "0.1.0".into(),
                description: "d".into(),
                depends: BTreeMap::new(),
            },
            files: files
                .iter()
                .map(|(r, b)| (r.to_string(), b.to_vec()))
                .collect(),
            executable: BTreeSet::new(),
        }
    }

    fn remap(pairs: &[(&str, &str)]) -> Remap {
        Remap::new(
            &pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
        .unwrap()
    }

    fn refused_with(files: &[(&str, &[u8])], remap: &Remap) -> Vec<String> {
        check(&pack(files), remap)
    }

    fn refused(files: &[(&str, &[u8])]) -> Vec<String> {
        refused_with(files, &Remap::default())
    }

    // sokf-tests: CLI-2_AC-23
    // sokf-tests: CLI-2_AC-25
    #[test]
    fn a_file_lands_only_where_a_pack_may_write() {
        assert_eq!(
            refused(&[
                ("files/rustfmt.toml", b"x"),
                ("files/.github/workflows/ci.yml", b"x"),
                ("files/knowledge.toml", b"x"),
                ("files/.claude/skills/house/SKILL.md", b"x"),
                ("files/knowledge/notes/a.md", b"x"),
                ("files/knowledge/index.md", b"x"),
                ("files/knowledge/manifest.sokf.yaml", b"x"),
                ("files/.sokf/roots.d/pakf/@m/m/roots.toml", b"x"),
                // A file of a harness lands as any other file does.
                ("files/AGENTS.md", b"x"),
                ("files/claude.md", b"x"),
                ("files/.claude/settings.json", b"x"),
            ]),
            [] as [std::string::String; 0]
        );
        for rel in [
            "files/.git/config",
            "files/.GIT/hooks/pre-commit",
            "files/sub/.git/config",
            "files/.pakf/lock.toml",
            "files/.Pakf/config.toml",
            "files/a/../b",
        ] {
            let found = refused(&[(rel, b"x")]);
            assert_eq!(found.len(), 1, "{rel}: {found:?}");
            assert!(found[0].starts_with(rel), "{found:?}");
        }
        // A binary file lands as any file does, but a pack's instructions
        // are text.
        let binary: &[u8] = &[0xFF, 0xFE, 0x00];
        assert_eq!(
            refused(&[("files/logo.png", binary), ("files/knowledge/a.md", binary)]),
            [] as [std::string::String; 0]
        );
        for bytes in [binary, b"a\x00"] {
            let found = refused(&[("instructions.md", bytes)]);
            assert_eq!(found.len(), 1, "{found:?}");
            assert!(found[0].contains("is binary"), "{found:?}");
        }
    }

    // sokf-tests: CLI-2_AC-23
    // sokf-tests: CLI-2_AC-77
    #[test]
    fn a_remapped_file_lands_only_where_a_pack_may_write() {
        let to_pakf = remap(&[("/conf/", "/.pakf/")]);
        let found = refused_with(&[("files/conf/x.toml", b"x")], &to_pakf);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("lands at /.pakf/x.toml"), "{found:?}");
        // A remap away from pakf's folder lands the file elsewhere.
        let away = remap(&[("/.pakf/", "/conf/")]);
        assert_eq!(
            refused_with(&[("files/.pakf/x.toml", b"x")], &away),
            Vec::<String>::new()
        );
        let to_root = remap(&[("/a.md", "/")]);
        let found = refused_with(&[("files/a.md", b"x")], &to_root);
        assert!(found[0].contains("at the repository root"), "{found:?}");
    }

    // sokf-tests: CLI-2_AC-81
    // sokf-tests: CLI-2_AC-23
    #[test]
    fn a_pack_writes_in_another_tools_folder_and_in_pakfs_only_at_its_id() {
        for rel in [
            "files/.sokf/roots.d/pakf/@m/m/roots.toml",
            "files/.sokf/formats.d/pakf/@m/m/a/formats.yaml",
            "files/tools/x.d/pakf/@m/m/x",
            "files/.pakf/ignore.d/@m/m/ignore",
            "files/.pakf/ignore.d/@m/m/more/ignore",
            "files/.sokf/roots.d/mine.toml",
            "files/.sokf/pakf/@other/x/a",
        ] {
            assert_eq!(refused(&[(rel, b"x")]), Vec::<String>::new(), "{rel}");
        }
        for (rel, says) in [
            (
                "files/.sokf/roots.d/pakf/@other/x/roots.toml",
                "outside /.sokf/roots.d/pakf/@m/m/",
            ),
            (
                "files/.sokf/Formats.D/PAKF/@m/m",
                "outside /.sokf/Formats.D/PAKF/@m/m/",
            ),
            (
                "files/.sokf/roots.d/pakf/roots.toml",
                "outside /.sokf/roots.d/pakf/@m/m/",
            ),
            ("files/x/y.d/pakf/@M/m/q", "outside /x/y.d/pakf/@m/m/"),
            (
                "files/.pakf/ignore.d/@other/x/ignore",
                "below /.pakf/ignore.d/@m/m/",
            ),
            ("files/.pakf/ignore", "among pakf's own files"),
            // pakf reads its folder of ignores in one case of its letters.
            ("files/.PAKF/Ignore.D/@m/m/ignore", "among pakf's own files"),
            ("files/.pakf/ignore.d/@m/m", "among pakf's own files"),
        ] {
            let found = refused(&[(rel, b"x")]);
            assert_eq!(found.len(), 1, "{rel}: {found:?}");
            assert!(
                found[0].starts_with(rel) && found[0].contains(says),
                "{found:?}"
            );
        }
    }

    // sokf-tests: CLI-2_AC-69
    #[test]
    fn no_two_files_of_a_pack_land_at_one_destination() {
        let p = pack(&[
            ("files/docs/guide.md", b"x"),
            ("files/knowledge/Guide.md", b"x"),
            ("files/knowledge/other.md", b"x"),
        ]);
        assert_eq!(collisions(&p, &Remap::default()), Vec::<String>::new());
        let found = collisions(&p, &remap(&[("/knowledge/", "/docs/")]));
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].contains("where the pack's files/docs/guide.md lands too"),
            "{found:?}"
        );
        // A file at a directory of the pack's files has no place either.
        let p = pack(&[("files/a.md", b"x"), ("files/docs/x.md", b"x")]);
        let found = collisions(&p, &remap(&[("/a.md", "/Docs")]));
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].contains("a directory that holds the pack's files/docs/x.md"),
            "{found:?}"
        );
    }
}
