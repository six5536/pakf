//! This module parses and evaluates JMESPath expressions, which `--query`
//! uses as projections (CLI-1_AC-17). The `jmespath` crate parses an expression. The `eval`
//! module walks the syntax tree over a `serde_json::Value`. An object
//! therefore keeps the key order that it was read or written with.

#[cfg(test)]
mod compliance;
mod eval;
mod expr;
mod functions;

pub use expr::Expr;
