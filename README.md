# pakf

pakf shares project files through packs. A pack is a set of files that many projects share, such as the configuration of a linter, the scripts of a release, or the rules for a coding agent, kept in a git repository or a folder. `pakf sync` copies each pack that a project declares into the project, records the base of each file in a lock, and merges each later change of the pack into the project's own edits with `git merge-file`. `pakf patch` sends the project's edits back to the pack.

This README helps you install pakf, take a pack into a project, and write a pack:

- [Install](#install)
- [Take a pack](#take-a-pack)
- [Commands](#commands)
- [Write a pack](#write-a-pack)
- [Files of a project](#files-of-a-project)
- [Instructions for coding agents](#instructions-for-coding-agents)
- [Licence](#licence)

## Install

pakf ships through npm as a prebuilt binary for Linux and macOS (`x64` and `arm64`) and for Windows (`x64`):

```sh
npm install -g @six5536/pakf
```

On any other platform, or to build from source, install it with the Rust toolchain:

```sh
cargo install pakf
```

pakf runs the `git` of your machine to fetch each pack, so `git` has to be on the `PATH`.

## Take a pack

In a git repository, declare a pack by its source. This command takes the pack in the folder `packs/lint` of a git repository, at its branch `main`:

```sh
pakf add https://github.com/OWNER/REPOSITORY.git --ref main --path packs/lint
```

`pakf add` appends the pack's entry to `/.pakf/config.toml` and syncs the pack: it copies each file of the pack to its path in the project, and records the pack and the base of each file in `/.pakf/lock.toml`. Commit `/.pakf/` with the pack's files.

Edit a file of a pack as any other file of the project. To take the pack's later changes, run:

```sh
pakf sync
```

`pakf sync` merges each change of the pack into your edit of the file. When the two conflict, it leaves git's conflict markers in the file and exits with code 1, and you resolve the conflict as you would in a merge. A file of a pack that you deleted stays deleted.

A check of the project, such as a commit hook or a CI job, runs `pakf status --check`, which fails while a file of a pack holds an edit that nobody has accepted. Run `pakf diff` to see your edits, `pakf accept PATH --note TEXT` to keep an edit on purpose, or `pakf patch NAME` to send it to the pack.

## Commands

```sh
pakf add SOURCE [--ref REF] [--path DIR]   # declare a pack at the end of /.pakf/config.toml, and sync it
pakf sync [NAME]                           # sync every declared pack, or one, and remove each pack that no entry declares
pakf status [--check]                      # list each pack and each of its files that is edited, conflicted, or deleted
pakf ls [PATH...]                          # list the entries of each folder, with the pack and the state of each file of a pack
pakf diff [NAME] [PATH...]                 # print your edits of the files of packs as a unified diff against their bases
pakf accept PATH... --note TEXT            # record that the project keeps an edit or a deletion on purpose; --withdraw undoes it
pakf patch NAME [PATH...]                  # print the patch that sends your edits, and new files, back to the pack's repository
pakf remove NAME [--keep PART]             # delete the pack's entry and each of its files that you haven't edited
```

The commands use these placeholders, and a placeholder in square brackets is optional:

- `SOURCE`: a git URL or the path of a pack's repository or folder. A relative path resolves from the current folder.
- `REF`: a branch, a tag, or a commit of a git source.
- `DIR`: the pack's folder in a git source.
- `NAME`: the id of a pack, `@scope/name`, as `pakf status` prints it.
- `PATH`: a file or a folder of the project.
- `TEXT`: the reason why the project keeps the edit.
- `PART`: the part of the pack that the project keeps as its own: `files`, `instructions`, or `all`.

Every command takes `--repo-root DIR`, which sets the repository root. `add`, `sync`, `status`, `ls`, `accept`, and `remove` take `--json`, which prints the report as JSON, and `--json-schema`, which prints the JSON Schema of that report. `pakf help COMMAND` prints the options of each command. pakf exits with code `0` when it finds no error, `1` when it finds errors, such as a conflict or an edit that nobody has accepted, and `2` on a usage error or an internal error.

## Write a pack

A pack is a folder with a manifest, `pakf.yaml`, at its root:

```text
packs/lint/
├── pakf.yaml           the manifest
├── instructions.md     the pack's rules for coding agents, which is optional
└── files/              the files that land in a project, each at its path from the repository root
    ├── .oxlintrc.json
    └── .github/workflows/lint.yml
```

The manifest gives the pack's id, its version, the oldest version of pakf that syncs it, its description, and the packs that it depends on:

```yaml
name: "@acme/lint"
version: 1.2.0
requires-pakf: 0.1.0
description: "The house's lint configuration and its workflow."
depends:
  "@acme/base": 1.0.0
```

Quote each id, because YAML reads an unquoted `@` as a reserved character. pakf refuses a pack until the project declares each pack that it depends on, and syncs a pack after those packs. When two packs deliver a file at one path, the pack that comes later in the order owns the file, and `pakf status` lists the other pack's file as shadowed.

## Files of a project

pakf keeps its own files in `/.pakf/`:

| File                     | What it holds                                                                                                                                                                  |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `config.toml`            | The packs of the project, in order, each in a `[[pack]]` entry with its `name`, its `source`, and an optional `ref`, `path`, and `paths`, which remaps the paths of its files. |
| `lock.toml`              | What each sync fetched: the commit and the version of each pack, and the path, the hash of the base, and the acceptance of each of its files. pakf writes it.                  |
| `packs/@scope/name/`     | The base copy of each file of the pack in `base/`, from which `pakf sync` merges, and the copy of the pack's `instructions.md`. pakf writes it.                                |
| `ignore` and `ignore.d/` | The new files that `pakf patch` leaves out of a patch, in the syntax of `.gitignore`: `ignore` is the project's list, and a pack ships its list in `ignore.d/@scope/name/`.    |

This `[[pack]]` entry takes a pack from a git repository at a tag, and lands the pack's files of `/knowledge/` in `/docs/` instead:

```toml
[[pack]]
name = "@acme/method"
source = "https://github.com/OWNER/REPOSITORY.git"
ref = "v2.0.0"
path = "packs/method"
paths = { "/knowledge/" = "/docs/" }
```

Commit `/.pakf/` with the files of the packs, because the lock and the base copies let the next sync tell your edits from the pack's changes.

## Instructions for coding agents

When a pack holds an `instructions.md`, pakf copies it to `/.pakf/packs/@scope/name/instructions.md`. After each `pakf add`, `pakf sync`, and `pakf remove`, pakf hands a block with the instructions of every pack, in the order of the packs, to [agent-harness-adapter](https://github.com/six5536/agent-harness-adapter) 0.4.0, which writes it into the instructions file of each coding agent of the project, such as `AGENTS.md` or `CLAUDE.md`, after the block of every other tool. agent-harness-adapter decides which coding agents and which files the block reaches, and reads its settings for pakf from the `[harness.<NAME>]` tables of `/.pakf/config.toml`, which pakf itself ignores. pakf reads and writes no file of a coding agent itself.

`pakf remove NAME --keep instructions` keeps the pack's instructions as the project's own text, beside pakf's block, through agent-harness-adapter.

## Licence

The [Elastic License 2.0](LICENSE) covers pakf.
