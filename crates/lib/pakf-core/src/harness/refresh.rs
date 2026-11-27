//! This module hands pakf's block to agent-harness-adapter after each
//! command that changes the lock: it reads the copies of the packs'
//! instructions, builds the block, and asks the adapter to follow it, with
//! the text that `pakf remove --keep instructions` keeps. The adapter
//! decides every harness and every file, and returns a report and an exit
//! code, which the command passes on as they stand, apart from the warnings
//! that name the adapter's own forced install, which `words` puts in
//! pakf's words.

use std::path::Path;

use agent_harness_adapter_core::{BlockOrder, Request, Scope};

use super::{block, words};
use crate::{
    Error, RepoRoot, Result,
    pack::{self, Dest, Lock},
};

/// The `NAME` of pakf as a tool of agent-harness-adapter, which sets the
/// markers of its block and the place of the adapter's files for it.
const NAME: &str = "pakf";

/// `Instructions` is what a command did with pakf's block: the report and
/// the exit code of agent-harness-adapter, in pakf's words, or the line of
/// a copy that holds the block, when the command handed the adapter
/// nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Instructions {
    /// The report of agent-harness-adapter, as it stands apart from the
    /// warnings that name its forced install, and empty when the command
    /// handed the adapter nothing.
    pub report: String,
    /// The exit code of agent-harness-adapter: 0 done, 1 a block that the
    /// project edited left as the adapter found it, and 2 a refusal.
    pub exit: u8,
    /// The `kept` line while a copy of a pack's `instructions.md` holds the
    /// block, which names the copy, says how to free it, and that the next
    /// sync then updates the block.
    pub held: Option<String>,
}

/// `Copies` are the copies of the packs' `instructions.md`, in the order
/// of the packs: the text of each readable one, and each that holds the
/// block, because it holds a conflict or isn't readable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Copies {
    /// Each pack's id with the text of its copy.
    pub texts: Vec<(String, String)>,
    /// The path of each copy that holds a conflict.
    pub conflicted: Vec<String>,
    /// The path of each copy that isn't readable, with why.
    pub unreadable: Vec<(String, String)>,
}

impl Copies {
    /// `held` returns whether a copy holds the block.
    fn held(&self) -> bool {
        !self.conflicted.is_empty() || !self.unreadable.is_empty()
    }

    /// `why` returns why the copies hold the block, such as
    /// `/.pakf/packs/@a/m/instructions.md holds a conflict`.
    pub(crate) fn why(&self) -> String {
        let mut out: Vec<String> = self
            .conflicted
            .iter()
            .map(|p| format!("{p} holds a conflict"))
            .collect();
        out.extend(
            self.unreadable
                .iter()
                .map(|(p, why)| format!("{p} can't be read: {why}")),
        );
        out.join("; ")
    }

    /// `fix` returns what the user does to free the block.
    pub(crate) fn fix(&self) -> &'static str {
        match (self.conflicted.len(), self.unreadable.len()) {
            (1, 0) => "resolve it",
            (_, 0) => "resolve them",
            (0, 1) => "replace it with a readable file",
            (0, _) => "replace them with readable files",
            _ => "resolve each conflict and replace each copy that can't be read",
        }
    }
}

/// `read_copy` reads the copy of the `instructions.md` of the pack `name`,
/// whose record in the lock is `record`, into `out`: nothing when it is
/// absent, its text, and whether it holds a conflict, as
/// `LockedFile::holds_conflict` reads one, or isn't readable, as a regular
/// file of UTF-8 text.
fn read_copy(root: &RepoRoot, name: &str, record: &pack::LockedFile, out: &mut Copies) {
    let path = pack::instructions_of(name);
    let shown = path.as_str().to_string();
    let conflicted = |bytes: &[u8]| record.holds_conflict(root, name, INSTRUCTIONS, bytes);
    match pack::read_dest(root, &path) {
        Dest::Absent => {}
        Dest::File(bytes) => match String::from_utf8(bytes) {
            Ok(text) => {
                if conflicted(text.as_bytes()) {
                    out.conflicted.push(shown);
                }
                out.texts.push((name.to_string(), text));
            }
            Err(e) => {
                if conflicted(e.as_bytes()) {
                    out.conflicted.push(shown);
                } else {
                    out.unreadable.push((shown, "its text isn't UTF-8".into()));
                }
            }
        },
        Dest::Unreadable(why) => out.unreadable.push((shown, why)),
    }
}

/// `INSTRUCTIONS` is the path of a pack's instructions in the pack, by which
/// the lock records the copy.
const INSTRUCTIONS: &str = "instructions.md";

/// `read_copies` reads the copy of the `instructions.md` of each pack of
/// `lock` that records one, in the order of the packs, so that a copy that
/// a pack no longer holds, or a file that no pack placed, gives the block
/// no text.
// sokf-implements: INST-1_AC-2
pub(crate) fn read_copies(root: &RepoRoot, lock: &Lock) -> Copies {
    let mut out = Copies::default();
    for name in pack::ordered(root, lock) {
        if let Some(record) = lock
            .packs
            .get(&name)
            .and_then(|p| p.files.get(INSTRUCTIONS))
        {
            read_copy(root, &name, record, &mut out);
        }
    }
    out
}

/// `request` returns the request of pakf to agent-harness-adapter at the
/// project's scope of the repository `root`, with pakf's block after the
/// block of every other tool, the block `block`, or no instructions where
/// it is `None`, and the text to keep `keep`.
// sokf-implements: INST-1_AC-1
// sokf-implements: INST-1_AC-13
// sokf-implements: INST-1_AC-15
fn request(root: &Path, block: Option<String>, keep: Option<&str>) -> Request {
    let mut out = Request::new(NAME, Scope::Project)
        .root(root)
        .block_order(BlockOrder::Last);
    if let Some(block) = block {
        out = out.instructions(block);
    }
    if let Some(text) = keep {
        out = out.keep(text);
    }
    out
}

/// `refresh` hands pakf's block of the packs of `lock`, which the command
/// has written, to agent-harness-adapter, with the text to keep `keep`, and
/// returns the report that the adapter gives, in pakf's words, and its exit
/// code. While a copy of a pack's instructions holds a conflict or isn't
/// readable, it hands the adapter nothing, and returns the line that names
/// the copy.
// sokf-implements: INST-1_AC-6
// sokf-implements: INST-1_AC-7
pub(crate) fn refresh(root: &RepoRoot, lock: &Lock, keep: Option<&str>) -> Instructions {
    let copies = read_copies(root, lock);
    if copies.held() {
        return Instructions {
            held: Some(format!(
                "kept pakf's block as it stands: {}; {}, and the next `pakf sync` updates the block",
                copies.why(),
                copies.fix()
            )),
            ..Instructions::default()
        };
    }
    let out = agent_harness_adapter_core::follow(&request(
        root.as_path(),
        block::block(&copies.texts),
        keep,
    ));
    Instructions {
        report: words::reword(&out.report),
        exit: out.exit,
        held: None,
    }
}

/// `kept_text` returns the text of the copy of the pack `name`'s
/// `instructions.md`, which `pakf remove --keep instructions` hands to
/// agent-harness-adapter to keep as the project's own, and `None` for a
/// pack without a copy that holds text, which has nothing to keep, with a
/// space before each line that reads as a marker of a tool's block, which
/// the adapter refuses in a text to keep, as in the block. It
/// refuses a copy of any pack of `lock` that holds a conflict or isn't
/// readable, before the removal writes, because the kept text would carry
/// the conflict's markers, and a held copy hands the adapter nothing.
// sokf-implements: CLI-7_AC-19
// sokf-implements: CLI-7_AC-20
pub(crate) fn kept_text(root: &RepoRoot, lock: &Lock, name: &str) -> Result<Option<String>> {
    let copies = read_copies(root, lock);
    if copies.held() {
        return Err(Error::Pack(format!(
            "--keep instructions: {}; {}, then remove the pack again",
            copies.why(),
            copies.fix()
        )));
    }
    Ok(copies
        .texts
        .into_iter()
        .find(|(n, t)| n == name && !t.trim().is_empty())
        .map(|(_, t)| block::escape(&t)))
}
