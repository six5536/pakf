# pakf

pakf shares project files through packs. A pack is a set of files that many projects share, such as the configuration of a linter, the scripts of a release, or the rules for a coding agent, kept in a git repository or a folder. `pakf sync` copies each pack that a project declares into the project, records the base of each file in a lock, and merges each later change of the pack into the project's own edits with `git merge-file`. `pakf patch` sends the project's edits back to the pack.

This README helps you install pakf, take a pack into a project, and write a pack:

- [Status](#status)
- [Install](#install)
- [Take a pack](#take-a-pack)
- [Commands](#commands)
- [Write a pack](#write-a-pack)
- [Files of a project](#files-of-a-project)
- [Instructions for coding agents](#instructions-for-coding-agents)
- [Licence](#licence)

## Status

**Unstable.** pakf is at a 0.x version, and its commands, options and file formats can change in any release.

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

A check of the project, such as a commit hook or a CI job, runs `pakf status --check`, which fails while a file of a pack holds an edit that nobody has accepted, other than the edit of an editable file. Run `pakf diff` to see your edits, `pakf accept PATH --note TEXT` to keep an edit on purpose, or `pakf patch NAME` to send it to the pack.

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

The manifest gives the pack's id, its version, the oldest version of pakf that syncs it, its description, the packs that it depends on, and its editable files:

```yaml
name: "@acme/lint"
version: 1.2.0
requires-pakf: 0.2.0
description: "The house's lint configuration and its workflow."
depends:
  "@acme/base": 1.0.0
editable:
  - /.config/lint.toml
```

Quote each id, because YAML reads an unquoted `@` as a reserved character. pakf refuses a pack until the project declares each pack that it depends on, and syncs a pack after those packs.

`editable` lists, by their exact paths, the files of `files/` that every project fills in with its own content, such as a workspace `Cargo.toml` or a `LICENSE`. pakf still merges each change of the pack into such a file, and `pakf status`, `pakf ls`, and `pakf diff` show its edits, but its edit needs no acceptance: `pakf status` prints it as `edited /Cargo.toml (editable)`, `pakf status --check` passes on it unless it holds a conflict or is deleted, and `pakf patch` sends its edit only where a `PATH` names the file. A pack that declares editable files needs pakf 0.2.0. When two packs deliver a file at one path, the pack that comes later in the order owns the file, and `pakf status` lists the other pack's file as shadowed.

## Files of a project

pakf keeps its own files in `/.pakf/`:

| File                     | What it holds                                                                                                                                                                                                                                                                              |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `config.toml`            | The packs of the project, in order, each in a `[[pack]]` entry with its `name`, its `source`, and an optional `ref`, `path`, and `paths`, which remaps the paths of its files, and the rules of the comparison in `[compare]`.                                                             |
| `lock.toml`              | What each sync fetched: the commit and the version of each pack, and the path, the hash of the base, whether it is editable, whether the last sync left a conflict in it, and the acceptance of each of its files, with the fingerprint of the accepted version's meaning. pakf writes it. |
| `harness.toml`           | The record of what agent-harness-adapter wrote of pakf's block into the file of each coding agent. agent-harness-adapter writes it.                                                                                                                                                        |
| `cache/`                 | The cache of the comparison by meaning, which pakf writes with a `.gitignore` that keeps it out of git. Delete it at any time.                                                                                                                                                             |
| `packs/@scope/name/`     | The base copy of each file of the pack in `base/`, from which `pakf sync` merges, and the copy of the pack's `instructions.md`. pakf writes it.                                                                                                                                            |
| `ignore` and `ignore.d/` | The new files that `pakf patch` leaves out of a patch, in the syntax of `.gitignore`: `ignore` is the project's list, and a pack ships its list in `ignore.d/@scope/name/`.                                                                                                                |

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

A symbolic link of the project that leads outside the repository or into `.git`, such as `docs -> ~/notes`, stops `pakf sync`, `pakf add`, `pakf remove` and `pakf accept` before they write any file below it, and no other command reads a file of a pack at its destination through it. A link that leads inside the repository counts as the directory that it leads to. A coding agent's file that is such a link, such as `AGENTS.md`, still takes pakf's block at its target.

## A formatter's layout is no edit

A project's formatter can lay out a file of a pack in its own way: wrap its lines, indent it with tabs, or sort its keys. pakf tells such a change of layout from an edit through the library [same-same](https://crates.io/crates/same-same), which compares two texts by their meaning. Where a file's bytes differ from its base, pakf asks the library whether the file means the same as its base copy, and a file that only the formatter changed is no edit: `pakf status` doesn't list it, `pakf status --check` passes on it, `pakf ls` calls it `unchanged`, and `pakf diff` and `pakf patch` leave it out. An edited file still shows its whole difference from its base.

Where the pack changed only the layout of a file, a sync gives a copy that you didn't change the pack's new layout, and writes nothing to a copy that you re-laid or edited. A sync writes nothing where your copy already means the pack's new version. Where you only re-laid a file and the pack changed it, the sync takes the pack's new version in the pack's layout, which your formatter lays out again at its next run. Where you and the pack both changed the meaning, the sync merges by lines, as before; where you also re-laid the file, a conflict block can differ in layout alone, and either side of such a block keeps the meaning. An accepted edit stays accepted after you re-lay the file.

same-same compares by meaning Markdown, with its front matter and the code of its fences, JSON and JSON with comments, TOML, YAML, JavaScript, JSX, TypeScript and TSX. It reads a difference of a few kinds as the same apart from a named thing, such as the order of keys, which pakf counts as no edit, except a value that a YAML 1.1 reader takes otherwise than YAML 1.2 does, such as a plain `no` where the base holds `"no"`, which pakf counts as an edit. A file of any other type compares by its bytes, unless a rule of `[compare]` names a formatter or a command that lays it out, or you turn on the detection of the project's formatter, oxfmt, Prettier, Biome or dprint, by its configuration file at the repository root. A file whose text doesn't parse in its format compares by its bytes too, and so does a file whose formatter or command is missing, fails, or runs longer than 10 seconds. pakf prints nothing of how it compared a file: an edited file is just `edited`. A formatter or a command that a rule names, or that detection found, and that is missing, fails or runs past its limit gets a warning, such as `warning: `cssfmt` of [[compare.rule]] number 1 is missing`. So where your formatter's configuration file sits at the repository root and `detect` is off, a file that only that formatter could compare, such as a re-laid `.css` file, reads as edited until you set `detect = true`.

What a sync writes and what the lock records are the same on every machine, whatever programs each can run: `pakf sync`, `pakf add` and `pakf remove` compare by same-same's own reading alone, with the `edits` of `[compare]`, and run no formatter or command, and `pakf accept` records the fingerprint of that reading. `pakf accept` refuses a file as unedited only where `pakf status` reads it so too, so under a rule with `override = true` it can run that rule's formatter or command. A formatter or a command of `[compare]`, and detection, serve `pakf status`, `pakf ls`, `pakf diff` and `pakf patch`, so a machine that can run them is spared the report of a file that only they lay out, and a sync reads such a file by its bytes, as pakf 0.2.0 did.

The table `[compare]` of `/.pakf/config.toml` sets the comparison:

```toml
[compare]
detect = true # detect the project's formatter; off by default

[[compare.rule]]
files = ["*.ts", "*.tsx"] # patterns in the syntax of .gitignore
edits = ["import-order", "yaml11-value"]

[[compare.rule]]
files = ["*.css"]
formatter = "prettier"
config = ".prettierrc.json"
settings = "internal-with-overrides"

[[compare.rule]]
files = ["*.sh"]
command = ["shfmt", "{file}"]
```

Each rule holds `files`, patterns in the syntax of `.gitignore` from the repository root, and `edits`, a `formatter` or a `command`, or `edits` with one of the others, and the last rule whose patterns match a file decides each of its keys for the file. `edits` lists the named things that count as an edit of the files: `key-order`, `comment-place`, `import-order`, `byte-order-mark`, `class-order`, `yaml11-value` and `region-indent`. A file that no rule with `edits` matches counts only `yaml11-value`. A rule's `edits` sets the whole list, so a rule that counts `import-order` for files that can hold YAML, such as the front matter of a Markdown file, names `yaml11-value` too to keep it, and `edits = []` counts none.

`formatter` names a formatter that same-same knows: `oxfmt`, `prettier`, `biome` or `dprint`. `config` is the path of that formatter's configuration file in the project, such as `.prettierrc.json`, and `settings` says whose settings the formatter runs with: `internal-with-overrides`, the default, runs it with same-same's own settings, which sort as much as the formatter can, and each key of your configuration file on top, so your keys win and the sorts that you leave alone stay; `as-written` runs it with your configuration file alone, and the formatter's defaults for the rest; `internal-only` runs it with same-same's settings alone and ignores your file. The formatter works on copies of the files in a temporary folder outside the project, with a settings file that same-same writes there, so it never reads any other configuration file of the project or of your machine. Your configuration file can be JSON or JSON with comments in every way; a file in another form, such as `prettier.config.js` or `.prettierrc.yaml`, works only with `as-written`. A few keys stay same-same's in every way, because the run needs them, such as Biome's `files` and `vcs` or dprint's `includes`, and each answer notes the keys of your file that were set aside. Results are kept in the cache in every way, unless your configuration names another file, such as Biome's `extends`, a Prettier plugin runs, or a dprint plugin is named by a path.

Prettier sorts nothing unless you list its plugins, such as `plugins = ["prettier-plugin-sort-json"]`, which same-same finds in the project's `node_modules`; put the plugin's options in your configuration file.

`command` is any other program, such as `command = ["stylelint", "--fix", "{files}"]`, which runs exactly as you wrote it, with `{files}`, an argument of its own, or `{file}` filled in, never both; nothing of a command is kept in the cache, and its own search for a configuration file can find one above the temporary folder or in your user configuration folder. Name a formatter that same-same knows with `formatter`, not `command`: a command knows nothing of it, and `prettier --write {files}` fails on a file that is formatted already.

A formatter or a command lays out only a file of a type that same-same doesn't know, or one whose text doesn't parse, unless its rule sets `override = true`, which puts it before same-same's own comparison. Detection is off unless `detect = true` turns it on, because running a project's formatter is the project's stated choice, and a rule with a `formatter` or a `command` is such a choice too. A formatter that detection finds runs from the project's `node_modules/.bin` or the path, and a rule's command as written, with a program named by a path, such as `tools/fmt`, read from the repository root, in `pakf status`, `pakf ls`, `pakf diff` and `pakf patch`, so these commands run programs that the repository names, as its own tasks do. Each run of a formatter or a command takes at most 10 seconds, and one that runs past it runs on no further file of the same command. pakf runs no formatter on a file that it writes.

pakf keeps the library's cache in `/.pakf/cache/same-same/`, beside a `/.pakf/cache/.gitignore` that keeps the folder out of git, so a later command answers without reading a file again. A command that can't write the folder goes on without it, and so does a command that finds a symbolic link on the way, so that it writes nothing outside the project.

## Instructions for coding agents

When a pack holds an `instructions.md`, pakf copies it to `/.pakf/packs/@scope/name/instructions.md`. After each `pakf add`, `pakf sync`, and `pakf remove`, pakf hands a block with the instructions of every pack, in the order of the packs, to [agent-harness-adapter](https://github.com/six5536/agent-harness-adapter) 0.5.0, which writes it into the instructions file of each coding agent of the project, such as `AGENTS.md` or `CLAUDE.md`, after the block of every other tool. agent-harness-adapter decides which coding agents and which files the block reaches, and reads its settings for pakf from the `[harness.<NAME>]` tables of `/.pakf/config.toml`, which pakf itself ignores. pakf reads and writes no file of a coding agent itself.

`pakf remove NAME --keep instructions` keeps the pack's instructions as the project's own text, beside pakf's block, through agent-harness-adapter.

agent-harness-adapter compares pakf's block by its meaning, as pakf compares the files of a pack, so a block that your formatter laid out in its own way stays current. A block that you edited stays as you left it: the sync reports it as `edited`, writes nothing into it, and exits with code 1. To take the packs' version again, delete the block with its markers, `<!-- pakf:harness last -->` and `<!-- /pakf:harness -->`, and run `pakf sync`.

pakf 0.3.0 records the block in a new form in `/.pakf/harness.toml`, which pakf 0.2.0 can't use. Its first sync rewrites the record where the block holds what the packs say. Where the block differs, such as after the packs changed their instructions, pakf can't tell your edit from the text that pakf 0.2.0 wrote, so it leaves the block with a warning:

```text
warning: claude: pakf's block in CLAUDE.md may not be an edit, as an older pakf recorded it
note: delete the block with its markers, then run `pakf sync`
```

## Licence

The [Elastic License 2.0](LICENSE) covers pakf.
