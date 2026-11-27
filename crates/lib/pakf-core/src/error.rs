//! The core has one error type, `Error`. The command-line interface (CLI)
//! exits with code 2 for every variant.

use std::path::PathBuf;

use crate::RepoPath;

/// `Result` is the result type of this crate, with `Error` as its error.
pub type Result<T> = std::result::Result<T, Error>;

/// `Error` holds the errors of this crate. Every variant renders a message
/// that the CLI prints unchanged after `error: `.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The tool finds no repository root.
    #[error("not in a Git repository; pass --repo-root")]
    NoRepository,

    /// The tool refuses a configuration file.
    #[error("{path}: {message}")]
    Config {
        /// `path` is the path of the configuration file that the tool refuses.
        path: RepoPath,
        /// The message says what is wrong with the file.
        message: String,
    },

    /// A command refuses to run, and the message names the pack, the
    /// source, or the file at fault.
    #[error("pack: {0}")]
    Pack(String),

    /// A JMESPath expression doesn't compile or fails to evaluate.
    #[error("query: {0}")]
    Query(String),

    /// An input or output (I/O) operation fails at a path.
    #[error("I/O error at {path}: {source}")]
    Io {
        /// `path` is the path at which the operation fails.
        path: PathBuf,
        /// `source` is the I/O error that causes the failure.
        #[source]
        source: std::io::Error,
    },

    /// The code breaks one of its own invariants.
    #[error("internal: {0}")]
    Internal(String),

    /// A command fails after its comparison of the files warned of a
    /// formatter or a command of `[compare]`, and the CLI prints the
    /// warnings before the error, whose message this is. A caller that
    /// matches the kind of an error matches the error that
    /// [`Error::into_warnings`] returns.
    #[error("{error}")]
    Warned {
        /// `error` is the error of the command.
        error: Box<Error>,
        /// `warnings` are the warnings of the comparison, each without the
        /// `warning: ` before it.
        warnings: Vec<String>,
    },
}

/// An error of agent-harness-adapter, whose atomic write the tool uses, is
/// an I/O error at its path, or else an internal error.
impl From<agent_harness_adapter_core::Error> for Error {
    fn from(e: agent_harness_adapter_core::Error) -> Self {
        match e {
            agent_harness_adapter_core::Error::Io { path, source } => Error::Io { path, source },
            agent_harness_adapter_core::Error::Internal(m) => Error::Internal(m),
            other => Error::Internal(other.to_string()),
        }
    }
}

impl Error {
    /// `io` builds an [`Error::Io`] for `path` from a [`std::io::Error`].
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    /// `config` builds an [`Error::Config`].
    pub fn config(path: RepoPath, message: impl Into<String>) -> Self {
        Error::Config {
            path,
            message: message.into(),
        }
    }

    /// `warned` returns the error with the warnings `warnings` of the
    /// command's comparison after any that it holds, or the error as it is
    /// where `warnings` is empty.
    #[must_use]
    pub fn warned(self, warnings: Vec<String>) -> Self {
        if warnings.is_empty() {
            return self;
        }
        match self {
            Error::Warned {
                error,
                warnings: mut held,
            } => {
                held.extend(warnings);
                Error::Warned {
                    error,
                    warnings: held,
                }
            }
            error => Error::Warned {
                error: Box::new(error),
                warnings,
            },
        }
    }

    /// `into_warnings` returns the warnings that the error holds, which come
    /// before it, and the error without them.
    pub fn into_warnings(self) -> (Vec<String>, Error) {
        match self {
            Error::Warned { error, warnings } => (warnings, *error),
            error => (Vec::new(), error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error as _, io, path::Path};

    use super::*;

    #[test]
    fn io_constructor_and_display() {
        let e = Error::io(
            Path::new("/tmp/x"),
            io::Error::new(io::ErrorKind::PermissionDenied, "denied"),
        );
        assert!(matches!(e, Error::Io { .. }));
        let msg = e.to_string();
        assert!(msg.contains("/tmp/x"), "{msg}");
        assert!(msg.contains("denied"), "{msg}");
        assert!(e.source().is_some());
    }

    #[test]
    fn an_adapter_error_is_an_io_error_or_an_internal_one() {
        let e: Error = agent_harness_adapter_core::Error::io(
            "/tmp/x",
            io::Error::new(io::ErrorKind::PermissionDenied, "denied"),
        )
        .into();
        assert!(matches!(e, Error::Io { .. }), "{e}");
        let e: Error = agent_harness_adapter_core::Error::Internal("bug".into()).into();
        assert_eq!(e.to_string(), "internal: bug");
        let e: Error = agent_harness_adapter_core::Error::file("/a.toml", "bad").into();
        assert_eq!(e.to_string(), "internal: /a.toml: bad");
    }

    #[test]
    fn a_warned_error_keeps_its_message_and_its_warnings_in_order() {
        let e = Error::Pack("refused".into());
        assert!(matches!(e.warned(Vec::new()), Error::Pack(_)));
        let e = Error::Pack("refused".into())
            .warned(vec!["a".into()])
            .warned(vec!["b".into()]);
        assert_eq!(e.to_string(), "pack: refused");
        let (warnings, inner) = e.into_warnings();
        assert_eq!(warnings, ["a", "b"]);
        assert!(matches!(inner, Error::Pack(_)));
        let (warnings, _) = Error::NoRepository.into_warnings();
        assert_eq!(warnings, Vec::<String>::new());
    }

    #[test]
    fn every_message_names_its_subject() {
        let path = RepoPath::parse("/a/config.toml").unwrap();
        assert_eq!(
            Error::config(path, "unknown key").to_string(),
            "/a/config.toml: unknown key"
        );
        assert_eq!(Error::Pack("x".into()).to_string(), "pack: x");
        assert_eq!(Error::Query("x".into()).to_string(), "query: x");
        assert_eq!(Error::Internal("x".into()).to_string(), "internal: x");
        assert!(Error::NoRepository.to_string().contains("--repo-root"));
        assert!(Error::NoRepository.source().is_none());
    }
}
