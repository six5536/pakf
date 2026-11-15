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
/// `[[pack]]` entry. Every other line, comment, and blank line stays, an
/// entry that no table follows goes after the comments at the end of the
/// file, and the text keeps the line ending of the file.
// sokf-implements: CLI-3_AC-3
// sokf-implements: CLI-3_AC-5
pub fn add_entry(text: Option<&str>, entry: &PackEntry) -> Result<String> {
    let eol = eol_of(text.unwrap_or_default());
    let lf = text.map(|t| t.replace("\r\n", "\n"));
    added(lf.as_deref(), entry).map(|out| with_eol(&out, eol))
}

/// `added` is `add_entry` on a text with LF line endings.
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
    // A file of blank lines gives way to the entry.
    if text.unwrap_or_default().trim().is_empty() {
        doc = DocumentMut::new();
    }
    // The comments at the end of the file are the trailing text of the
    // document, which follows every table, so a new entry that no table
    // follows takes them before it, and goes after the file's whole text.
    let trailing = trailing(&doc);
    let last = match doc.get("pack") {
        None => true,
        Some(Item::ArrayOfTables(packs)) => {
            packs.iter().map(last_of).max().flatten() >= last_position(doc.as_table())
        }
        Some(_) => return Err(not_a_list()),
    };
    if last && !trailing.trim().is_empty() {
        // A blank line comes between the file's text and the entry, after
        // the final line ending that the text lacks, and the removal of the
        // entry takes it back.
        let gap = if trailing.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        table.decor_mut().set_prefix(format!("{trailing}{gap}"));
        table.set_position(Some(last_position(doc.as_table()).map_or(0, |p| p + 1)));
        doc.set_trailing("");
    }
    if let Some(Item::ArrayOfTables(packs)) = doc.get_mut("pack") {
        packs.push(table);
    } else {
        let mut packs = ArrayOfTables::new();
        packs.push(table);
        doc.insert("pack", Item::ArrayOfTables(packs));
    }
    Ok(doc.to_string())
}

/// `last_position` returns the greatest position in the document of the
/// tables below `table`, or none when no table below it has a position.
fn last_position(table: &Table) -> Option<isize> {
    table
        .iter()
        .filter_map(|(_, item)| match item {
            Item::Table(t) => last_of(t),
            Item::ArrayOfTables(a) => a.iter().map(last_of).max().flatten(),
            _ => None,
        })
        .max()
}

/// `last_of` returns the greatest position of the table `table` and of the
/// tables below it.
fn last_of(table: &Table) -> Option<isize> {
    table.position().max(last_position(table))
}

/// `trailing` returns the text of the document `doc` after its last table.
fn trailing(doc: &DocumentMut) -> String {
    doc.trailing().as_str().unwrap_or_default().to_string()
}

/// `up_to_last_blank_line` returns the part of the decor `prefix` before a
/// table up to and with its last blank line, which leaves out the comment
/// lines right above the table.
fn up_to_last_blank_line(prefix: &str) -> &str {
    let mut end = 0;
    let mut at = 0;
    for line in prefix.split_inclusive('\n') {
        at += line.len();
        if line.ends_with('\n') && line.trim().is_empty() {
            end = at;
        }
    }
    &prefix[..end]
}

/// `remove_entry` returns the text of the configuration `text` without the
/// `[[pack]]` entry of the pack `name`, or `None` when no entry declares
/// it. Every other line, comment, and blank line stays, the text that
/// `add_entry` appended to comes back, and the text keeps the line ending of
/// the file.
// sokf-implements: CLI-3_AC-3
// sokf-implements: CONF-4_AC-2
pub fn remove_entry(text: &str, name: &str) -> Result<Option<String>> {
    let eol = eol_of(text);
    removed(&text.replace("\r\n", "\n"), name).map(|out| out.map(|o| with_eol(&o, eol)))
}

/// `removed` is `remove_entry` on a text with LF line endings.
fn removed(text: &str, name: &str) -> Result<Option<String>> {
    let mut doc = parse(Some(text))?;
    let trailing = trailing(&doc);
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
    let (prefix, position) = packs
        .get(i)
        .map(|t| {
            let prefix = t.decor().prefix().and_then(|p| p.as_str());
            (prefix.unwrap_or_default().to_string(), t.position())
        })
        .unwrap_or_default();
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
    }
    // The lines before the last table of the file up to its last blank line,
    // or all of them in a file that holds no other key or table, stay before
    // the file's trailing text, such as the comments that `pakf add` put
    // before its entry, without the blank line that `pakf add` put after
    // them. The comment lines right above the table describe it, and go
    // with it.
    if position.is_some() && last_position(doc.as_table()) < position {
        let kept = if doc.as_table().is_empty() {
            &prefix
        } else {
            up_to_last_blank_line(&prefix)
        };
        if !kept.trim().is_empty() {
            let kept = if kept.ends_with("\n\n") {
                &kept[..kept.len() - 1]
            } else {
                kept
            };
            doc.set_trailing(format!("{kept}{trailing}"));
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

/// `with_eol` returns the text `text`, which has LF line endings, with each
/// line ending as `eol`.
fn with_eol(text: &str, eol: &str) -> String {
    text.replace('\n', eol)
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

    // sokf-tests: CLI-3_AC-5
    // sokf-tests: CLI-3_AC-3
    #[test]
    fn a_new_entry_follows_the_comment_at_the_end_of_the_file() {
        let new = "[[pack]]\nname = \"@acme/base\"\nsource = \"git@example.com:acme/base.git\"\npath = \"packs/base\"\n";
        let first = "[[pack]]\nname = \"@a/a\"\nsource = \"x\"\n";
        for text in [
            "version = 1\n# Add packs below.\n",
            "version = 1\n# Add packs below.",
            "version = 1\r\n# Add packs below.\r\n",
            &format!("{first}# Add packs below.\n"),
        ] {
            let added = add_entry(Some(text), &entry("@acme/base")).unwrap();
            let eol = if text.contains('\r') { "\r\n" } else { "\n" };
            assert!(added.starts_with(text), "{added:?}");
            assert!(added.ends_with(&new.replace('\n', eol)), "{added:?}");
            // The removal of the entry keeps the comment.
            let removed = remove_entry(&added, "@acme/base").unwrap().unwrap();
            assert_eq!(removed.trim_end(), text.trim_end());
        }
        // The comment lines right above the last entry describe it, and go
        // with it, and the lines before them stay.
        let described = format!("{first}\n# The base, pinned.\n{new}# The end.\n");
        let removed = remove_entry(&described, "@acme/base").unwrap().unwrap();
        assert_eq!(removed, format!("{first}# The end.\n"));
        let described = format!("{first}# Kept.\n\n# The base, pinned.\n{new}");
        let removed = remove_entry(&described, "@acme/base").unwrap().unwrap();
        assert_eq!(removed, format!("{first}# Kept.\n"));
        // An entry that a table follows goes after the last entry, and the
        // comment at the end of the file stays after that table.
        let text = format!("{first}\n[harness.claude]\nwith = []\n# The end.\n");
        let added = add_entry(Some(&text), &entry("@acme/base")).unwrap();
        assert!(
            added.starts_with(&format!("{first}\n{new}\n[harness.claude]")),
            "{added:?}"
        );
        assert!(added.ends_with("with = []\n# The end.\n"), "{added:?}");
    }

    // sokf-tests: CONF-4_AC-2
    #[test]
    fn a_removal_gives_back_the_file_before_the_addition() {
        let first = "[[pack]]\nname = \"@a/a\"\nsource = \"x\"\n";
        for text in [
            "",
            "# c\n",
            "# c\n\n",
            "# The packs.   \n\n\n",
            "# c\r\n",
            "# c\r\n\r\n",
            "version = 1\n",
            "version = 1\n\n",
            "version = 1\n# Add packs below.\n",
            "version = 1\n\n# Add packs below.\n\n",
            "version = 1\r\n# Add packs below.\r\n",
            first,
            &format!("{first}# Add packs below.\n"),
            &format!("{first}\n# Add packs below.\n\n"),
            &format!("{first}\n[harness.claude]\nwith = []\n# The end.\n"),
        ] {
            let added = add_entry(Some(text), &entry("@acme/base")).unwrap();
            let removed = remove_entry(&added, "@acme/base").unwrap().unwrap();
            assert_eq!(removed, text, "{added:?}");
        }
        // A text without its final line ending comes back with it.
        for text in ["# c", "version = 1\n# c"] {
            let added = add_entry(Some(text), &entry("@acme/base")).unwrap();
            let removed = remove_entry(&added, "@acme/base").unwrap().unwrap();
            assert_eq!(removed, format!("{text}\n"), "{added:?}");
        }
        // The removal of an entry that a person wrote keeps each line ending
        // of the comments before it.
        let new = "[[pack]]\nname = \"@a/n\"\nsource = \"y\"\n";
        for (text, kept) in [
            (format!("# Packs\n{new}"), "# Packs\n"),
            (format!("# Packs\n{new}# end\n"), "# Packs\n# end\n"),
            (format!("# Packs\n\n{new}"), "# Packs\n"),
            (format!("version = 1\n\n\n{new}"), "version = 1\n"),
            (
                format!("version = 1\n# c\n\n# n\n{new}# end\n"),
                "version = 1\n# c\n# end\n",
            ),
        ] {
            assert_eq!(remove_entry(&text, "@a/n").unwrap().unwrap(), kept);
        }
    }
}
