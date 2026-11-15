# pakf-core

`pakf-core` is the library of the
[`pakf`](https://crates.io/crates/pakf) command, which shares project files
through packs. It reads a project's packs and its lock, fetches each pack
through the `git` of the machine, merges each change of a pack into the
project's edits, and runs each command of `pakf`.

The library serves the `pakf` command, and its interface can change in any
release before 1.0. To share files through packs, install the command:

```sh
cargo install pakf
```

The [pakf repository](https://github.com/six5536/pakf#readme) describes the
command and the files that it reads and writes.

## Licence

The [Elastic License 2.0](https://github.com/six5536/pakf/blob/main/LICENSE)
covers `pakf-core`.
