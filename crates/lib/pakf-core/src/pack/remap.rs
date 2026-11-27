//! This module reads the remaps of a `[[pack]]` entry, its `paths`: a table
//! from a prefix of the paths of a pack's files to the prefix at which they
//! land, and maps a path of the pack forward to its destination and a path
//! of the project back to its path in the pack (the decision "Remapped
//! prefixes" of design-pack).

use std::collections::BTreeMap;

use crate::RepoPath;

/// A `Remap` is the remaps of one entry: the table as the entry writes it,
/// which the lock records, and each prefix and its mapped prefix, both
/// without a trailing `/`, except the root `/`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Remap {
    /// The table as written.
    written: BTreeMap<String, String>,
    /// Each prefix, with its mapped prefix.
    pairs: Vec<(String, String)>,
}

/// `prefix` returns the prefix `s` without its trailing `/`, or why it
/// isn't a `/`-rooted path.
fn prefix(s: &str) -> Result<String, String> {
    let trimmed = if s.len() > 1 {
        s.strip_suffix('/').unwrap_or(s)
    } else {
        s
    };
    // A path of more than the root ends in at most one `/`.
    let trimmed = if trimmed.len() < s.len() && trimmed.ends_with('/') {
        ""
    } else {
        trimmed
    };
    RepoPath::parse(trimmed)
        .map(|p| p.as_str().to_string())
        .ok_or_else(|| format!("`{s}` is not a /-rooted path"))
}

/// `holds` returns whether the prefix `p` holds the path `path`: the path
/// is the prefix or lies below it.
fn holds(p: &str, path: &str) -> bool {
    p == "/" || path == p || path.strip_prefix(p).is_some_and(|r| r.starts_with('/'))
}

/// `swap` returns the path `path`, which the prefix `from` holds, with
/// `from` replaced by `to`.
fn swap(from: &str, to: &str, path: &str) -> String {
    let rest = if from == "/" {
        path
    } else {
        &path[from.len()..]
    };
    match (to, rest) {
        ("/", "") => "/".to_string(),
        ("/", rest) => rest.to_string(),
        (to, rest) => format!("{to}{rest}"),
    }
}

impl Remap {
    /// `new` reads the remaps of the table `written`. It refuses a key or a
    /// value that isn't a `/`-rooted path, with or without a trailing `/`,
    /// and two keys or two values that name one prefix, because the longest
    /// prefix can't choose between them.
    // sokf-implements: CONF-1_AC-3
    // sokf-implements: CONF-1_AC-4
    // sokf-implements: CONF-1_AC-5
    pub fn new(written: &BTreeMap<String, String>) -> Result<Remap, String> {
        let mut pairs: Vec<(String, String)> = Vec::new();
        let mut from: BTreeMap<String, &str> = BTreeMap::new();
        let mut to: BTreeMap<String, &str> = BTreeMap::new();
        for (key, value) in written {
            let (k, v) = (prefix(key)?, prefix(value)?);
            if let Some(other) = from.insert(k.clone(), key) {
                return Err(format!(
                    "the prefixes `{other}` and `{key}` name one prefix"
                ));
            }
            if let Some(other) = to.insert(v.clone(), value) {
                return Err(format!(
                    "the mapped prefixes `{other}` and `{value}` name one prefix"
                ));
            }
            pairs.push((k, v));
        }
        Ok(Remap {
            written: written.clone(),
            pairs,
        })
    }

    /// `written` returns the table as the entry writes it.
    pub fn written(&self) -> &BTreeMap<String, String> {
        &self.written
    }

    /// `forward` returns the destination of the path `path` of a pack's
    /// file from the repository root: the path with the longest prefix that
    /// holds it replaced by its mapped prefix, or the path itself where no
    /// prefix holds it.
    // sokf-implements: CLI-2_AC-77
    // sokf-implements: CLI-2_AC-78
    pub fn forward(&self, path: &str) -> String {
        self.pairs
            .iter()
            .filter(|(from, _)| holds(from, path))
            .max_by_key(|(from, _)| from.len())
            .map_or_else(|| path.to_string(), |(from, to)| swap(from, to, path))
    }

    /// `backward` returns the path from the repository root of the pack's
    /// file that lands at `dest`: the first of the paths that the mapped
    /// prefixes that hold it give, from the longest prefix, and of the path
    /// itself, that maps forward to `dest`. It refuses a path at which no
    /// file of the pack lands, where each of those paths maps forward
    /// elsewhere, and names where the first lands.
    // sokf-implements: CLI-9_AC-30
    pub fn backward(&self, dest: &str) -> Result<String, String> {
        let mut mapped: Vec<&(String, String)> = self
            .pairs
            .iter()
            .filter(|(_, to)| holds(to, dest))
            .collect();
        mapped.sort_by_key(|(_, to)| std::cmp::Reverse(to.len()));
        let candidates: Vec<String> = mapped
            .iter()
            .map(|(from, to)| swap(to, from, dest))
            .chain(std::iter::once(dest.to_string()))
            .collect();
        if let Some(back) = candidates.iter().find(|c| self.forward(c) == dest) {
            return Ok(back.clone());
        }
        let back = &candidates[0];
        Err(format!(
            "the paths of the pack's entry land the pack's {back} at {}, so no file of the pack lands at {dest}",
            self.forward(back)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remap(pairs: &[(&str, &str)]) -> Result<Remap, String> {
        Remap::new(
            &pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    // sokf-tests: CLI-2_AC-77
    // sokf-tests: CLI-2_AC-78
    #[test]
    fn a_path_lands_by_its_longest_prefix() {
        let r = remap(&[
            ("/knowledge/", "/docs/"),
            ("/knowledge/schemas", "/docs/kinds/"),
            ("/rustfmt.toml", "/.config/rustfmt.toml"),
        ])
        .unwrap();
        for (path, dest) in [
            ("/knowledge/a.md", "/docs/a.md"),
            ("/knowledge/notes/b.md", "/docs/notes/b.md"),
            ("/knowledge/schemas/issue.md", "/docs/kinds/issue.md"),
            ("/rustfmt.toml", "/.config/rustfmt.toml"),
            // A prefix holds whole segments only.
            ("/knowledge.toml", "/knowledge.toml"),
            ("/knowledgebase/a.md", "/knowledgebase/a.md"),
            ("/deny.toml", "/deny.toml"),
        ] {
            assert_eq!(r.forward(path), dest, "{path}");
        }
        let root = remap(&[("/", "/vendor/")]).unwrap();
        assert_eq!(root.forward("/a/b.md"), "/vendor/a/b.md");
        let up = remap(&[("/pack/", "/")]).unwrap();
        assert_eq!(up.forward("/pack/a.md"), "/a.md");
        assert_eq!(Remap::default().forward("/a.md"), "/a.md");
    }

    // sokf-tests: CLI-9_AC-30
    #[test]
    fn a_path_of_the_project_maps_back_by_its_longest_mapped_prefix() {
        let r = remap(&[
            ("/knowledge/", "/docs/"),
            ("/knowledge/schemas", "/docs/kinds/"),
        ])
        .unwrap();
        for (dest, path) in [
            ("/docs/new.md", "/knowledge/new.md"),
            ("/docs/kinds/new.md", "/knowledge/schemas/new.md"),
            ("/src/main.rs", "/src/main.rs"),
        ] {
            assert_eq!(r.backward(dest).as_deref(), Ok(path), "{dest}");
            assert_eq!(r.forward(path), dest);
        }
        // The pack's own path of a remapped file lands elsewhere.
        let e = r.backward("/knowledge/new.md").unwrap_err();
        assert!(e.contains("at /docs/new.md"), "{e}");
        // A longer prefix takes the path that a shorter mapped prefix gives,
        // so the path itself lands there.
        assert_eq!(
            r.backward("/docs/schemas/a.md").as_deref(),
            Ok("/docs/schemas/a.md")
        );
        // Where the longest mapped prefix gives a path that lands elsewhere,
        // the path itself lands there.
        let r = remap(&[("/a/", "/x/"), ("/a/q/", "/z/")]).unwrap();
        assert_eq!(r.backward("/x/q/r").as_deref(), Ok("/x/q/r"));
        assert_eq!(r.backward("/x/s").as_deref(), Ok("/a/s"));
    }

    // sokf-tests: CONF-1_AC-4
    // sokf-tests: CONF-1_AC-5
    #[test]
    fn a_remap_refuses_a_bad_prefix_and_two_of_one_prefix() {
        for bad in [
            ("knowledge/", "/docs/"),
            ("/k/", "docs"),
            ("/k/../x", "/d"),
            ("//", "/d"),
            ("/k//", "/d"),
        ] {
            let e = remap(&[bad]).unwrap_err();
            assert!(e.contains("is not a /-rooted path"), "{e}");
        }
        let e = remap(&[("/knowledge", "/a"), ("/knowledge/", "/b")]).unwrap_err();
        assert!(e.contains("`/knowledge` and `/knowledge/`"), "{e}");
        let e = remap(&[("/a/", "/docs"), ("/b/", "/docs/")]).unwrap_err();
        assert!(e.contains("`/docs` and `/docs/`"), "{e}");
    }
}
