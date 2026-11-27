//! The `harness` module hands pakf's block of the packs' instructions to
//! agent-harness-adapter after each command that changes the lock. The
//! adapter decides which harnesses and files the block reaches, so pakf
//! reads and writes no file of a harness, and knows none.

mod block;
mod refresh;
mod words;

pub use refresh::Instructions;
pub(crate) use refresh::{kept_text, refresh};

#[cfg(test)]
mod tests;
