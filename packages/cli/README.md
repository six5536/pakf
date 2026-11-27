# @six5536/pakf

`pakf` shares project files through packs. A pack is a set of files that many
projects share, kept in a git repository or a folder. `pakf sync` copies each
pack that a project declares into the project, records the base of each file
in a lock, and merges each later change of the pack into the project's own
edits. `pakf patch` sends the project's edits back to the pack.

## Status

**Unstable.** `pakf` is at a 0.x version, and its commands, options and file formats can change in any release.

## Install

```sh
npm install -g @six5536/pakf
```

This package only starts a prebuilt binary. It declares a prebuilt binary for
each supported platform as an `optionalDependency`, and npm installs only the
binary that matches your machine. When you run `pakf`, the JavaScript launcher
starts that binary.

The npm package supports Linux and macOS (`x64` and `arm64`) and Windows
(`x64`). On any other platform, or to build from source, install with the Rust
toolchain instead:

```sh
cargo install pakf
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
