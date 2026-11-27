//! This file holds the entry point of the `pakf` command-line tool.
//!
//! **This crate is unstable.** It is at a 0.x version, and its commands,
//! options and file formats can change in any release.
// When the nightly coverage job sets `coverage_nightly` through cargo-llvm-cov,
// this line turns on the attribute that leaves untestable glue code out of the
// coverage report. Normal builds and tests use the stable toolchain, which
// ignores this line.
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![warn(missing_docs)]

mod cli;
mod commands;
mod output;

use std::process::ExitCode;

use clap::{CommandFactory, Parser, error::ErrorKind};

use crate::cli::Cli;

// sokf-implements: CLI-1_AC-7
// sokf-implements: CLI-1_AC-8
fn main() -> ExitCode {
    // `finish` exits with the code that `run` returns. On an error, it prints
    // `error: ` and the message and exits with code 2, unless the program that
    // reads the output closed the pipe. Such a program is `head`, or a pager
    // that the user quit early. A closed pipe isn't a failure of pakf, so pakf
    // prints nothing and exits with success, as ripgrep and other tools do.
    // Rust sets `SIGPIPE` to `SIG_IGN`, so a write to the closed pipe fails
    // with an `EPIPE` error and the `SIGPIPE` signal doesn't kill the process.
    agent_harness_adapter_core::cli::finish(run())
}

// sokf:begin run
fn run() -> pakf_core::Result<u8> {
    let cli = Cli::parse();
    if cli.version {
        // The output is `NAME VERSION`, such as `pakf 1.2.3`, as the usual
        // convention of command-line tools gives it, so that a bug report
        // that quotes the output identifies the tool and its version.
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        return Ok(output::EXIT_OK);
    }
    let Some(command) = cli.command else {
        Cli::command().print_help().ok();
        return Ok(output::EXIT_OK);
    };
    // clap refuses a global option after the command beside the exclusive
    // `--json-schema`, but not one before it, which it parses on `pakf`
    // itself. This check refuses both, so that the position of a global
    // option changes nothing (CLI-1_AC-1).
    // sokf-implements: CLI-1_AC-11
    if let Some(schema) = command.json_schema() {
        if cli.repo_root.is_some() {
            Cli::command()
                .error(
                    ErrorKind::ArgumentConflict,
                    "the argument '--json-schema' cannot be used with one or more of the other specified arguments",
                )
                .exit();
        }
        return output::print_schema(schema);
    }
    commands::run(cli.repo_root.as_deref(), command)
}
// sokf:end run
