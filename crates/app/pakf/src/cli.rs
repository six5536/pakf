//! The command-line interface, declared with clap's derive macros.
//!
//! This module declares the commands and the options that the contract of
//! the command line specifies.

use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use pakf_core::Output;

// sokf-implements: CLI-1_AC-1
// sokf:begin cli
/// Share project files through packs: copy each pack that the project
/// declares into the project, merge each later change of the pack into your
/// edits, and send your edits back to the pack.
#[derive(Debug, Parser)]
// The `version` attribute puts the version in `--help`. The built-in `-V`
// flag is disabled because `main::run` prints the version itself. The
// built-in `-h` flag is disabled so that `help` below can describe `--help`
// in a full sentence. The `about` text is the crate's description without
// its last sentence, the mark of an unstable crate, which crates.io shows but
// the help leaves out. A test of the help checks that the two texts still
// agree.
#[command(
    name = "pakf",
    about = "pakf shares project files through packs.",
    long_about = None,
    version,
    disable_version_flag = true,
    disable_help_flag = true
)]
#[command(
    after_help = "Exit codes: 0 if there are no errors, 1 if there are errors, 2 on a usage or internal error."
)]
pub struct Cli {
    /// `--repo-root` sets the repository root, and pakf otherwise uses the
    /// root of the Git repository that contains the current directory.
    #[arg(long, global = true, value_name = "DIR")]
    pub repo_root: Option<PathBuf>,

    /// The command to run.
    #[command(subcommand)]
    pub command: Option<Command>,

    /// `--version` prints the version and exits.
    #[arg(short = 'V', long = "version", global = true)]
    pub version: bool,

    /// `--help` prints the help of the command and exits.
    #[arg(short = 'h', long = "help", global = true, action = ArgAction::Help)]
    pub help: Option<bool>,
}

// `--query` requires `--json`: clap refuses `--query` alone, and the tool
// exits with code 2.
// sokf-implements: CLI-4_AC-13
/// `Command` lists the commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Sync each pack that `/.pakf/config.toml` declares: merge each
    /// upstream change with your edits, and remove each pack that no entry
    /// declares.
    Sync(PackSyncArgs),
    /// Declare the pack of a source at the end of the `[[pack]]` list of
    /// `/.pakf/config.toml`, and sync it.
    Add(PackAddArgs),
    /// Print each pack in the lock, and each of its files that is edited,
    /// conflicted, or deleted or that has an acceptance, with the
    /// acceptance. It writes nothing and uses no network.
    Status(PackStatusArgs),
    /// List the entries of each directory as `ls` does, with the pack, the
    /// kind, the state, and the acceptance of each file that a pack
    /// delivered. It writes nothing and uses no network.
    Ls(PackLsArgs),
    /// Record that the project keeps your edit or deletion of a pack's file
    /// on purpose, with the reason, so that `pakf status --check` passes on
    /// it while the file holds what you accepted. It writes only the lock.
    Accept(PackAcceptArgs),
    /// Delete the pack's `[[pack]]` entry and the files of the pack that you
    /// haven't edited, keep the edited ones, and drop the pack from the lock.
    /// With `--keep`, keep the pack's files or its instructions as the
    /// project's own.
    Remove(PackRemoveArgs),
    /// Print your edits of the files of every pack, of one pack, or of the
    /// files of PATH, as a unified diff against the recorded bases. It
    /// writes nothing.
    Diff(PackDiffArgs),
    /// Print the patch that sends your edits of a pack's files, and the
    /// files of PATH that the pack doesn't hold yet, back to the pack's
    /// repository, for `git apply`.
    Patch(PackPatchArgs),
}
// sokf:end cli

impl Command {
    /// `json_schema` returns the output whose schema the command prints, when
    /// it runs with `--json-schema`.
    pub fn json_schema(&self) -> Option<Output> {
        match self {
            Command::Sync(a) => a.json_schema.then_some(Output::Sync),
            Command::Add(a) => a.json_schema.then_some(Output::Sync),
            Command::Status(a) => a.json_schema.then_some(Output::Status),
            Command::Ls(a) => a.json_schema.then_some(Output::Ls),
            Command::Accept(a) => a.json_schema.then_some(Output::Accept),
            Command::Remove(a) => a.json_schema.then_some(Output::Remove),
            Command::Diff(_) | Command::Patch(_) => None,
        }
    }
}

// The help of each `--json` flag names `--json-schema`, which prints the
// schema of the output.
// sokf-implements: CLI-1_AC-12
// sokf:begin commands
/// `PackSyncArgs` holds the arguments of `pakf sync`.
#[derive(Debug, Args)]
pub struct PackSyncArgs {
    /// The id of one declared pack to sync, `@scope/name`. Default: every
    /// pack that `/.pakf/config.toml` declares.
    #[arg(value_name = "NAME")]
    pub name: Option<String>,

    /// Keep each existing file that a file of a pack lands on, and record
    /// it as your edit of the pack's version. Delete a file first to take
    /// the pack's version of it.
    #[arg(long)]
    pub keep_existing: bool,

    /// Print one JSON object of the report to stdout instead of the text
    /// lines. `--json-schema` prints its schema.
    #[arg(long)]
    pub json: bool,

    /// Print the JSON Schema of the `--json` output, and exit.
    #[arg(long, exclusive = true)]
    pub json_schema: bool,
}

/// `PackAddArgs` holds the arguments of `pakf add`.
#[derive(Debug, Args)]
pub struct PackAddArgs {
    /// The pack's source: a git URL or path. A relative path resolves from
    /// the current directory, and the entry records it from the repository
    /// root.
    #[arg(value_name = "SOURCE", required = true)]
    pub source: Option<String>,

    /// The branch, tag, or commit of a git source. Default: the source's
    /// default branch.
    #[arg(long = "ref", value_name = "REF")]
    pub git_ref: Option<String>,

    /// The pack's directory in a git source. Default: the repository's root.
    #[arg(long, value_name = "DIR")]
    pub path: Option<String>,

    /// Keep each existing file that a file of the pack lands on, and record
    /// it as your edit of the pack's version. Delete a file first to take
    /// the pack's version of it.
    #[arg(long)]
    pub keep_existing: bool,

    /// Print one JSON object of the report to stdout instead of the text
    /// lines. `--json-schema` prints its schema.
    #[arg(long)]
    pub json: bool,

    /// Print the JSON Schema of the `--json` output, and exit.
    #[arg(long, exclusive = true)]
    pub json_schema: bool,
}

/// `PackStatusArgs` holds the arguments of `pakf status`.
#[derive(Debug, Args)]
pub struct PackStatusArgs {
    /// Exit with code 1 when a file of a pack is conflicted or deleted, or
    /// edited and not editable, and its acceptance isn't `accepted`, so that
    /// a check fails on an edit that nobody has reviewed.
    #[arg(long)]
    pub check: bool,

    /// Print one JSON array of the packs to stdout instead of the text lines.
    /// `--json-schema` prints its schema.
    #[arg(long)]
    pub json: bool,

    /// Print the JSON Schema of the `--json` output, and exit.
    #[arg(long, exclusive = true)]
    pub json_schema: bool,

    /// Apply a JMESPath expression to the JSON array. Requires --json.
    #[arg(long, value_name = "EXPR", requires = "json")]
    pub query: Option<String>,
}

/// `PackLsArgs` holds the arguments of `pakf ls`.
#[derive(Debug, Args)]
pub struct PackLsArgs {
    /// The files and directories to list, as filesystem paths or paths from
    /// the repository root. A file of a pack that you deleted counts as a
    /// file. Default: the current directory.
    #[arg(value_name = "PATH")]
    pub paths: Vec<String>,

    /// List the entries of each directory below too.
    #[arg(short = 'R', long)]
    pub recursive: bool,

    /// List each entry whose name begins with `.` too. Default: list only
    /// those that are files of packs or hold one.
    #[arg(short = 'a', long)]
    pub all: bool,

    /// List each entry that git ignores too. Default: list only those that
    /// are files of packs or hold one.
    #[arg(long)]
    pub ignored: bool,

    /// Print the state, the acceptance, the pack, and the kind of each entry
    /// in columns, the note of a file's acceptance, and the number of a
    /// directory's files of packs in each state and with each acceptance.
    #[arg(short = 'l', long)]
    pub long: bool,

    /// When to colour each entry by its state or its acceptance: `auto`
    /// (only on a terminal, while `NO_COLOR` is unset or empty), `always`,
    /// or `never`.
    #[arg(long, value_enum, value_name = "WHEN", default_value_t)]
    pub color: ColorArg,

    /// List only the files of the pack NAME, its id `@scope/name`, and the
    /// directories that hold one.
    #[arg(long, value_name = "NAME")]
    pub pack: Option<String>,

    /// List only the files in the state STATE, whatever their acceptance,
    /// and the directories that hold one.
    #[arg(long, value_enum, value_name = "STATE")]
    pub state: Option<StateArg>,

    /// List only the files with the acceptance ACCEPTANCE, and the
    /// directories that hold one, or, for `unaccepted`, the files whose
    /// acceptance is `stale` or absent, so that with `--state edited` it
    /// lists the edits that nobody has accepted, the edits of editable files
    /// among them, which `pakf status --check` passes.
    #[arg(long, value_enum, value_name = "ACCEPTANCE")]
    pub acceptance: Option<AcceptanceArg>,

    /// Print one JSON array of the entries to stdout instead of the text
    /// lines. `--json-schema` prints its schema.
    #[arg(long)]
    pub json: bool,

    /// Print the JSON Schema of the `--json` output, and exit.
    #[arg(long, exclusive = true)]
    pub json_schema: bool,

    /// Apply a JMESPath expression to the JSON array. Requires --json.
    #[arg(long, value_name = "EXPR", requires = "json")]
    pub query: Option<String>,
}

/// `ColorArg` lists the values of `--color`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum ColorArg {
    // The variants have no doc comments, so that clap keeps the help of
    // `--color` on one line, as the help of every other option.
    #[default]
    Auto,
    Always,
    Never,
}

/// `StateArg` lists the values of `--state` of `pakf ls`. It holds no
/// `accepted`, which is an acceptance, so clap refuses `--state accepted`
/// as a value that it doesn't know, and the tool exits with code 2.
// sokf-implements: CLI-5_AC-36
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum StateArg {
    // The variants have no doc comments, so that clap keeps the help of
    // `--state` on one line, as the help of every other option.
    Unchanged,
    Edited,
    Conflicted,
    Deleted,
}

/// `AcceptanceArg` lists the values of `--acceptance` of `pakf ls`: an
/// acceptance, or `unaccepted` for the files whose acceptance is `stale` or
/// none.
// sokf-implements: CLI-5_AC-35
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AcceptanceArg {
    // The variants have no doc comments, so that clap keeps the help of
    // `--acceptance` on one line, as the help of every other option.
    Accepted,
    Stale,
    None,
    Unaccepted,
}

/// `PackAcceptArgs` holds the arguments of `pakf accept`.
#[derive(Debug, Args)]
pub struct PackAcceptArgs {
    /// The edited or deleted files of packs to accept.
    #[arg(value_name = "PATH", required = true)]
    pub paths: Vec<String>,

    /// Why the project keeps the edit. Required, unless `--withdraw` is
    /// given; `pakf status` prints it.
    #[arg(long, value_name = "TEXT", conflicts_with = "withdraw")]
    pub note: Option<String>,

    /// Withdraw the acceptance of each file instead.
    #[arg(long)]
    pub withdraw: bool,

    /// Print one JSON object of the report to stdout instead of the text
    /// lines. `--json-schema` prints its schema.
    #[arg(long)]
    pub json: bool,

    /// Print the JSON Schema of the `--json` output, and exit.
    #[arg(long, exclusive = true)]
    pub json_schema: bool,
}

/// `PackDiffArgs` holds the arguments of `pakf diff`. It has no `--new`,
/// which `pakf patch` replaced, so clap refuses `--new` as an option that
/// it doesn't know, and the tool exits with code 2.
// sokf-implements: CLI-8_AC-12
#[derive(Debug, Args)]
pub struct PackDiffArgs {
    /// The pack's id, `@scope/name`, as `pakf status` prints it. Default:
    /// every pack in the lock, in the order of the packs.
    #[arg(value_name = "NAME")]
    pub name: Option<String>,

    /// The files and directories of the pack NAME whose edits to print, as
    /// filesystem paths or paths from the repository root. Default: every
    /// file of the pack.
    #[arg(value_name = "PATH")]
    pub paths: Vec<String>,

    /// Also print each edit that the project accepted.
    #[arg(long)]
    pub accepted: bool,
}

/// `PackPatchArgs` holds the arguments of `pakf patch`. `-o` and `--apply`
/// exclude each other, and `-r` requires a path, so clap refuses both and
/// `-r` alone, and the tool exits with code 2.
// sokf-implements: CLI-9_AC-14
// sokf-implements: CLI-9_AC-16
#[derive(Debug, Args)]
pub struct PackPatchArgs {
    /// The pack's id, `@scope/name`, as `pakf status` prints it.
    #[arg(value_name = "NAME")]
    pub name: String,

    /// The files of the pack whose edits to take, and the files that the
    /// pack doesn't hold yet to add to it at their places in the pack, as
    /// filesystem paths or paths from the repository root. Default: every
    /// edit of the pack, apart from the edit of an editable file, which a
    /// PATH has to name.
    #[arg(value_name = "PATH")]
    pub paths: Vec<String>,

    /// Take each file below each directory of PATH that git doesn't ignore.
    /// It requires PATH.
    #[arg(short = 'r', long, requires = "paths")]
    pub recursive: bool,

    /// Write the patch to FILE instead of stdout.
    #[arg(short = 'o', long, value_name = "FILE", conflicts_with = "apply")]
    pub output: Option<PathBuf>,

    /// Apply the patch to the working tree of the pack's directory, where
    /// the pack's source is a local directory, and print each file that it
    /// applies.
    #[arg(long)]
    pub apply: bool,

    /// Also take each edit that the project accepted.
    #[arg(long)]
    pub accepted: bool,
}

/// `PackRemoveArgs` holds the arguments of `pakf remove`.
#[derive(Debug, Args)]
pub struct PackRemoveArgs {
    /// The pack's id, `@scope/name`, as `pakf status` prints it.
    #[arg(value_name = "NAME", required = true)]
    pub name: Option<String>,

    /// Keep the part `PART` of the pack as the project's own: `files`, every
    /// file that the pack placed, `instructions`, its instructions as the
    /// project's own text beside pakf's block, or `all`. You can repeat the
    /// option or list parts with commas.
    #[arg(long, value_enum, value_name = "PART", value_delimiter = ',', action = ArgAction::Append)]
    pub keep: Vec<KeepArg>,

    /// Print one JSON object of the report to stdout instead of the text
    /// lines. `--json-schema` prints its schema.
    #[arg(long)]
    pub json: bool,

    /// Print the JSON Schema of the `--json` output, and exit.
    #[arg(long, exclusive = true)]
    pub json_schema: bool,
}

/// `KeepArg` lists the values of `--keep` of `pakf remove`. clap refuses any
/// other value, and the tool exits with code 2.
// sokf-implements: CLI-7_AC-18
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum KeepArg {
    // The variants have no doc comments, so that clap keeps the help of
    // `--keep` on one line, as the help of every other option.
    Files,
    Instructions,
    All,
}
// sokf:end commands
