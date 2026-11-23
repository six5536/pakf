//! The HTML blocks of the reader of fences: the seven start conditions of
//! `CommonMark` 0.31.2 and the end condition of each.

use super::is_blank;

/// An `HtmlEnd` is the condition that ends an HTML block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HtmlEnd {
    /// The block ends before the next blank line.
    Blank,
    /// The block ends with the first line that holds one of the texts, in
    /// any case of their ASCII letters.
    Text(&'static [&'static str]),
}

impl HtmlEnd {
    /// `ends` returns whether the line `line` of the block ends it.
    pub(super) fn ends(self, line: &str) -> bool {
        match self {
            Self::Blank => is_blank(line),
            Self::Text(texts) => texts.iter().any(|t| {
                line.as_bytes()
                    .windows(t.len())
                    .any(|w| w.eq_ignore_ascii_case(t.as_bytes()))
            }),
        }
    }
}

/// `BLOCK_TAGS` are the names of the tags that begin an HTML block that
/// runs to a blank line, as `CommonMark` 0.31.2 lists them.
const BLOCK_TAGS: [&str; 62] = [
    "address",
    "article",
    "aside",
    "base",
    "basefont",
    "blockquote",
    "body",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hr",
    "html",
    "iframe",
    "legend",
    "li",
    "link",
    "main",
    "menu",
    "menuitem",
    "nav",
    "noframes",
    "ol",
    "optgroup",
    "option",
    "p",
    "param",
    "search",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "track",
    "ul",
];

/// `RAW_TAGS` are the names of the tags that begin an HTML block that runs
/// to the end tag of any of them, which `RAW_ENDS` holds.
const RAW_TAGS: [&str; 4] = ["pre", "script", "style", "textarea"];

/// `RAW_ENDS` are the end tags of `RAW_TAGS`.
const RAW_ENDS: [&str; 4] = ["</pre>", "</script>", "</style>", "</textarea>"];

/// `tag_name` returns the end of the name of a tag that begins at `from` in
/// the line `b`: an ASCII letter, and then letters, digits, and `-`, or
/// `from` itself where no name begins.
fn tag_name(b: &[u8], from: usize) -> usize {
    if !b.get(from).is_some_and(u8::is_ascii_alphabetic) {
        return from;
    }
    from + b[from..]
        .iter()
        .take_while(|c| c.is_ascii_alphanumeric() || **c == b'-')
        .count()
}

/// `is_one_of` returns whether the name `name` is one of `names`, in any
/// case of its ASCII letters.
fn is_one_of(name: &[u8], names: &[&str]) -> bool {
    names
        .iter()
        .any(|t| name.eq_ignore_ascii_case(t.as_bytes()))
}

/// `html_start` returns the end of the HTML block that the line `s` begins,
/// from its first character that isn't a space, by the seven start
/// conditions of `CommonMark`. A line that holds only a whole tag begins no
/// block where it would interrupt a `paragraph`.
// sokf-implements: CLI-1_AC-31
pub(super) fn html_start(s: &str, paragraph: bool) -> Option<HtmlEnd> {
    let b = s.as_bytes();
    if b.first() != Some(&b'<') {
        return None;
    }
    // Whether a tag's name ends at `at`, before a space, a tab, `>`, the end
    // of the line, or, with `slash`, `/>`.
    let name_ends = |at: usize, slash: bool| match b.get(at) {
        None | Some(b' ' | b'\t' | b'>') => true,
        Some(b'/') => slash && b.get(at + 1) == Some(&b'>'),
        _ => false,
    };
    // The name of the tag, from `from` to `to`, after `</` in a closing tag.
    let closing = b.get(1) == Some(&b'/');
    let from = 1 + usize::from(closing);
    let to = tag_name(b, from);
    let name = &b[from..to];
    if !closing && is_one_of(name, &RAW_TAGS) && name_ends(to, false) {
        return Some(HtmlEnd::Text(&RAW_ENDS));
    }
    if s.starts_with("<!--") {
        return Some(HtmlEnd::Text(&["-->"]));
    }
    if s.starts_with("<?") {
        return Some(HtmlEnd::Text(&["?>"]));
    }
    if s.starts_with("<![CDATA[") {
        return Some(HtmlEnd::Text(&["]]>"]));
    }
    if b.get(1) == Some(&b'!') && b.get(2).is_some_and(u8::is_ascii_alphabetic) {
        return Some(HtmlEnd::Text(&[">"]));
    }
    if is_one_of(name, &BLOCK_TAGS) && name_ends(to, true) {
        return Some(HtmlEnd::Blank);
    }
    let whole = !paragraph && to > from && !is_one_of(name, &RAW_TAGS) && whole_tag(s, closing, to);
    whole.then_some(HtmlEnd::Blank)
}

/// `whole_tag` returns whether the line `s`, from its `<`, holds one whole
/// open tag, or a closing tag where `closing`, whose name ends at `to`, and
/// then only spaces and tabs.
fn whole_tag(s: &str, closing: bool, to: usize) -> bool {
    let b = s.as_bytes();
    let spaces = |at: usize| {
        b[at..]
            .iter()
            .take_while(|c| matches!(c, b' ' | b'\t'))
            .count()
    };
    let mut at = to;
    if closing {
        at += spaces(at);
    } else {
        // Each attribute: spaces, a name, and an optional value after `=`.
        loop {
            let gap = spaces(at);
            let name = at + gap;
            if gap == 0
                || !b
                    .get(name)
                    .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, b'_' | b':'))
            {
                break;
            }
            at = name
                + b[name..]
                    .iter()
                    .take_while(|c| {
                        c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b':' | b'-')
                    })
                    .count();
            let eq = at + spaces(at);
            if b.get(eq) != Some(&b'=') {
                continue;
            }
            let value = eq + 1 + spaces(eq + 1);
            let len = match b.get(value) {
                Some(q @ (b'"' | b'\'')) => match b[value + 1..].iter().position(|c| c == q) {
                    Some(n) => n + 2,
                    None => return false,
                },
                _ => b[value..]
                    .iter()
                    .take_while(|c| {
                        !matches!(c, b' ' | b'\t' | b'"' | b'\'' | b'=' | b'<' | b'>' | b'`')
                    })
                    .count(),
            };
            if len == 0 {
                return false;
            }
            at = value + len;
        }
        at += spaces(at);
        if b.get(at) == Some(&b'/') {
            at += 1;
        }
    }
    b.get(at) == Some(&b'>') && is_blank(&s[at + 1..])
}
