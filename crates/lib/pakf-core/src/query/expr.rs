//! `Expr` is a compiled JMESPath expression over `serde_json::Value`. The
//! `jmespath` crate parses the expression, and the `eval` module walks its
//! syntax tree, so an object read from the input keeps its key order.

use std::fmt;

use serde_json::Value;

use super::eval;
use crate::{Error, Result};

/// `Expr` is a compiled JMESPath expression, kept with the source text it
/// was compiled from.
#[derive(Clone)]
pub struct Expr {
    source: String,
    compiled: jmespath::Expression<'static>,
}

impl Expr {
    /// `compile` compiles `source`. A parse failure is an [`Error::Query`]
    /// that carries the `jmespath` crate's message.
    // sokf-implements: CLI-1_AC-18
    pub fn compile(source: &str) -> Result<Expr> {
        let compiled = jmespath::compile(source)
            .map_err(|e| Error::Query(e.to_string().trim_end().to_string()))?;
        Ok(Expr {
            source: source.to_string(),
            compiled,
        })
    }

    /// `source` returns the expression as written.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// `eval` evaluates the expression against `value`. An object in the
    /// result keeps the key order that it was read with. An object literal in
    /// the expression is the exception: its keys come out sorted, because the
    /// `jmespath` crate parses it into a sorted map. A runtime
    /// failure is an [`Error::Query`].
    // sokf-implements: CLI-1_AC-17
    // sokf-implements: CLI-1_AC-18
    pub fn eval(&self, value: &Value) -> Result<Value> {
        eval::eval(self.compiled.as_ast(), value).map_err(|e| Error::Query(e.to_string()))
    }
}

impl fmt::Debug for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Expr").field(&self.source).finish()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn compiles_or_refuses_with_the_crate_message() {
        let e = Expr::compile("type=='Decision'").unwrap();
        assert_eq!(e.source(), "type=='Decision'");
        assert!(format!("{e:?}").contains("type=='Decision'"));
        let err = Expr::compile("nodes[?").unwrap_err();
        assert!(matches!(err, Error::Query(_)));
        assert!(err.to_string().starts_with("query: "), "{err}");
    }

    #[test]
    fn eval_projects_and_reports_runtime_failures() {
        let e = Expr::compile("nodes[?kind=='document'].id").unwrap();
        let value = json!({"nodes": [
            {"kind": "document", "id": "a"},
            {"kind": "file"},
            {"kind": "document", "id": "b"}
        ]});
        assert_eq!(e.eval(&value).unwrap(), json!(["a", "b"]));
        assert_eq!(
            Expr::compile("nope").unwrap().eval(&value).unwrap(),
            Value::Null
        );
        let bad = Expr::compile("contains(tags, 'core')").unwrap();
        assert!(matches!(bad.eval(&json!({})), Err(Error::Query(_))));
    }
}
