//! This module reads the `[[pack]]` entries of `/.pakf/config.toml`, which
//! declare the packs of the repository in order, and edits them
//! for `pakf add` and `pakf remove`, keeping every other line of
//! the file.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

use super::file::config_path;
use crate::{Error, RepoRoot, Result, source::read_optional};

// sokf:begin config-pack
/// `PackEntry` holds one `[[pack]]` entry: a pack of the
/// repository, by its name, with the source, the ref, and the directory
/// that `pakf sync` fetches it from, and the remaps of the paths at which
/// its files land.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackEntry {
    /// The pack's id, `@<scope>/<name>`, which its manifest gives.
    pub name: String,
    /// A git URL or path. A relative path resolves from the repository
    /// root.
    pub source: String,
    /// The branch, tag, or commit of a git source. Default: the source's
    /// default branch.
    #[serde(rename = "ref", default)]
    pub git_ref: Option<String>,
    /// The pack's directory in a git source. Default: the source's root.
    #[serde(default)]
    pub path: Option<String>,
    /// The remaps of the paths of the pack's files, from each prefix of
    /// their paths from the repository root to the prefix at which they
    /// land, such as `{ "/knowledge/" = "/docs/" }`. Default: none.
    #[serde(default)]
    pub paths: BTreeMap<String, String>,
}
// sokf:end config-pack

/// `PacksFile` is the part of the configuration that this module reads: the
/// `[[pack]]` entries, and no other key, such as a table that
/// agent-harness-adapter reads for pakf.
// sokf-implements: CONF-1_AC-6
#[derive(Debug, Default, Deserialize)]
struct PacksFile {
    #[serde(default)]
    pack: Vec<PackEntry>,
}

/// `read_packs` returns the `[[pack]]` entries of `/.pakf/config.toml`, in
/// their order, and none when the file is absent. It refuses an entry that
/// lacks `name` or `source` or has another key, `store` among them, a name
/// that isn't an id `@<scope>/<name>`, remaps that `Remap::new` refuses,
/// and a second entry of one name.
// sokf-implements: CONF-1_AC-1
// sokf-implements: CONF-1_AC-2
pub fn read_packs(root: &RepoRoot) -> Result<Vec<PackEntry>> {
    let path = config_path();
    let file = parse_packs(root)?;
    let mut names = BTreeSet::new();
    for p in &file.pack {
        // sokf-implements: CLI-1_AC-29
        if !crate::pack::is_pack_id(&p.name) {
            let hint = if crate::pack::is_slug(&p.name) {
                format!(
                    ", such as `@acme/{}`; give each [[pack]] entry the id that its pack's manifest declares",
                    p.name
                )
            } else {
                String::new()
            };
            return Err(Error::config(
                path,
                format!(
                    "the pack name `{}` is not the id of a pack, of the form `@scope/name`{hint}",
                    p.name
                ),
            ));
        }
        // The whole id names the pack, so two scopes give two packs.
        // sokf-implements: CLI-1_AC-30
        if !names.insert(p.name.clone()) {
            return Err(Error::config(
                path,
                format!("two [[pack]] entries declare the pack {}", p.name),
            ));
        }
        if let Err(why) = crate::pack::Remap::new(&p.paths) {
            return Err(Error::config(
                path,
                format!("the paths of the pack {}: {why}", p.name),
            ));
        }
    }
    Ok(file.pack)
}

/// `declared_order` returns the names of the `[[pack]]` entries in their
/// order, as they stand, for a command that only reads the packs. It
/// returns none when the file is absent or doesn't parse.
pub fn declared_order(root: &RepoRoot) -> Vec<String> {
    parse_packs(root)
        .map(|f| f.pack.into_iter().map(|e| e.name).collect())
        .unwrap_or_default()
}

/// `parse_packs` reads the `[[pack]]` entries of `/.pakf/config.toml`
/// without checking them, and none when the file is absent. It refuses an
/// entry that names `store`, which pakf no longer reads, with how to move
/// the pack's files.
fn parse_packs(root: &RepoRoot) -> Result<PacksFile> {
    let path = config_path();
    let Some(text) = read_optional(&path.to_fs(root))? else {
        return Ok(PacksFile::default());
    };
    let value: toml::Table =
        toml::from_str(&text).map_err(|e| Error::config(path.clone(), e.message()))?;
    // sokf-implements: CLI-2_AC-79
    let stored = value
        .get("pack")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_table)
        .find(|t| t.contains_key("store"));
    if let Some(entry) = stored {
        let name = entry
            .get("name")
            .and_then(toml::Value::as_str)
            .unwrap_or("a pack");
        return Err(Error::config(
            path,
            format!(
                "the [[pack]] entry of {name} names `store`, which pakf no longer reads; a pack is one tree of files, so move the pack's files under files/ at the paths where they land, remove `store`, and remap a prefix with `paths` where the project keeps the files elsewhere"
            ),
        ));
    }
    value
        .try_into()
        .map_err(|e: toml::de::Error| Error::config(path, e.message()))
}

/// `add_entry` returns the text of the configuration `text`, or of an
/// absent file when it is `None`, with `entry` appended as the last
/// `[[pack]]` entry. Every other line, comment, and blank line stays, and the
/// text keeps the line ending of the file.
// sokf-implements: CLI-3_AC-3
pub fn add_entry(text: Option<&str>, entry: &PackEntry) -> Result<String> {
    let eol = eol_of(text.unwrap_or_default());
    added(text, entry).map(|out| with_eol(&out, eol))
}

/// `added` is `add_entry` with LF line endings for the entry.
fn added(text: Option<&str>, entry: &PackEntry) -> Result<String> {
    let mut doc = parse(text)?;
    let mut table = Table::new();
    table.insert("name", value(&entry.name));
    table.insert("source", value(&entry.source));
    for (key, v) in [("ref", &entry.git_ref), ("path", &entry.path)] {
        if let Some(v) = v {
            table.insert(key, value(v));
        }
    }
    if !entry.paths.is_empty() {
        let mut paths = toml_edit::InlineTable::new();
        for (from, to) in &entry.paths {
            paths.insert(from, to.as_str().into());
        }
        table.insert("paths", value(paths));
    }
    match doc.get_mut("pack") {
        None => {
            // A file that holds no table keeps its comments as the trailing
            // text of the document, which would follow the new entry, so
            // the entry goes after the file's text instead.
            let empty = doc.as_table().is_empty();
            let mut packs = ArrayOfTables::new();
            packs.push(table);
            if empty {
                let mut alone = DocumentMut::new();
                alone.insert("pack", Item::ArrayOfTables(packs));
                let before = text.unwrap_or_default();
                if before.trim().is_empty() {
                    return Ok(alone.to_string());
                }
                // A blank line comes between the file's text and the entry.
                let gap = if before.ends_with("\n\n") || before.ends_with("\n\r\n") {
                    ""
                } else if before.ends_with('\n') {
                    "\n"
                } else {
                    "\n\n"
                };
                return Ok(format!("{before}{gap}{alone}"));
            }
            doc.insert("pack", Item::ArrayOfTables(packs));
        }
        Some(Item::ArrayOfTables(packs)) => packs.push(table),
        Some(_) => return Err(not_a_list()),
    }
    Ok(doc.to_string())
}

/// `remove_entry` returns the text of the configuration `text` without the
/// `[[pack]]` entry of the pack `name`, or `None` when no entry declares
/// it. Every other line, comment, and blank line stays, and the text keeps
/// the line ending of the file.
// sokf-implements: CLI-3_AC-3
pub fn remove_entry(text: &str, name: &str) -> Result<Option<String>> {
    let eol = eol_of(text);
    removed(text, name).map(|out| out.map(|o| with_eol(&o, eol)))
}

/// `removed` is `remove_entry` before the line ending of the file.
fn removed(text: &str, name: &str) -> Result<Option<String>> {
    let mut doc = parse(Some(text))?;
    let Some(item) = doc.get_mut("pack") else {
        return Ok(None);
    };
    let Item::ArrayOfTables(packs) = item else {
        return Err(not_a_list());
    };
    let Some(i) = packs
        .iter()
        .position(|t| t.get("name").and_then(Item::as_str) == Some(name))
    else {
        return Ok(None);
    };
    // The comments before the last entry of a file that holds no other
    // table stay, as the file's own text.
    let prefix = packs
        .get(i)
        .and_then(|t| t.decor().prefix())
        .and_then(|p| p.as_str())
        .unwrap_or_default()
        .to_string();
    packs.remove(i);
    // The comments before the first entry, such as the file's header, stay
    // before the entry that comes first now.
    if i == 0
        && !prefix.trim().is_empty()
        && let Some(next) = packs.get_mut(0)
    {
        let own = next
            .decor()
            .prefix()
            .and_then(|p| p.as_str())
            .unwrap_or_default()
            .to_string();
        next.decor_mut().set_prefix(format!("{prefix}{own}"));
    }
    if packs.is_empty() {
        doc.remove("pack");
        if doc.as_table().is_empty() && !prefix.trim().is_empty() {
            return Ok(Some(format!("{prefix}{doc}")));
        }
    }
    Ok(Some(doc.to_string()))
}

/// `eol_of` returns the line ending of the first line of the text `text`,
/// or LF when it has no line break.
fn eol_of(text: &str) -> &'static str {
    match crate::LineEnding::detect_bytes(text.as_bytes()) {
        crate::LineEnding::CrLf => "\r\n",
        crate::LineEnding::Lf => "\n",
    }
}

/// `with_eol` returns `text` with each line ending as `eol`.
fn with_eol(text: &str, eol: &str) -> String {
    if eol == "\n" {
        text.to_string()
    } else {
        text.replace("\r\n", "\n").replace('\n', eol)
    }
}

fn parse(text: Option<&str>) -> Result<DocumentMut> {
    text.unwrap_or_default()
        .parse::<DocumentMut>()
        .map_err(|e| Error::config(config_path(), e.message()))
}

fn not_a_list() -> Error {
    Error::config(config_path(), "`pack` is not a list of [[pack]] tables")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> PackEntry {
        PackEntry {
            name: name.into(),
            source: format!("git@example.com:acme/{}.git", &name[6..]),
            git_ref: None,
            path: Some("packs/base".into()),
            paths: BTreeMap::new(),
        }
    }

    // sokf-tests: CLI-3_AC-3
    // sokf-tests: CONF-4_AC-1
    #[test]
    fn an_edit_keeps_every_other_line_of_the_file() {
        let text = "# The roots.\n[knowledge]\nroots = [\"/knowledge\"]\n\n# The house's packs.\n[[pack]]\nname = \"@acme/base\" # the base\nsource = \"x\"\n";
        let added = add_entry(Some(text), &entry("@acme/rust")).unwrap();
        assert!(added.starts_with(text), "{added}");
        assert!(
            added.ends_with(
                "\n[[pack]]\nname = \"@acme/rust\"\nsource = \"git@example.com:acme/rust.git\"\npath = \"packs/base\"\n"
            ),
            "{added}"
        );
        let removed = remove_entry(&added, "@acme/base").unwrap().unwrap();
        assert!(removed.contains("# The roots.\n[knowledge]"), "{removed}");
        assert!(!removed.contains("name = \"@acme/base\""), "{removed}");
        assert!(removed.contains("name = \"@acme/rust\""), "{removed}");
        assert_eq!(remove_entry(&removed, "@acme/base").unwrap(), None);
        let none = remove_entry(&removed, "@acme/rust").unwrap().unwrap();
        assert!(!none.contains("[[pack]]"), "{none}");
        // The remaps of an entry go as one inline table.
        let mut remapped = entry("@acme/docs");
        remapped.paths.insert("/knowledge/".into(), "/docs/".into());
        let added = add_entry(None, &remapped).unwrap();
        assert!(
            added.ends_with("paths = { \"/knowledge/\" = \"/docs/\" }\n"),
            "{added}"
        );
        let fresh = add_entry(None, &entry("@acme/base")).unwrap();
        assert!(
            fresh.starts_with("[[pack]]\nname = \"@acme/base\"\n"),
            "{fresh}"
        );
        // An entry follows the comments of a file that holds no table.
        for comments in ["# The packs.\n", "# The packs.", "# The packs.\n\n", "\n"] {
            let added = add_entry(Some(comments), &entry("@acme/base")).unwrap();
            assert!(
                added.starts_with(comments.trim_end())
                    && (added.contains("\n\n[[pack]]\nname = \"@acme/base\"\n")
                        || added.starts_with("[[pack]]\n")),
                "{added:?}"
            );
        }
        // The comments of a file without a table stay after the removal of
        // its last entry, and the blank lines of its text stay as they are.
        let spaced = "# The packs.   \n\n\n";
        let added = add_entry(Some(spaced), &entry("@acme/base")).unwrap();
        assert!(added.starts_with(spaced), "{added:?}");
        let gone = remove_entry(&added, "@acme/base").unwrap().unwrap();
        assert_eq!(gone, spaced);
        // The header of the file stays when its first entry goes.
        let two = "# The packs.\n\n[[pack]]\nname = \"@a/a\"\nsource = \"x\"\n\n[[pack]]\nname = \"@a/b\"\nsource = \"y\"\n";
        let one = remove_entry(two, "@a/a").unwrap().unwrap();
        assert!(one.starts_with("# The packs.\n"), "{one:?}");
        assert!(one.contains("name = \"@a/b\""), "{one:?}");
        // A file with CRLF line endings keeps them in every line.
        for text in [
            "# The packs.\r\n",
            "[[pack]]\r\nname = \"@acme/rust\"\r\nsource = \"x\"\r\n",
        ] {
            let crlf = add_entry(Some(text), &entry("@acme/base")).unwrap();
            assert!(
                crlf.starts_with(text) && !crlf.replace("\r\n", "").contains('\n'),
                "{crlf:?}"
            );
            let back = remove_entry(&crlf, "@acme/base").unwrap().unwrap();
            assert!(!back.replace("\r\n", "").contains('\n'), "{back:?}");
        }
    }
}
