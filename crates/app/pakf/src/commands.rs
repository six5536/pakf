//! This module runs the commands: `sync`, `add`, `status`, `ls`, `accept`,
//! `remove`, `diff`, and `patch`.

use std::{ffi::OsStr, fmt::Write as _, io::IsTerminal as _, path::Path};

use pakf_core::{
    Error, Expr, PackAcceptOptions, PackAddOptions, PackDiffOptions, PackKeep, PackLsAcceptance,
    PackLsOptions, PackLsState, PackPatchOptions, PackSyncOptions, PackSyncReport,
};

use crate::{
    cli::{
        AcceptanceArg, ColorArg, Command, KeepArg, PackAcceptArgs, PackAddArgs, PackLsArgs,
        PackPatchArgs, PackRemoveArgs, PackStatusArgs, PackSyncArgs, StateArg,
    },
    output,
};

/// `run` runs the command `command`. Only `sync` and
/// `add` use the network. `sync`, `add`, and `remove` exit with code 1 when
/// a merge conflicts, or with the code of agent-harness-adapter where it is
/// higher, `status --check` exits with code 1 when a file of a
/// pack differs from its base, other than by the edit of an editable file,
/// and its acceptance isn't `accepted`, and every
/// command exits with code 2 when it refuses to run. With `--json`, each
/// command that reports prints its report as one JSON value, with the same
/// exit code, and with `--query`, the value that the expression selects from
/// it, which the command compiles before it reads the tree, so that a bad
/// expression fails first.
// sokf-implements: CLI-2_AC-9
pub fn run(repo_root: Option<&Path>, command: Command) -> pakf_core::Result<u8> {
    // A command that fails after its comparison warned prints the warnings
    // before its error.
    // sokf-implements: CLI-1_AC-45
    let (text, code) = outcome(repo_root, command).map_err(|e| {
        let (warnings, error) = e.into_warnings();
        warn(&warnings);
        error
    })?;
    output::write_stdout(text.as_bytes())?;
    Ok(code)
}

/// `outcome` runs the command `command` and returns its text and its exit
/// code.
fn outcome(repo_root: Option<&Path>, command: Command) -> pakf_core::Result<(String, u8)> {
    Ok(match command {
        Command::Sync(a) => sync(repo_root, a)?,
        Command::Add(a) => add(repo_root, a)?,
        Command::Status(a) => status(repo_root, &a)?,
        Command::Ls(a) => ls(repo_root, a)?,
        Command::Accept(a) => accept(repo_root, a)?,
        Command::Remove(a) => remove(repo_root, a)?,
        // A file that the patch leaves out gets a note on stderr, so the
        // patch on stdout still applies.
        // sokf-implements: CLI-8_AC-14
        Command::Diff(a) => {
            let (cwd, root) = output::root(repo_root)?;
            let diff = pakf_core::pack_diff_with_notes(
                &root,
                &PackDiffOptions {
                    name: a.name,
                    accepted: a.accepted,
                    paths: a.paths,
                    cwd,
                },
            )?;
            for note in &diff.notes {
                eprintln!("note: pakf diff: {note}");
            }
            warn(&diff.warnings);
            (diff.patch, output::EXIT_OK)
        }
        Command::Patch(a) => patch(repo_root, a)?,
    })
}

/// `patch` makes the patch of the pack that `args` names, and prints it,
/// writes it to the file of `-o`, or with `--apply` prints a line for each
/// file that it applied. A file that the patch leaves out gets a note on
/// stderr.
// sokf-implements: CLI-8_AC-14
// sokf-implements: CLI-9_AC-10
fn patch(repo_root: Option<&Path>, args: PackPatchArgs) -> pakf_core::Result<(String, u8)> {
    let (cwd, root) = output::root(repo_root)?;
    let file = args.output.map(|o| cwd.join(o));
    let patch = pakf_core::pack_patch(
        &root,
        &PackPatchOptions {
            name: args.name,
            accepted: args.accepted,
            paths: args.paths,
            recursive: args.recursive,
            apply: args.apply,
            cwd,
        },
    )?;
    for note in &patch.notes {
        eprintln!("note: pakf patch: {note}");
    }
    warn(&patch.warnings);
    if let Some(file) = file {
        std::fs::write(&file, &patch.patch).map_err(|e| Error::io(&file, e))?;
        return Ok((String::new(), output::EXIT_OK));
    }
    let text = if args.apply {
        patch.applied.iter().fold(String::new(), |mut out, path| {
            let _ = writeln!(out, "applied {path}");
            out
        })
    } else {
        patch.patch
    };
    Ok((text, output::EXIT_OK))
}

/// `sync` syncs the declared packs, or the one that `args` names.
fn sync(repo_root: Option<&Path>, args: PackSyncArgs) -> pakf_core::Result<(String, u8)> {
    let (_, root) = output::root(repo_root)?;
    let report = pakf_core::pack_sync(
        &root,
        &PackSyncOptions {
            name: args.name,
            keep_existing: args.keep_existing,
        },
    )?;
    synced(&report, args.json)
}

/// `add` declares the pack of a source and syncs it.
fn add(repo_root: Option<&Path>, args: PackAddArgs) -> pakf_core::Result<(String, u8)> {
    let (cwd, root) = output::root(repo_root)?;
    let report = pakf_core::pack_add(
        &root,
        &PackAddOptions {
            source: output::required(args.source, "SOURCE")?,
            git_ref: args.git_ref,
            path: args.path,
            keep_existing: args.keep_existing,
            cwd,
        },
    )?;
    synced(&report, args.json)
}

/// `synced` returns the report of `sync` or `add`, as text or with `json` as
/// the JSON object, and exits with code 1 when a merge conflicts, or with
/// the code of agent-harness-adapter where it is higher.
fn synced(report: &PackSyncReport, json: bool) -> pakf_core::Result<(String, u8)> {
    let code = if report.conflicted() {
        output::EXIT_ERRORS
    } else {
        output::EXIT_OK
    };
    // sokf-implements: INST-1_AC-18
    let code = code.max(report.instructions.exit);
    let text = if json {
        output::json_text(&report.to_json(), None)?
    } else {
        report.to_text()
    };
    Ok((text, code))
}

/// `status` reports each pack in the lock, as text, as the JSON array of
/// `--json`, or as the value that `--query` selects from it.
// sokf-implements: CLI-4_AC-12
fn status(repo_root: Option<&Path>, args: &PackStatusArgs) -> pakf_core::Result<(String, u8)> {
    let query = args.query.as_deref().map(Expr::compile).transpose()?;
    let (_, root) = output::root(repo_root)?;
    let status = pakf_core::pack_status(&root)?;
    warn(&status.warnings);
    // sokf-implements: CLI-4_AC-3
    let code = if args.check && status.drifted() {
        output::EXIT_ERRORS
    } else {
        output::EXIT_OK
    };
    let text = if args.json {
        output::json_text(&status.to_json(), query.as_ref())?
    } else {
        status.to_text()
    };
    Ok((text, code))
}

/// `accept` records or withdraws the acceptance of each named file.
fn accept(repo_root: Option<&Path>, args: PackAcceptArgs) -> pakf_core::Result<(String, u8)> {
    let (cwd, root) = output::root(repo_root)?;
    let report = pakf_core::pack_accept(
        &root,
        &PackAcceptOptions {
            paths: args.paths,
            note: args.note,
            withdraw: args.withdraw,
            cwd,
        },
    )?;
    warn(&report.warnings);
    let text = if args.json {
        output::json_text(&report.to_json(), None)?
    } else {
        report.to_text()
    };
    Ok((text, output::EXIT_OK))
}

/// `remove` removes the pack that `args` names, and exits with code 1 when
/// a merge of a file that it hands to another pack conflicts, or with the
/// code of agent-harness-adapter where it is higher.
fn remove(repo_root: Option<&Path>, args: PackRemoveArgs) -> pakf_core::Result<(String, u8)> {
    let (_, root) = output::root(repo_root)?;
    let name = output::required(args.name, "NAME")?;
    let report = pakf_core::pack_remove_keeping(&root, &name, keep(&args.keep))?;
    let code = if report.conflicted() {
        output::EXIT_ERRORS
    } else {
        output::EXIT_OK
    };
    // sokf-implements: INST-1_AC-18
    let code = code.max(report.instructions.exit);
    let text = if args.json {
        output::json_text(&report.to_json(), None)?
    } else {
        report.to_text()
    };
    Ok((text, code))
}

/// `warn` prints on stderr, outside the JSON of the command, each warning of
/// a formatter or a command of `[compare]` that was missing, failed, or ran
/// past its limit.
// sokf-implements: CLI-1_AC-44
fn warn(warnings: &[String]) {
    for w in warnings {
        eprintln!("warning: {w}");
    }
}

/// `keep` returns the parts that the values of `--keep` name: each part that
/// any value names, with `all` for both.
// sokf-implements: CLI-7_AC-15
fn keep(args: &[KeepArg]) -> PackKeep {
    let any = |part| args.iter().any(|a| *a == part || *a == KeepArg::All);
    PackKeep {
        files: any(KeepArg::Files),
        instructions: any(KeepArg::Instructions),
    }
}

/// `ls` lists the entries of the tree with the state of each file of a pack,
/// as text, as the JSON array of `--json`, or as the value that `--query`
/// selects from it. It compiles the expression before it reads the tree, so
/// that a bad expression fails first.
// sokf-implements: CLI-5_AC-20
// sokf-implements: CLI-5_AC-21
fn ls(repo_root: Option<&Path>, args: PackLsArgs) -> pakf_core::Result<(String, u8)> {
    let query = args.query.as_deref().map(Expr::compile).transpose()?;
    let (cwd, root) = output::root(repo_root)?;
    let report = pakf_core::pack_ls(
        &root,
        &PackLsOptions {
            paths: args.paths,
            cwd,
            recursive: args.recursive,
            all: args.all,
            ignored: args.ignored,
            pack: args.pack,
            state: args.state.map(state),
            acceptance: args.acceptance.map(acceptance),
        },
    )?;
    warn(&report.warnings);
    let text = if args.json {
        output::json_text(&report.to_json(), query.as_ref())?
    } else {
        let colour = colour(
            args.color,
            std::io::stdout().is_terminal(),
            std::env::var_os("NO_COLOR").as_deref(),
        );
        report.to_text(args.long, colour)
    };
    Ok((text, output::EXIT_OK))
}

/// `state` returns the state of the core library that `--state` names.
fn state(arg: StateArg) -> PackLsState {
    match arg {
        StateArg::Unchanged => PackLsState::Unchanged,
        StateArg::Edited => PackLsState::Edited,
        StateArg::Conflicted => PackLsState::Conflicted,
        StateArg::Deleted => PackLsState::Deleted,
    }
}

/// `acceptance` returns the filter of the core library that `--acceptance`
/// names.
fn acceptance(arg: AcceptanceArg) -> PackLsAcceptance {
    match arg {
        AcceptanceArg::Accepted => PackLsAcceptance::Accepted,
        AcceptanceArg::Stale => PackLsAcceptance::Stale,
        AcceptanceArg::None => PackLsAcceptance::NoAcceptance,
        AcceptanceArg::Unaccepted => PackLsAcceptance::Unaccepted,
    }
}

/// `colour` returns whether `pakf ls` colours its text: always with
/// `always`, never with `never`, and with `auto` only while stdout is a
/// terminal and `NO_COLOR` is unset or empty, as the convention of
/// `NO_COLOR` defines it.
// sokf-implements: CLI-5_AC-15
// sokf-implements: CLI-5_AC-16
// sokf-implements: CLI-5_AC-17
fn colour(when: ColorArg, terminal: bool, no_color: Option<&OsStr>) -> bool {
    match when {
        ColorArg::Always => true,
        ColorArg::Never => false,
        ColorArg::Auto => terminal && no_color.is_none_or(OsStr::is_empty),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // sokf-tests: CLI-5_AC-15
    // sokf-tests: CLI-5_AC-16
    // sokf-tests: CLI-5_AC-17
    #[test]
    fn auto_colours_only_a_terminal_without_no_color() {
        let set = Some(OsStr::new("1"));
        let empty = Some(OsStr::new(""));
        assert!(colour(ColorArg::Auto, true, None));
        assert!(colour(ColorArg::Auto, true, empty));
        assert!(!colour(ColorArg::Auto, true, set));
        assert!(!colour(ColorArg::Auto, false, None));
        for (terminal, no_color) in [(true, set), (false, None), (false, set)] {
            assert!(colour(ColorArg::Always, terminal, no_color));
            assert!(!colour(ColorArg::Never, terminal, no_color));
        }
    }
}
