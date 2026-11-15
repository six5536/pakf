//! This module builds pakf's block of the packs' instructions, which the
//! refresh hands to agent-harness-adapter.

use std::fmt::Write as _;

/// `is_marker` returns whether agent-harness-adapter reads the line `line`,
/// its end trimmed, as a marker of the block of a tool, such as
/// `<!-- pakf:harness -->`, `<!-- pakf:harness last -->`, or
/// `<!-- /sokf:harness -->`, which would end pakf's block early or open
/// another block inside it.
fn is_marker(line: &str) -> bool {
    let Some(inner) = line
        .strip_prefix("<!-- ")
        .and_then(|l| l.strip_suffix(" -->"))
    else {
        return false;
    };
    match inner.strip_prefix('/') {
        Some(closing) => closing
            .strip_suffix(":harness")
            .is_some_and(|n| !n.is_empty()),
        None => inner
            .strip_suffix(":harness last")
            .or_else(|| inner.strip_suffix(":harness"))
            .is_some_and(|n| !n.is_empty()),
    }
}

/// `block` returns pakf's block: the text of each pack of `texts`, a pack's
/// id with the text of its `instructions.md`, in the order given, each
/// after the line `<!-- pack: <ID> -->` and a blank line, and `None` when
/// no text holds anything but whitespace. A line of a text that reads as a
/// marker of the block of any tool takes a space before it, so that
/// agent-harness-adapter finds pakf's block whole, with no other block in
/// it.
// sokf-implements: INST-1_AC-2
pub(crate) fn block(texts: &[(String, String)]) -> Option<String> {
    let mut out = String::new();
    for (name, text) in texts.iter().filter(|(_, t)| !t.trim().is_empty()) {
        if !out.is_empty() {
            out.push('\n');
        }
        let _ = write!(out, "<!-- pack: {name} -->\n\n");
        out.push_str(&escape(text.trim_start_matches(['\n', '\r'])));
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    (!out.is_empty()).then_some(out)
}

/// `escape` returns `text` with a space before each line that reads as a
/// marker of the block of any tool, which agent-harness-adapter would read
/// as the end of pakf's block, the start of another block, or a text to
/// keep that it refuses.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if is_marker(line.trim_end()) {
            out.push(' ');
        }
        out.push_str(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn each_text_follows_the_line_that_names_its_pack() {
        assert_eq!(block(&[]), None);
        assert_eq!(block(&[("@a/m".into(), " \n\n".into())]), None);
        assert_eq!(
            block(&[
                ("@a/m".into(), "Write the issue first.\n".into()),
                ("@a/blank".into(), "\n".into()),
                ("@a/s".into(), "\nWrite short sentences.".into()),
            ])
            .unwrap(),
            "<!-- pack: @a/m -->\n\nWrite the issue first.\n\n<!-- pack: @a/s -->\n\nWrite short sentences.\n"
        );
        // A marker of any tool in a text can't end the block early or open
        // another block in it.
        assert_eq!(
            block(&[(
                "@a/m".into(),
                "Before.\n<!-- /pakf:harness -->\n<!-- pakf:harness last -->\n<!-- sokf:harness -->\n<!-- /sokf:harness -->\nAfter.\n".into()
            )])
            .unwrap(),
            "<!-- pack: @a/m -->\n\nBefore.\n <!-- /pakf:harness -->\n <!-- pakf:harness last -->\n <!-- sokf:harness -->\n <!-- /sokf:harness -->\nAfter.\n"
        );
        // A line that only looks like a marker stays.
        for line in [
            "<!-- :harness -->",
            "<!-- pakf:harnessx -->",
            "<!-- pack: @a/m -->",
            "<!-- /pakf:harness last -->",
        ] {
            assert!(!is_marker(line), "{line}");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        // sokf-tests: INST_P-1
        #[test]
        fn the_block_holds_each_text_once_in_order(
            texts in proptest::collection::vec("[a-z]{1,8}( [a-z]{1,8}){0,4}\n?| *\n?|<!-- /pakf:harness -->\n|<!-- sokf:harness last -->\n", 0..6)
        ) {
            let packs: Vec<(String, String)> = texts
                .iter()
                .enumerate()
                .map(|(i, t)| (format!("@p/n{i}"), t.clone()))
                .collect();
            let out = block(&packs);
            let texts: Vec<&(String, String)> =
                packs.iter().filter(|(_, t)| !t.trim().is_empty()).collect();
            prop_assert_eq!(out.is_none(), texts.is_empty());
            let out = out.unwrap_or_default();
            // The block holds no marker of a tool, so no text ends it.
            prop_assert!(!out.lines().any(is_marker), "{}", out);
            let mut at = 0;
            for (name, text) in texts {
                let text = if is_marker(text.trim_end()) {
                    format!(" {}", text.trim_end())
                } else {
                    text.trim_end().to_string()
                };
                let item = format!("<!-- pack: {name} -->\n\n{text}");
                let found = out[at..].find(&item);
                prop_assert!(found.is_some(), "{} in {}", item, out);
                at += found.unwrap() + item.len();
                prop_assert_eq!(out.matches(&format!("<!-- pack: {name} -->\n")).count(), 1);
            }
        }
    }
}
