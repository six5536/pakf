//! This module reads `pakf.yaml`, the manifest of a pack, and
//! compares the versions that the manifest gives.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    Error, Result,
    yaml::{self, Spanned},
};

/// `MANIFEST` is the file name of a pack's manifest.
pub const MANIFEST: &str = "pakf.yaml";

/// `EDITABLE` is the key of a manifest that lists the pack's editable
/// files, and the word of their kind in the reports.
pub const EDITABLE: &str = "editable";

/// `KEYS` lists the keys of a manifest. Each is required except `depends`
/// and `editable`.
const KEYS: &[&str] = &[
    "name",
    "version",
    "requires-pakf",
    "description",
    "depends",
    EDITABLE,
];

/// A `PackManifest` is the content of `pakf.yaml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackManifest {
    /// The pack's id, `@<scope>/<name>`, where both parts are slugs.
    pub name: String,
    /// The pack's version, in semantic versioning.
    pub version: String,
    /// The oldest version of pakf that the pack works with.
    pub requires_pakf: String,
    /// One sentence that says what the pack holds.
    pub description: String,
    /// The packs that the pack depends on, by id, each with the oldest
    /// version that it works with.
    pub depends: BTreeMap<String, String>,
    /// The `/`-rooted path of each editable file: a file of `files/` whose
    /// content every project fills in, such as `/Cargo.toml`.
    pub editable: BTreeSet<String>,
}

impl PackManifest {
    /// `is_editable` returns whether the manifest declares the pack's file
    /// `rel`, a path in the pack, editable.
    pub fn is_editable(&self, rel: &str) -> bool {
        rel.strip_prefix("files")
            .is_some_and(|p| p.starts_with('/') && self.editable.contains(p))
    }
}

/// `parse_manifest` reads the text of a manifest. `label` names the file in
/// an error. It refuses a manifest that lacks a required key, has an
/// unknown key, or gives a name that isn't an id `@<scope>/<name>` or a
/// version that isn't `MAJOR.MINOR.PATCH`, and a `depends` that isn't a map
/// from the id of another pack to such a version. A manifest that doesn't
/// parse because a value begins with an unquoted `@` gets the advice to
/// quote the id.
// sokf-implements: CONF-3_AC-4
// sokf-implements: CONF-3_AC-5
// sokf-implements: CONF-3_AC-1
// sokf-implements: CONF-3_AC-3
pub fn parse_manifest(text: &str, label: &str) -> Result<PackManifest> {
    let fail = |m: String| Error::Pack(format!("{label}: {m}"));
    // sokf-implements: CONF-3_AC-6
    let doc = yaml::parse(text, 1).map_err(|e| {
        let unquoted = starts_with_at(text);
        if unquoted {
            fail(format!(
                "{e}; quote a pack's id, as in `name: \"@acme/method\"`"
            ))
        } else {
            fail(e.to_string())
        }
    })?;
    let map = doc
        .as_map()
        .ok_or_else(|| fail("the manifest is not a map".into()))?;
    for (key, _) in map {
        if !KEYS.contains(&key.value.as_str()) {
            return Err(fail(format!("unknown key `{}`", key.value)));
        }
    }
    let get = |key: &str| -> Result<String> {
        doc.get(key)
            .and_then(Spanned::scalar_text)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| fail(format!("the manifest has no `{key}`")))
    };
    let manifest = PackManifest {
        name: get("name")?,
        version: get("version")?,
        requires_pakf: get("requires-pakf")?,
        description: get("description")?,
        depends: depends(&doc, &fail)?,
        editable: BTreeSet::new(),
    };
    if !is_pack_id(&manifest.name) {
        return Err(fail(not_an_id("the name", &manifest.name)));
    }
    for (key, v) in [
        ("version", &manifest.version),
        ("requires-pakf", &manifest.requires_pakf),
    ] {
        if parse_version(v).is_none() {
            return Err(fail(format!(
                "the `{key}` `{v}` is not a version of the form MAJOR.MINOR.PATCH"
            )));
        }
    }
    let manifest = PackManifest {
        editable: editable(&doc, &fail)?,
        ..manifest
    };
    Ok(manifest)
}

/// `editable` reads the optional `editable` of the manifest `doc`: a list
/// of the `/`-rooted paths of files of `files/`. It refuses any other value
/// and a path given twice, and leaves to the read of the pack's tree the
/// refusal of a path that names no file of the pack.
// sokf-implements: CONF-3_AC-11
// sokf-implements: CONF-3_AC-12
fn editable(doc: &Spanned, fail: &dyn Fn(String) -> Error) -> Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    let Some(value) = doc.get(EDITABLE) else {
        return Ok(out);
    };
    let not_paths = || {
        fail(format!(
            "`{EDITABLE}` is not a list of the `/`-rooted paths of files of `files/`, such as `[\"/Cargo.toml\"]`"
        ))
    };
    let items = value.as_seq().ok_or_else(not_paths)?;
    for item in items {
        let path = item
            .as_str()
            .filter(|p| *p != "/" && crate::RepoPath::parse(p).is_some())
            .ok_or_else(not_paths)?;
        // sokf-implements: CONF-3_AC-13
        if !out.insert(path.to_string()) {
            return Err(fail(format!("`{EDITABLE}` names {path} twice")));
        }
    }
    Ok(out)
}

/// `depends` reads the optional `depends` of the manifest `doc`: a map from
/// the id of a pack to the oldest version of it that the pack works with.
// sokf-implements: CONF-3_AC-7
// sokf-implements: CONF-3_AC-5
fn depends(doc: &Spanned, fail: &dyn Fn(String) -> Error) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    let Some(value) = doc.get("depends") else {
        return Ok(out);
    };
    let map = value.as_map().ok_or_else(|| {
        fail("`depends` is not a map from the id of a pack to its version".into())
    })?;
    for (key, v) in map {
        let name = key.value.clone();
        let version = v.scalar_text().unwrap_or_default();
        if !is_pack_id(&name) {
            return Err(fail(not_an_id("the key in `depends`", &name)));
        }
        if parse_version(&version).is_none() {
            return Err(fail(format!(
                "the version `{version}` of `{name}` in `depends` is not of the form MAJOR.MINOR.PATCH"
            )));
        }
        out.insert(name, version);
    }
    Ok(out)
}

/// `starts_with_at` returns whether a plain key or value of the YAML `text`
/// begins with `@`: an `@` at the start of a line, after the `-` of a list
/// item there, or after `:`, `{`, `[`, or `,`, with only spaces between.
fn starts_with_at(text: &str) -> bool {
    let mut after_indicator = true;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '@' if after_indicator => return true,
            // A quoted scalar and a comment hold no plain value.
            '"' | '\'' => {
                let _ = chars.by_ref().find(|&q| q == c || q == '\n');
                after_indicator = false;
            }
            '#' => {
                let _ = chars.by_ref().find(|&q| q == '\n');
                after_indicator = true;
            }
            // A `-` begins a list item only where a key could begin, so it
            // leaves the state as a space does.
            ' ' | '\t' | '-' => {}
            '\n' | ':' | '{' | '[' | ',' => after_indicator = true,
            _ => after_indicator = false,
        }
    }
    false
}

/// `not_an_id` returns the error for `what`, the value `value` that isn't the
/// id of a pack, which names the form `@scope/name`, and the id to write for
/// a bare slug.
fn not_an_id(what: &str, value: &str) -> String {
    let hint = if is_slug(value) {
        format!(", such as `@acme/{value}`, where `acme` is the slug of the pack's owner")
    } else {
        String::new()
    };
    format!(
        "{what} `{value}` is not the id of a pack: an id has the form `@scope/name`, two slugs of lowercase words of letters and digits joined by `-`{hint}"
    )
}

/// `is_pack_id` returns whether `s` is the id of a pack, `@<scope>/<name>`,
/// where both parts are slugs.
// sokf-implements: CLI-1_AC-29
pub fn is_pack_id(s: &str) -> bool {
    split_id(s).is_some()
}

/// `ids_named` returns each id of `ids` whose name, after its scope, is the
/// bare name `name`, for the hint of an error.
pub fn ids_named<'a>(ids: impl Iterator<Item = &'a str>, name: &str) -> Vec<&'a str> {
    ids.filter(|id| split_id(id).is_some_and(|(_, n)| n == name))
        .collect()
}

/// `split_id` returns the scope and the name of the pack id `s`, without the
/// `@`, or none when `s` isn't an id.
pub fn split_id(s: &str) -> Option<(&str, &str)> {
    let (scope, name) = s.strip_prefix('@')?.split_once('/')?;
    (is_slug(scope) && is_slug(name)).then_some((scope, name))
}

/// `is_slug` returns whether `s` is lowercase words of letters and digits,
/// joined by `-`.
pub fn is_slug(s: &str) -> bool {
    !s.is_empty()
        && s.split('-').all(|w| {
            !w.is_empty()
                && w.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

/// `parse_version` reads `MAJOR.MINOR.PATCH`, with any pre-release or build
/// suffix after `-` or `+` left out of the comparison.
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// `tool_version` returns the version of the running tool.
pub fn tool_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// `requires_newer` returns whether `required` names a version newer than
/// `tool`. A version that doesn't parse counts as newer.
// sokf-implements: CONF-3_AC-8
pub fn requires_newer(required: &str, tool: &str) -> bool {
    match (parse_version(required), parse_version(tool)) {
        (Some(r), Some(t)) => r > t,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "name: \"@acme/method\"\nversion: 1.2.0\nrequires-pakf: 0.1.0\ndescription: The method of a store.\n";

    #[test]
    fn a_manifest_reads_its_four_keys() {
        let m = parse_manifest(GOOD, "pakf.yaml").unwrap();
        assert_eq!(m.name, "@acme/method");
        assert_eq!(m.version, "1.2.0");
        assert_eq!(m.requires_pakf, "0.1.0");
        assert_eq!(m.description, "The method of a store.");
        assert!(m.depends.is_empty());
    }

    // sokf-tests: CONF-3_AC-7
    #[test]
    fn a_manifest_reads_the_packs_that_it_depends_on() {
        let text = format!("{GOOD}depends:\n  \"@acme/house-base\": 1.4.0\n");
        let m = parse_manifest(&text, "pakf.yaml").unwrap();
        assert_eq!(
            m.depends,
            BTreeMap::from([("@acme/house-base".to_string(), "1.4.0".to_string())])
        );
        for bad in [
            "depends: [a]\n",
            "depends:\n  \"@acme/Base\": 1.0.0\n",
            "depends:\n  \"@acme/base\": 1.0\n",
            "depends:\n  base: 1.0.0\n",
        ] {
            let e = parse_manifest(&format!("{GOOD}{bad}"), "x/pakf.yaml").unwrap_err();
            assert!(e.to_string().contains("depends"), "{bad:?} -> {e}");
        }
    }

    // sokf-tests: CONF-3_AC-11
    // sokf-tests: CONF-3_AC-12
    // sokf-tests: CONF-3_AC-13
    #[test]
    fn a_manifest_reads_its_editable_files_as_exact_paths() {
        let text = format!("{GOOD}editable:\n  - /Cargo.toml\n  - /docs/LICENSE\n");
        let m = parse_manifest(&text, "pakf.yaml").unwrap();
        assert_eq!(
            m.editable,
            BTreeSet::from(["/Cargo.toml".to_string(), "/docs/LICENSE".to_string()])
        );
        assert!(m.is_editable("files/Cargo.toml"));
        assert!(m.is_editable("files/docs/LICENSE"));
        assert!(!m.is_editable("files/docs"));
        assert!(!m.is_editable("instructions.md"));
        assert!(!m.is_editable("filesCargo.toml"));
        assert!(
            parse_manifest(GOOD, "pakf.yaml")
                .unwrap()
                .editable
                .is_empty()
        );
        for bad in [
            "editable: /Cargo.toml\n",
            "editable: {a: b}\n",
            "editable: [1]\n",
            "editable: [Cargo.toml]\n",
            "editable: [\"/\"]\n",
            "editable: [/docs/]\n",
            "editable: [/a/../b]\n",
        ] {
            let e = parse_manifest(&format!("{GOOD}{bad}"), "x/pakf.yaml")
                .unwrap_err()
                .to_string();
            assert!(e.contains("`editable` is not a list"), "{bad:?} -> {e}");
        }
        let e = parse_manifest(&format!("{GOOD}editable: [/a, /a]\n"), "x/pakf.yaml")
            .unwrap_err()
            .to_string();
        assert!(e.contains("`editable` names /a twice"), "{e}");
    }

    // sokf-tests: CONF-3_AC-4
    #[test]
    fn a_bad_manifest_is_refused_with_its_label() {
        for text in [
            "name: [",
            "- a\n",
            "name: \"@acme/method\"\n",
            "name: \"@acme/Method\"\nversion: 1.0.0\nrequires-pakf: 0.1.0\ndescription: d\n",
            "name: \"@acme/method\"\nversion: 1.0\nrequires-pakf: 0.1.0\ndescription: d\n",
            "name: \"@acme/method\"\nversion: 1.0.0\nrequires-pakf: 0.1.0\ndescription: d\nextra: 1\n",
        ] {
            let e = parse_manifest(text, "x/pakf.yaml").unwrap_err();
            assert!(
                e.to_string().starts_with("pack: x/pakf.yaml: "),
                "{text:?} -> {e}"
            );
        }
    }

    // sokf-tests: CONF-3_AC-5
    #[test]
    fn a_name_without_a_scope_is_refused_with_the_form_of_an_id() {
        for (name, hint) in [
            ("method", "such as `@acme/method`"),
            ("\"acme/method\"", "`@scope/name`"),
            ("\"@acme\"", "`@scope/name`"),
            ("\"@acme/method/x\"", "`@scope/name`"),
            ("\"@/method\"", "`@scope/name`"),
        ] {
            let text =
                format!("name: {name}\nversion: 1.0.0\nrequires-pakf: 0.1.0\ndescription: d\n");
            let e = parse_manifest(&text, "x/pakf.yaml")
                .unwrap_err()
                .to_string();
            assert!(e.contains("is not the id of a pack"), "{name} -> {e}");
            assert!(e.contains(hint), "{name} -> {e}");
        }
        let text = format!("{GOOD}depends:\n  base: 1.0.0\n");
        let e = parse_manifest(&text, "x/pakf.yaml")
            .unwrap_err()
            .to_string();
        assert!(e.contains("the key in `depends` `base`"), "{e}");
        assert!(e.contains("`@acme/base`"), "{e}");
    }

    // sokf-tests: CONF-3_AC-6
    #[test]
    fn an_unquoted_id_gets_the_advice_to_quote_it() {
        for text in [
            "name: @acme/method\nversion: 1.0.0\nrequires-pakf: 0.1.0\ndescription: d\n",
            "name: \"@acme/method\"\nversion: 1.0.0\nrequires-pakf: 0.1.0\ndescription: d\ndepends:\n  @acme/base: 1.0.0\n",
            "name: \"@acme/method\"\nversion: 1.0.0\nrequires-pakf: 0.1.0\ndescription: d\ndepends: {@acme/base: 1.0.0}\n",
        ] {
            let e = parse_manifest(text, "x/pakf.yaml").unwrap_err().to_string();
            assert!(e.contains("quote a pack's id"), "{text:?} -> {e}");
        }
        for text in [
            "name: [",
            "description: Owned by - @team\nname: [",
            "description: \"Owned by: @team\"\nname: [",
            "# contact: @team\nname: [",
        ] {
            let e = parse_manifest(text, "x/pakf.yaml").unwrap_err().to_string();
            assert!(!e.contains("quote"), "{text:?} -> {e}");
        }
    }

    // sokf-tests: CLI-1_AC-29
    // sokf-tests: CLI-1_AC-30
    #[test]
    fn an_id_is_a_scope_and_a_name() {
        assert_eq!(split_id("@acme/method"), Some(("acme", "method")));
        assert_eq!(split_id("@pakf/pakf"), Some(("pakf", "pakf")));
        assert_ne!(split_id("@acme/method"), split_id("@other/method"));
        for bad in [
            "method",
            "acme/method",
            "@acme",
            "@acme/",
            "@/m",
            "@Acme/m",
            "@a/b/c",
            "@a/b-",
        ] {
            assert!(!is_pack_id(bad), "{bad}");
        }
    }

    #[test]
    fn versions_compare_by_their_three_numbers() {
        assert!(!requires_newer("0.1.3", "0.1.3"));
        assert!(!requires_newer("0.1.2", "0.1.3"));
        assert!(requires_newer("0.2.0", "0.1.3"));
        assert!(requires_newer("1.0.0", "0.9.9"));
        assert!(!requires_newer("0.1.3-rc.1", "0.1.3"));
        assert!(requires_newer("x", "0.1.3"));
        assert_eq!(parse_version("1.2.3+build"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2.3.4"), None);
    }

    #[test]
    fn a_slug_is_lowercase_words_joined_by_hyphens() {
        assert!(is_slug("pakf"));
        assert!(is_slug("my-method-2"));
        assert!(!is_slug(""));
        assert!(!is_slug("-a"));
        assert!(!is_slug("a--b"));
        assert!(!is_slug("A"));
    }
}
