//! YAML 1.2 core-schema parsing with the line of each value, for the
//! manifest of a pack and for a pack's formats.

mod parse;
mod value;

pub use parse::parse;
pub use value::{Spanned, Yaml, YamlError};
