# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While the version is below 1.0, breaking changes may land in minor releases.

## [Unreleased]

## [0.1.0] - 2026-10-19

First release.

### Added

- Terminal interface listing every git checkout one level below a directory,
  with branch, distance from the upstream, distance from the remote default
  branch, and working tree state.
- Push from the list: single repository (`p`), every visible one (`A`), or every
  marked one (`P`). Bulk pushes ask for confirmation and name their targets.
- Push fetches the repository first, refuses when the branch is behind
  `origin/<branch>`, and uses an explicit refspec so a wrong upstream cannot
  send commits to the wrong remote branch. A branch missing on the remote is
  created with its upstream set.
- Background fetch on start; rows are re-read as each fetch lands.
- Open GitUI in the selected repository (`g`), with full terminal handover.
- Filter by branch or repository name (`/`), mark rows (`Space`), toggle between
  "needs attention" and all repositories (`a`).
- Plain table output (`-T`), used automatically when stdout is not a terminal.
- Man page and shell completions for bash, zsh and fish.

[Unreleased]: https://github.com/pie-ai/unpushed/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/pie-ai/unpushed/releases/tag/v0.1.0
