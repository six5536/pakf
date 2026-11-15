//! This module holds the parts of a pack that `pakf remove --keep` keeps as
//! the project's own. The harness module hands the pack's instructions to
//! agent-harness-adapter, which keeps them.

/// A `PackKeep` holds the parts of a pack that `pakf remove` keeps as the
/// project's own, from its `--keep`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PackKeep {
    /// Whether `files` or `all` is given: every file of the pack stays.
    pub files: bool,
    /// Whether `instructions` or `all` is given: the pack's instructions
    /// stay as the project's own text, where agent-harness-adapter keeps
    /// them.
    pub instructions: bool,
}
