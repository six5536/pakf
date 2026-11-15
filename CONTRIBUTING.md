# Contributing to pakf

This repository is a read-only mirror of pakf. Each release reaches it as one commit, which an export from the project's own repository creates, and nobody commits to it directly. This guide explains how to report a bug or ask for a feature, and how to build pakf from this repository.

- [Report a bug or ask for a feature](#report-a-bug-or-ask-for-a-feature)
- [Build pakf](#build-pakf)

## Report a bug or ask for a feature

Open an issue. A pull request can't be merged here, because the next release replaces the whole tree. When you have a change in mind, describe it in an issue, and include a patch there if you have one.

## Build pakf

The project pins its toolchains in `rust-toolchain.toml` and in the files of `.config/mise/conf.d/`, which [mise](https://mise.jdx.dev/) installs:

```sh
mise install
cargo build --locked -p pakf
```

To build the npm packages as the release does, install the JavaScript workspace and run the build task of a platform, such as Linux on x64:

```sh
npm install
mise run build:release:ubuntu:linux-x64
```

`mise tasks ls --hidden` lists a task for each of the five platforms, and the middle part of each name says which operating system it runs on. Each task builds the binary, puts it in its platform package, and smoke-tests it when it can run on the machine.

`.github/workflows/publish.yml` is the release: with `build.yml`, it builds the five binaries, smoke-tests them, publishes the crates and the packages, and creates the GitHub release with the notes that the tag's message carries. The GitHub releases hold the notes of every release: they are the project's changelog.
