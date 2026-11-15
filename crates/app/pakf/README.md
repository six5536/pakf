# pakf

`pakf` shares project files through packs. A pack is a set of files that many
projects share, kept in a git repository or a folder. `pakf sync` copies each
pack that a project declares into the project, records the base of each file
in a lock, and merges each later change of the pack into the project's own
edits. `pakf patch` sends the project's edits back to the pack.

## Install

```sh
cargo install pakf
```

pakf also ships through npm as a prebuilt binary for Linux, macOS, and
Windows:

```sh
npm install -g @six5536/pakf
```

pakf runs the `git` of your machine to fetch each pack.

## Usage

```sh
pakf add https://github.com/OWNER/REPOSITORY.git --ref main --path packs/lint
pakf sync
pakf status --check
```

The [pakf repository](https://github.com/six5536/pakf#readme) describes each
command, the manifest of a pack, and the files that pakf keeps in `/.pakf/`.

## Licence

The [Elastic License 2.0](https://github.com/six5536/pakf/blob/main/LICENSE)
covers `pakf`.
