//! This module puts the warnings of agent-harness-adapter that name its own
//! forced install or forced uninstall of pakf's block into pakf's words,
//! because pakf has neither: the user deletes the block with its markers,
//! and `pakf sync` then writes the packs' version. Every other line of the
//! report stays as it stands.

use std::fmt::Write as _;

/// `PART` is the part of a harness that pakf's block is, as the adapter
/// names it.
const PART: &str = "instructions";

/// `CAUSES` maps each reason of the adapter why its record can't tell an
/// edit from what it wrote to pakf's words for it.
const CAUSES: [(&str, &str); 4] = [
    (
        "its record is from an older version",
        "an older pakf recorded it",
    ),
    (
        "its record is from a later version",
        "a later pakf recorded it",
    ),
    (
        "its record was made under other rules",
        "another version of pakf recorded it",
    ),
    (
        "its record doesn't fit the part",
        "the record of the harnesses doesn't fit it",
    ),
];

/// `Unvouched` is a warning of the adapter that its record can't vouch for
/// pakf's block in a file: the harness, the file, pakf's words for the
/// cause, and whether the adapter's remedy is a forced install, rather
/// than a forced uninstall.
#[derive(Debug, PartialEq, Eq)]
struct Unvouched<'a> {
    harness: &'a str,
    path: &'a str,
    cause: &'static str,
    install: bool,
}

/// `unvouched` reads `line` as the adapter's warning of the form
/// `warning: <harness>: instructions in <path> may not be an edit, as
/// <why>; <remedy>`, with one of the causes of [`CAUSES`] and one of the
/// two remedies, and returns `None` for any other line.
fn unvouched(line: &str) -> Option<Unvouched<'_>> {
    let rest = line.strip_prefix("warning: ")?;
    let (harness, rest) = rest.split_once(": ")?;
    let rest = rest.strip_prefix(PART)?.strip_prefix(" in ")?;
    let (path, rest) = rest.split_once(" may not be an edit, as ")?;
    let (why, remedy) = rest.split_once("; ")?;
    let cause = CAUSES.iter().find(|(w, _)| *w == why)?.1;
    let install = if remedy == format!("install --force {PART} writes the tool's version") {
        true
    } else if remedy == format!("a forced uninstall of {harness} removes it") {
        false
    } else {
        return None;
    };
    Some(Unvouched {
        harness,
        path,
        cause,
        install,
    })
}

/// `reword` returns the report `report` of agent-harness-adapter with each
/// warning that names the adapter's forced install or forced uninstall of
/// pakf's block in pakf's words, and, after them, one line that says what
/// the user does: delete each such block with its markers, and run `pakf
/// sync` where pakf writes the block again.
// sokf-implements: INST-1_AC-19
// sokf-implements: INST-1_AC-20
// sokf-implements: INST-1_AC-21
// sokf-implements: INST-1_AC-22
pub(crate) fn reword(report: &str) -> String {
    let mut out = String::with_capacity(report.len());
    let mut count = 0;
    let mut install = false;
    for line in report.split_inclusive('\n') {
        let text = line.strip_suffix('\n').unwrap_or(line);
        match unvouched(text) {
            Some(w) => {
                let _ = writeln!(
                    out,
                    "warning: {}: pakf's block in {} may not be an edit, as {}",
                    w.harness, w.path, w.cause
                );
                count += 1;
                install |= w.install;
            }
            None => out.push_str(line),
        }
    }
    if count > 0 {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        let which = if count == 1 {
            "the block"
        } else {
            "each such block"
        };
        let then = if install {
            ", then run `pakf sync`"
        } else {
            ""
        };
        let _ = writeln!(out, "note: delete {which} with its markers{then}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `warning` returns the adapter's warning for `harness`'s block in
    /// `path` with the reason `why`, and the remedy of a forced install, or
    /// of a forced uninstall where `install` is false.
    fn warning(harness: &str, path: &str, why: &str, install: bool) -> String {
        let remedy = if install {
            "install --force instructions writes the tool's version".to_string()
        } else {
            format!("a forced uninstall of {harness} removes it")
        };
        format!(
            "warning: {harness}: instructions in {path} may not be an edit, as {why}; {remedy}\n"
        )
    }

    // sokf-tests: INST-1_AC-19
    // sokf-tests: INST-1_AC-20
    #[test]
    fn a_forced_install_is_a_deletion_and_a_sync() {
        let report = format!(
            "claude:\n  edited  CLAUDE.md (instructions)\n{}",
            warning(
                "claude",
                "CLAUDE.md",
                "its record is from an older version",
                true
            )
        );
        assert_eq!(
            reword(&report),
            "claude:\n  edited  CLAUDE.md (instructions)\nwarning: claude: pakf's block in CLAUDE.md may not be an edit, as an older pakf recorded it\nnote: delete the block with its markers, then run `pakf sync`\n"
        );
    }

    // sokf-tests: INST-1_AC-19
    // sokf-tests: INST-1_AC-21
    #[test]
    fn a_forced_uninstall_is_a_deletion() {
        let report = warning(
            "claude",
            "CLAUDE.md",
            "its record is from an older version",
            false,
        );
        assert_eq!(
            reword(&report),
            "warning: claude: pakf's block in CLAUDE.md may not be an edit, as an older pakf recorded it\nnote: delete the block with its markers\n"
        );
    }

    // sokf-tests: INST-1_AC-20
    // sokf-tests: INST-1_AC-21
    #[test]
    fn several_warnings_take_one_note() {
        let older = "its record is from an older version";
        let two_installs = format!(
            "{}{}",
            warning("claude", "CLAUDE.md", older, true),
            warning("codex", "AGENTS.md", older, true)
        );
        let out = reword(&two_installs);
        assert_eq!(out.matches("note: ").count(), 1, "{out}");
        assert!(
            out.ends_with("note: delete each such block with its markers, then run `pakf sync`\n"),
            "{out}"
        );
        // A block that pakf writes again needs the sync, also beside one
        // that it takes out.
        let mixed = format!(
            "{}{}",
            warning("codex", "AGENTS.md", older, false),
            warning("claude", "CLAUDE.md", older, true)
        );
        assert!(reword(&mixed).ends_with(", then run `pakf sync`\n"));
        let two_uninstalls = format!(
            "{}{}",
            warning("codex", "AGENTS.md", older, false),
            warning("claude", "CLAUDE.md", older, false)
        );
        assert!(
            reword(&two_uninstalls).ends_with("note: delete each such block with its markers\n")
        );
    }

    // sokf-tests: INST-1_AC-22
    #[test]
    fn each_cause_takes_pakfs_words() {
        for (why, cause) in CAUSES {
            let out = reword(&warning("claude", "CLAUDE.md", why, true));
            assert!(
                out.starts_with(&format!(
                    "warning: claude: pakf's block in CLAUDE.md may not be an edit, as {cause}\n"
                )),
                "{out}"
            );
        }
    }

    // A line of another form, such as another warning of the adapter, a
    // warning of the same form for another part, or with a cause or a
    // remedy that the adapter didn't write, stays as it stands.
    #[test]
    fn every_other_line_stays_as_it_stands() {
        for line in [
            "claude:\n  current CLAUDE.md (instructions)\n",
            "warning: cursor may load the instructions twice: CLAUDE.md, AGENTS.md\n",
            "warning: claude: skills in .claude/skills may not be an edit, as its record is from an older version; install --force skills writes the tool's version\n",
            "warning: claude: instructions in CLAUDE.md may not be an edit, as the moon is full; install --force instructions writes the tool's version\n",
            "warning: claude: instructions in CLAUDE.md may not be an edit, as its record is from an older version; run something else\n",
            "warning: claude: instructions in CLAUDE.md may not be an edit, as its record is from an older version; a forced uninstall of codex removes it\n",
            "error: `pakf` is no tool name\n",
            "",
        ] {
            assert_eq!(reword(line), line);
        }
        // A report that ends without a newline keeps its last line whole.
        let last = warning(
            "claude",
            "CLAUDE.md",
            "its record is from an older version",
            true,
        );
        let out = reword(last.trim_end());
        assert!(
            out.starts_with("warning: claude: pakf's block in CLAUDE.md"),
            "{out}"
        );
        assert!(out.ends_with("then run `pakf sync`\n"), "{out}");
    }
}
