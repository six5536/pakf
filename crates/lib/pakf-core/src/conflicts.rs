//! The conflict check: a line that begins with git's conflict marker
//! `<<<<<<< ` begins a conflict that a merge of a sync left for the user to
//! resolve. A line inside a fenced code block of a Markdown file is an
//! example of a conflict, such as one in a recipe, so the check passes it by.
//! The reader of fences follows the blocks of `CommonMark` 0.31.2 at the top
//! level and in list items, so that a line of backticks inside another block,
//! such as an HTML comment, opens no fence.

mod html;

use std::ops::RangeInclusive;

use html::{HtmlEnd, html_start};

/// `MARKER` is the start of the first line of a conflict that git writes.
pub const MARKER: &str = "<<<<<<< ";

/// `conflict_lines` returns the number of each line of the file `path`,
/// with the text `text`, that begins a conflict, from 1. In a Markdown file,
/// a line inside a fenced code block doesn't count, except in a file whose
/// frontmatter gives `type: Schema`, whose fence is the file's content
/// itself, so that a merge of such a file conflicts inside it.
#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "only a name that ends in a lowercase `.md` is Markdown, so the comparison is case-sensitive on purpose"
)]
// sokf-implements: CLI-1_AC-20
pub fn conflict_lines(path: &crate::RepoPath, text: &str) -> Vec<usize> {
    let marked: Vec<usize> = text
        .lines()
        .enumerate()
        .filter(|(_, l)| l.starts_with(MARKER))
        .map(|(i, _)| i)
        .collect();
    let skip = if !marked.is_empty() && path.as_str().ends_with(".md") && !is_schema(text) {
        fences(text)
    } else {
        Vec::new()
    };
    marked
        .into_iter()
        .filter(|i| !skip.iter().any(|f| f.contains(i)))
        .map(|i| i + 1)
        .collect()
}

/// `is_schema` returns whether the frontmatter of the Markdown text `text`,
/// the YAML between the `---` lines that begin it, gives `type: Schema`.
fn is_schema(text: &str) -> bool {
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return false;
    };
    let mut at = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return crate::yaml::parse(&rest[..at], 2).is_ok_and(|fm| {
                fm.as_map().is_some_and(|m| {
                    m.iter()
                        .any(|(k, v)| k.value == "type" && v.as_str() == Some("Schema"))
                })
            });
        }
        at += line.len();
    }
    false
}

/// `indent` returns the number of spaces at the start of `line`.
fn indent(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

/// `is_blank` returns whether `line` holds only spaces and tabs.
fn is_blank(line: &str) -> bool {
    line.bytes().all(|b| b == b' ' || b == b'\t')
}

/// `fence` returns the character and the length of the run of three or
/// more backticks or tildes that `line` begins with after its spaces.
/// The info string of a backtick fence holds no backtick, as `CommonMark`
/// reads it, so a line such as ``` a`b``` begins a paragraph.
fn fence(line: &str) -> Option<(char, usize)> {
    let rest = line.trim_start_matches(' ');
    let c = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = rest.chars().take_while(|x| *x == c).count();
    (n >= 3 && (c == '~' || !rest[n..].contains('`'))).then_some((c, n))
}

/// `thematic_break` returns whether the line `s`, from its first character
/// that isn't a space, is a thematic break: three or more of one of `-`,
/// `*`, and `_`, with only spaces and tabs between them.
fn thematic_break(s: &str) -> bool {
    s.chars().next().is_some_and(|first| {
        matches!(first, '-' | '*' | '_')
            && s.chars().all(|c| c == first || c == ' ' || c == '\t')
            && s.chars().filter(|c| *c == first).count() >= 3
    })
}

/// `atx_heading` returns whether the line `s`, from its first character
/// that isn't a space, begins a heading of one to six `#`.
fn atx_heading(s: &str) -> bool {
    let n = s.bytes().take_while(|b| *b == b'#').count();
    (1..=6).contains(&n) && matches!(s.as_bytes().get(n), None | Some(b' ' | b'\t'))
}

/// `setext_underline` returns whether the line `s`, from its first character
/// that isn't a space, underlines a paragraph as a heading: a run of `=` or
/// of `-`, and then only spaces and tabs.
fn setext_underline(s: &str) -> bool {
    let run = s.trim_end_matches([' ', '\t']);
    run.chars()
        .next()
        .is_some_and(|first| matches!(first, '=' | '-') && run.chars().all(|c| c == first))
}

/// `list_item` returns the indent of the content of the list item that the
/// line `s` begins, from its first character that isn't a space: a bullet
/// `-`, `+`, or `*`, or one to nine digits and `.` or `)`, and then a space
/// or the end of the line. An item that interrupts a `paragraph` holds text
/// on its first line, and an ordered one begins at 1.
fn list_item(s: &str, paragraph: bool) -> Option<usize> {
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    let marker = match s.as_bytes().first()? {
        b'-' | b'+' | b'*' => 1,
        _ if (1..=9).contains(&digits) && s[digits..].starts_with(['.', ')']) => {
            if paragraph && s[..digits].trim_start_matches('0') != "1" {
                return None;
            }
            digits + 1
        }
        _ => return None,
    };
    let after = &s[marker..];
    let spaces = indent(after);
    if spaces == 0 && !after.is_empty() {
        return None;
    }
    let empty = is_blank(after);
    if paragraph && empty {
        return None;
    }
    // An item whose first line holds no text, or holds an indented code
    // block, has its content one space after the marker.
    Some(if empty || spaces > 4 {
        marker + 1
    } else {
        marker + spaces
    })
}

/// A `Leaf` is the block that holds the last line that the reader of fences
/// read, inside the innermost list item that the reader is in or at the top
/// level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leaf {
    /// No block that the next line continues, such as after a blank line, a
    /// heading, or a line of an indented code block.
    None,
    /// A paragraph, which the next line continues unless it begins a block.
    Paragraph,
    /// A fenced code block, by its first line and the character and the
    /// length of its opening fence.
    Fence { start: usize, c: char, n: usize },
    /// An HTML block, by the condition that ends it.
    Html(HtmlEnd),
}

/// `MAX_ITEMS` is the number of list items that one line begins at most, as
/// in cmark, beyond which the rest of the line is text, so that a line of
/// many list markers costs no more than a line of that many.
const MAX_ITEMS: usize = 100;

/// A `Start` is what a line begins, from the content of its container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Start {
    /// Nothing, as the line is blank.
    Blank,
    /// An indented code block, or the next line of a paragraph.
    Indented,
    /// A fenced code block, by the character and the length of its fence.
    Fence(char, usize),
    /// An HTML block, by the condition that ends it.
    Html(HtmlEnd),
    /// A block of the one line, a heading, a thematic break, or the
    /// underline of a setext heading.
    Line,
    /// A list item, by the indent of its content from the line's first
    /// character that isn't a space.
    Item(usize),
    /// A block quote, which ends the paragraph of a list item on a lazy
    /// line, and whose content the reader reads as a paragraph.
    Quote,
    /// The text of a paragraph.
    Text,
}

/// `start` returns what the line `s`, after `spaces` spaces from the
/// content of its container, begins, after a line of a `paragraph`, or as a
/// `lazy` line, which follows the paragraph of a list item that it doesn't
/// continue, so that `lazy` comes with `paragraph`: a lazy line begins no
/// setext underline, and it begins a list item that the paragraph doesn't
/// restrict.
fn start(s: &str, spaces: usize, paragraph: bool, lazy: bool) -> Start {
    if is_blank(s) {
        Start::Blank
    } else if spaces >= 4 {
        Start::Indented
    } else if let Some((c, n)) = fence(s) {
        Start::Fence(c, n)
    } else if let Some(end) = html_start(s, paragraph) {
        Start::Html(end)
    } else if atx_heading(s) || thematic_break(s) || (paragraph && !lazy && setext_underline(s)) {
        Start::Line
    } else if let Some(width) = list_item(s, paragraph && !lazy) {
        Start::Item(width)
    } else if s.starts_with('>') {
        Start::Quote
    } else {
        Start::Text
    }
}

/// A `Reader` reads the blocks of a Markdown text line by line, as
/// `CommonMark` reads them, to find its fenced code blocks.
struct Reader {
    /// The lines, from 0, of each fenced code block that the reader has read
    /// to its end, from its opening fence to its closing fence, or to the end
    /// of the text or of its list item where it has none.
    out: Vec<RangeInclusive<usize>>,
    /// The indent of the content of each list item that the reader is in,
    /// from the outermost, so each indent is greater than the one before.
    items: Vec<usize>,
    /// Whether a line of the innermost item holds text, without which a
    /// blank line ends the item; every other item holds the line that began
    /// the next.
    filled: bool,
    /// The block of the last line, in the innermost item or at the top
    /// level.
    leaf: Leaf,
}

impl Reader {
    /// `container` returns the indent of the content of the innermost list
    /// item that the reader is in, and 0 at the top level.
    fn container(&self) -> usize {
        self.items.last().copied().unwrap_or(0)
    }

    /// `line` reads the line `line`, whose number is `i`, from 0.
    // sokf-implements: CLI-1_AC-33
    fn line(&mut self, i: usize, line: &str) {
        let lead = indent(line);
        let blank = is_blank(line);
        // A blank line continues each item that holds text, and any other
        // line each item whose content it is indented as far as.
        let kept = if blank {
            self.items.len() - usize::from(!self.filled)
        } else {
            self.items.partition_point(|content| *content <= lead)
        };
        if kept < self.items.len() {
            let base = self.items[..kept].last().copied().unwrap_or(0);
            if self.leaf == Leaf::Paragraph && !blank && lazy(line, base) {
                // A lazy continuation line, which keeps the paragraph and
                // every item.
                return;
            }
            // Any other line ends each item that it doesn't continue, and
            // the block in it.
            self.end(i);
            self.items.truncate(kept);
            self.filled = true;
        }
        let container = self.container();
        match self.leaf {
            Leaf::Fence { start, c, n } => {
                let closing = !blank
                    && lead - container <= 3
                    && fence(line).is_some_and(|(k, m)| {
                        k == c && m >= n && is_blank(&line.trim_start_matches(' ')[m..])
                    });
                if closing {
                    self.out.push(start..=i);
                    self.leaf = Leaf::None;
                }
            }
            Leaf::Html(end) => {
                if end.ends(line) {
                    self.leaf = Leaf::None;
                }
            }
            Leaf::None | Leaf::Paragraph => self.open(i, line),
        }
    }

    /// `open` reads the line `line`, whose number is `i`, from the content
    /// of its container, where no block continues: it begins a block there
    /// or continues a paragraph.
    fn open(&mut self, i: usize, line: &str) {
        // Each list item that the line begins takes the rest of the line.
        let mut items = 0;
        loop {
            let base = self.container();
            let rest = line.get(base..).unwrap_or("");
            let spaces = indent(rest);
            let s = &rest[spaces..];
            let paragraph = self.leaf == Leaf::Paragraph;
            let start = start(s, spaces, paragraph, false);
            if start != Start::Blank {
                self.filled = true;
            }
            self.leaf = match start {
                Start::Indented if paragraph => Leaf::Paragraph,
                // A block whose first line ends it holds that line alone.
                Start::Html(end) if !end.ends(s) => Leaf::Html(end),
                Start::Blank | Start::Line | Start::Indented | Start::Html(_) => Leaf::None,
                Start::Fence(c, n) => Leaf::Fence { start: i, c, n },
                Start::Item(width) if items < MAX_ITEMS => {
                    self.items.push(base + spaces + width);
                    self.filled = false;
                    self.leaf = Leaf::None;
                    items += 1;
                    continue;
                }
                Start::Item(_) | Start::Quote | Start::Text => Leaf::Paragraph,
            };
            return;
        }
    }

    /// `end` ends the block of the last line at the line before the line
    /// `i`.
    fn end(&mut self, i: usize) {
        if let Leaf::Fence { start, .. } = self.leaf {
            self.out.push(start..=i - 1);
        }
        self.leaf = Leaf::None;
    }
}

/// `lazy` returns whether the line `line`, from the column `base` of the
/// content of the innermost list item that it continues, is a lazy
/// continuation line of the paragraph of an item that it doesn't continue:
/// a line that begins no block, or that is indented four spaces or more.
// sokf-implements: CLI-1_AC-32
fn lazy(line: &str, base: usize) -> bool {
    let rest = line.get(base..).unwrap_or("");
    let spaces = indent(rest);
    matches!(
        start(&rest[spaces..], spaces, true, true),
        Start::Indented | Start::Text
    )
}

/// `fences` returns the lines, from 0, of each fenced code block of the
/// Markdown `text`, as `CommonMark` reads one at the top level or in a list
/// item: an opening fence of three or more backticks or tildes, at most
/// three spaces in from its container, outside every HTML block and
/// indented code block, and a closing fence of the same character, at least
/// as long, at most three spaces in, with only spaces and tabs after it. A
/// line indented less than the content of the list item that holds the
/// block ends the item, and the block with it.
fn fences(text: &str) -> Vec<RangeInclusive<usize>> {
    let mut reader = Reader {
        out: Vec::new(),
        items: Vec::new(),
        filled: true,
        leaf: Leaf::None,
    };
    let mut count = 0;
    for (i, line) in text.lines().enumerate() {
        reader.line(i, line);
        count = i + 1;
    }
    reader.end(count);
    reader.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RepoPath;

    fn lines(path: &str, text: &str) -> Vec<usize> {
        conflict_lines(&RepoPath::parse(path).unwrap(), text)
    }

    /// `md` returns the lines that begin a conflict in a Markdown file.
    fn md(text: &str) -> Vec<usize> {
        lines("/knowledge/r.md", text)
    }

    /// `none` checks that no line begins a conflict in a Markdown file.
    fn none(text: &str) {
        let found = md(text);
        assert!(found.is_empty(), "{text:?}: {found:?}");
    }

    #[test]
    fn each_line_that_begins_a_conflict_counts() {
        let text = "a\n<<<<<<< ours\nb\n=======\nc\n>>>>>>> theirs\n <<<<<<< x\n<<<<<<<\n";
        assert_eq!(lines("/knowledge/a.md", text), vec![2]);
        assert_eq!(lines("/.pakf/packs/m/formats.yaml", text), vec![2]);
        assert_eq!(lines("/knowledge/a.md", "no conflict\n"), [] as [usize; 0]);
    }

    // sokf-tests: CLI-1_AC-20
    #[test]
    fn a_marker_in_a_fence_of_a_document_is_an_example() {
        // A line at the start of a line ends the fence of a list item.
        let item = "1. Run:\n   ```sh\n   <<<<<<< a\n<<<<<<< b\n   ```\n";
        assert_eq!(lines("/knowledge/r.md", item), vec![4]);
        // A fence four spaces in is content, and a closing fence at a
        // smaller indent than the opening one closes a top-level block.
        let nested = "```markdown\n    ```sh\n<<<<<<< a\n    ```\n```\n<<<<<<< b\n";
        assert_eq!(lines("/knowledge/r.md", nested), vec![6]);
        let shifted = "   ```text\ncode\n<<<<<<< a\n```\n<<<<<<< b\n";
        assert_eq!(lines("/knowledge/r.md", shifted), vec![5]);
        // A backtick in the info string makes no fence, a thematic break is
        // no list item, and an item can begin with a fence.
        assert_eq!(lines("/knowledge/r.md", "``` a`b\n<<<<<<< x\n"), vec![2]);
        let rule = "* * *\n  ```text\n<<<<<<< a\n  ```\n";
        assert_eq!(lines("/knowledge/r.md", rule), [] as [usize; 0]);
        let item = "- ```text\n  <<<<<<< a\n  ```\n<<<<<<< b\n";
        assert_eq!(lines("/knowledge/r.md", item), vec![4]);
        let example = "```text\n<<<<<<< /knowledge/a.md\nmine\n=======\ntheirs\n>>>>>>> /knowledge/a.md\n```\n";
        assert_eq!(lines("/knowledge/r.md", example), [] as [usize; 0]);
        let tilde = "~~~~\n```\n<<<<<<< a\n~~~~\n<<<<<<< b\n";
        assert_eq!(lines("/knowledge/r.md", tilde), vec![5]);
        // Only spaces and tabs follow a closing fence.
        let nbsp = "```\n<<<<<<< a\n```\u{a0}\n<<<<<<< b\n```\t\n<<<<<<< c\n";
        assert_eq!(lines("/knowledge/r.md", nbsp), vec![6]);
        // An empty item has its content one space after the marker, even
        // with more spaces after it.
        let empty = "-  \n  ```\n<<<<<<< a\n```\n";
        assert_eq!(lines("/knowledge/r.md", empty), vec![3]);
        // An item that interrupts a paragraph begins at 1, and a `-` after a
        // paragraph underlines it, so the fence after either of the others
        // is a top-level one.
        let first = "text\n1. a\n   ```\n<<<<<<< a\n";
        assert_eq!(lines("/knowledge/r.md", first), vec![4]);
        for text in [
            "text\n2. a\n   ```\n<<<<<<< a\n",
            "text\n-\n  ```\n<<<<<<< a\n",
        ] {
            let found = lines("/knowledge/r.md", text);
            assert!(found.is_empty(), "{text:?}: {found:?}");
        }
        // The same lines count in a file that isn't Markdown.
        assert_eq!(lines("/r.txt", example), vec![2]);
        // Every line of a schema counts, because its fence is the schema.
        let schema = format!("---\ntype: Schema\nid: s\n---\n\n{example}");
        assert_eq!(lines("/knowledge/schemas/s.md", &schema), vec![7]);
        // An empty frontmatter ends at its second line, and a frontmatter of
        // another type is no schema's.
        assert!(!is_schema("---\n---\n# T\n\ntype: Schema\n---\n"));
        assert!(!is_schema("---\ntype: Note\n---\n"));
        assert!(is_schema("---\r\ntype: Schema\r\n---\r\n"));
    }

    // sokf-tests: CLI-1_AC-31
    #[test]
    fn a_fence_inside_an_html_block_opens_no_block() {
        // A comment, a processing instruction, a declaration, and CDATA end
        // at the line that holds their end, the first line among them.
        assert_eq!(md("<!--\n```\n-->\n<<<<<<< a\n"), vec![4]);
        none("<!-- c -->\n```\n<<<<<<< a\n```\n");
        none("<!-->\n```\n<<<<<<< a\n```\n");
        assert_eq!(md("<?php\n```\n?>\n<<<<<<< a\n"), vec![4]);
        assert_eq!(md("<!DOCTYPE html\n~~~\n>\n<<<<<<< a\n"), vec![4]);
        assert_eq!(md("<![CDATA[\n```\n]]>\n<<<<<<< a\n"), vec![4]);
        // `pre` and its kin run past a blank line to any of their end tags.
        assert_eq!(md("<pre>\n\n```\n</TEXTAREA>\n<<<<<<< a\n"), vec![5]);
        assert_eq!(md("<pre\n```\n</pre>\n<<<<<<< a\n"), vec![4]);
        // A block tag, or a whole tag alone on its line, runs to a blank line.
        assert_eq!(md("<DIV class=\"x\">\n```\n\n<<<<<<< a\n"), vec![4]);
        assert_eq!(md("<details/>\n```\n<<<<<<< a\n"), vec![3]);
        assert_eq!(md("<div-x>\n```\n\n<<<<<<< a\n```\n"), vec![4]);
        assert_eq!(
            md("<a href=\"x\" title='y' hidden>\n```\n<<<<<<< a\n"),
            vec![3]
        );
        assert_eq!(md("<a b = \"c\">\n```\n<<<<<<< a\n"), vec![3]);
        assert_eq!(md("</SPAN >\n```\n<<<<<<< a\n"), vec![3]);
        // A whole tag doesn't interrupt a paragraph, and a tag with text or
        // a broken attribute after it begins no block, so the fence after
        // each opens a block.
        none("text\n<span>\n```\n<<<<<<< a\n```\n");
        none("<span>x\n```\n<<<<<<< a\n```\n");
        none("<a b=>\n```\n<<<<<<< a\n```\n");
        none("<a b=\"c\"d>\n```\n<<<<<<< a\n```\n");
        // A heading and a setext underline end a paragraph, and so does a
        // list item with text that begins at 1, so a whole tag after them
        // begins a block; an empty item and one that begins at 2 end none.
        assert_eq!(md("# T\n<span>\n```\n<<<<<<< a\n"), vec![4]);
        assert_eq!(md("text\n===\n<span>\n```\n<<<<<<< a\n"), vec![5]);
        assert_eq!(md("text\n-\n<span>\n```\n<<<<<<< a\n"), vec![5]);
        assert_eq!(md("text\n01. a\n\n<span>\n```\n<<<<<<< a\n"), vec![6]);
        none("text\n1.\n<span>\n```\n<<<<<<< a\n```\n");
        none("text\n2. a\n<span>\n```\n<<<<<<< a\n```\n");
        // A block four spaces in is code, and a block in a list item ends with
        // the item.
        none("    <!--\n```\n<<<<<<< a\n```\n");
        assert_eq!(md("  <!--\n  ```\n  -->\n<<<<<<< a\n"), vec![4]);
        assert_eq!(md("- <!--\n<<<<<<< a\n```\n<<<<<<< b\n```\n"), vec![2]);
        // A line of a fence begins no HTML block.
        assert_eq!(md("```\n<!--\n```\n<<<<<<< a\n"), vec![4]);
        // A whole tag after the paragraph of a list item continues the
        // paragraph, and a tag of `pre` or its kin begins no block of a whole
        // tag.
        none("- a\n<span>\n```\n<<<<<<< a\n```\n");
        none("</script>\n```\n<<<<<<< a\n```\n");
        none("<pre/>\n```\n<<<<<<< a\n```\n");
        // A line of many list markers is read without a call for each.
        let deep = format!("{}```\n<<<<<<< a\n", "+ ".repeat(100_000));
        assert_eq!(md(&deep), vec![2]);
        let deep = format!("{}x\n```\n<<<<<<< a\n", "- ".repeat(100_000));
        none(&deep);
        // A lazy line four spaces in continues the paragraph too.
        none("-    a\n    b\n<span>\n```\n<<<<<<< a\n```\n");
    }

    // sokf-tests: CLI-1_AC-32
    #[test]
    fn a_lazy_line_keeps_the_list_item() {
        // A lazy line, of text, of a whole tag, of `===`, or four spaces in,
        // keeps the item, so a line at the start of a line ends its fence.
        assert_eq!(md("- a\nb\n  ```\n<<<<<<< x\n"), vec![4]);
        assert_eq!(md("- a\n<span>\n  ```\n<<<<<<< x\n"), vec![4]);
        assert_eq!(md("- a\n===\n  ```\n<<<<<<< x\n"), vec![4]);
        assert_eq!(md("1.   a\n    b\n     ```\n<<<<<<< x\n"), vec![4]);
        // A line that begins a block is no lazy line: it ends the item, so
        // the fence after it is a top-level one, or lies in an HTML block.
        for block in ["---", "# h", "> q", "2. b"] {
            none(&format!("- a\n{block}\n  ```\n<<<<<<< x\n"));
        }
        assert_eq!(md("- a\n<div>\n```\n<<<<<<< x\n```\n"), vec![4]);
        // A lazy line of an inner item keeps both items, and a line that
        // begins a block from the content of the outer item, as a heading,
        // ends the inner item alone.
        assert_eq!(md("- a\n  - b\nc\n    ```\n<<<<<<< x\n"), vec![5]);
        assert_eq!(
            md("- a\n  -   b\n    # h\n<span>\n```\n<<<<<<< x\n```\n"),
            vec![6]
        );
        // A line after a blank line is no lazy line either.
        none("- a\n\nb\n  ```\n<<<<<<< x\n");
    }

    // sokf-tests: CLI-1_AC-33
    #[test]
    fn a_fence_belongs_to_the_item_that_holds_it() {
        // A line that ends an inner item keeps the outer one.
        assert_eq!(md("- a\n  - b\n\n  ```\n<<<<<<< x\n"), vec![5]);
        assert_eq!(md("- - a\n  ```\n<<<<<<< x\n"), vec![3]);
        let nested = "1. Run the step\n   - note\n   ```sh\n<<<<<<< ours\n   a\n=======\n";
        assert_eq!(md(nested), vec![4]);
        // An item three levels in ends at a line indented as its parent.
        let deep = "- a\n  - b\n    - c\n      ```\n    <<<<<<< y\n  ```\n<<<<<<< x\n```\n";
        assert_eq!(md(deep), vec![7]);
        // An empty item ends at a blank line, even one indented as its
        // content, so the fence after it is a top-level one.
        none("-\n\n  ```\n<<<<<<< x\n```\n");
        none("-\n  \n  ```\n<<<<<<< x\n```\n");
    }
}
