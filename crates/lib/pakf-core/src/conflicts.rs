//! The conflict check: a line that begins with git's conflict marker
//! `<<<<<<< ` begins a conflict that a merge of a sync left for the user to
//! resolve. A line inside a fenced code block of a Markdown file is an
//! example of a conflict, such as one in a recipe, so the check passes it by.

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
        .filter(|i| !skip.iter().any(|f| f.lines.contains(i)))
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

/// A `Fenced` is one fenced code block of a Markdown text, by its lines,
/// from 0, from its opening fence to its closing fence, or to the end of the
/// text or of its list item when it has none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fenced {
    /// The lines of the block, the fences among them.
    pub lines: std::ops::RangeInclusive<usize>,
    /// Whether the block ends with a closing fence.
    pub closed: bool,
}

/// `indent` returns the number of spaces at the start of `line`.
fn indent(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
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

/// `item_content` returns the indent of the content of a list item that
/// `line` begins, after at most three spaces: a bullet `-`, `+`, or `*`, or
/// one to nine digits and `.` or `)`, and then a space.
fn item_content(line: &str) -> Option<usize> {
    let lead = indent(line);
    if lead > 3 {
        return None;
    }
    let rest = &line[lead..];
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    let first = rest.chars().next()?;
    // Three or more of one bullet with only spaces between them is a
    // thematic break, such as `* * *`, and no list item.
    if matches!(first, '-' | '*' | '_')
        && rest.trim().chars().all(|c| c == first || c == ' ')
        && rest.chars().filter(|c| *c == first).count() >= 3
    {
        return None;
    }
    let marker = match first {
        '-' | '+' | '*' => 1,
        _ if (1..=9).contains(&digits) && rest[digits..].starts_with(['.', ')']) => digits + 1,
        _ => return None,
    };
    let after = &rest[marker..];
    let spaces = after.len() - after.trim_start_matches(' ').len();
    match spaces {
        0 if !after.is_empty() => None,
        1..=4 => Some(lead + marker + spaces),
        _ => Some(lead + marker + 1),
    }
}

/// `fences` returns each fenced code block of the Markdown `text`, as
/// `CommonMark` reads one at the top level or in a list item: an opening
/// fence of three or more backticks or tildes, at most three spaces in from
/// its container, and a closing fence of the same character, at least as
/// long, at most three spaces in, with nothing after it. A line indented
/// less than the content of the list item that holds the block ends the
/// item, and the block with it.
pub(crate) fn fences(text: &str) -> Vec<Fenced> {
    let mut out = Vec::new();
    // The indent of the content of the list item that the text is in.
    let mut container = 0;
    // The first line, the character, the length, and the container of the
    // open block.
    let mut open: Option<(usize, char, usize, usize)> = None;
    let mut last = 0;
    for (i, line) in text.lines().enumerate() {
        last = i;
        let lead = indent(line);
        let blank = line.trim().is_empty();
        if let Some((start, c, n, base)) = open {
            if !blank && base > 0 && lead < base {
                out.push(Fenced {
                    lines: start..=i - 1,
                    closed: false,
                });
                open = None;
                container = 0;
            } else {
                let closing = lead >= base
                    && lead - base <= 3
                    && fence(line).is_some_and(|(k, m)| {
                        k == c && m >= n && line.trim_start_matches(' ')[m..].trim().is_empty()
                    });
                if closing {
                    out.push(Fenced {
                        lines: start..=i,
                        closed: true,
                    });
                    open = None;
                }
                continue;
            }
        }
        if !blank {
            if let Some(content) = item_content(line) {
                container = content;
                // An item can begin with a fence, as in `- ```yaml`.
                if let Some((c, n)) = line.get(content..).and_then(fence) {
                    open = Some((i, c, n, container));
                }
                continue;
            }
            if lead < container {
                container = 0;
            }
        }
        if let Some((c, n)) = fence(line) {
            let base = if container > 0 && lead >= container && lead - container <= 3 {
                Some(container)
            } else if lead <= 3 {
                Some(0)
            } else {
                None
            };
            if let Some(base) = base {
                open = Some((i, c, n, base));
            }
        }
    }
    if let Some((start, ..)) = open {
        out.push(Fenced {
            lines: start..=last,
            closed: false,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RepoPath;

    fn lines(path: &str, text: &str) -> Vec<usize> {
        conflict_lines(&RepoPath::parse(path).unwrap(), text)
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
}
