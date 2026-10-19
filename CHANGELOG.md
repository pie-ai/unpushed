# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While the version is below 1.0, breaking changes may land in minor releases.

## [Unreleased]

### Added

- `→` opens a tree of a repository's unpushed commits beside the list: commit,
  date, summary, and the changed files underneath. Commits touching eight files
  or fewer come open, larger ones stay shut and report how many they hold.
  `Space` folds a commit, `←` walks back out. Push, fetch and gitui keep working
  while the tree is open and act on the repository it belongs to.
- `→` on a file shows its diff. Three readings sit on top of each other, in
  order of how quickly they need to register: the whole line is green or red for
  added or removed; the words that actually changed within it are brightened and
  emboldened, so a one-word edit in a long line is not a hunt; and syntax
  colouring is applied to context lines, where the first two do not apply.
- `Enter` lists everything that can be done to a repository, each with the key
  that does it. Pressing the key runs the action and closes the list — a way to
  work before knowing the keys, and to learn them while doing so.

### Changed

- `Enter` no longer opens GitUI; `g` still does. Enter now opens the action
  list, which is where `g` is written down.

## [0.2.1] - 2026-10-19

### Added

- Published to crates.io, so `cargo install unpushed` works. The library target
  comes along with it but carries no stability promise — it exists to serve the
  binary and its tests, and says so.

### Fixed

- The apt repository now indexes every architecture found in its pool instead
  of a hard-coded pair. The `armhf` package shipped as a release asset in 0.2.0
  but never reached the indices, so `apt install` on a 32-bit Raspberry Pi found
  nothing. The publish step also fails now if an architecture has packages but
  no index entries.

## [0.2.0] - 2026-10-19

### Added

- Builds for `armv7-unknown-linux-musleabihf`, published as a tarball and as an
  `armhf` Debian package. Covers 32-bit Raspberry Pi OS from the Pi 2 onwards.
- `Enter` opens GitUI in the selected repository, matching the key used to drill
  into a repository in GitUI itself. `g` continues to work.
- A row that is being read, fetched or pushed shows a spinner, says what it is
  doing, and is dimmed. Pushing takes seconds, and until now the only sign of it
  was a status character that did not move.
- The list refreshes itself every few seconds, so changes made in another
  window appear without pressing a key. Background refreshes are silent: no
  spinner, no lock, busy rows skipped. `--refresh SECONDS` tunes the interval,
  `0` turns it off.

### Fixed

- `r` and `R` now re-read every repository, not only the visible ones. The
  default view hides repositories with nothing to do, so one that had since
  become dirty could never reappear. Pushing stays scoped to the visible rows.
- The cursor stays on the repository you selected when a refresh reorders the
  list around it, instead of sliding onto a different one.
- A repository without an `origin` remote no longer reports a fetch error. Its
  row showed `fetch: and the repository exists.` — the tail of git's four-line
  complaint, read as a sentence on its own. Error text now keeps git's first,
  informative line.
- A row no longer looks idle while a fetch is still running. Startup schedules a
  scan *and* a fetch for every repository; the scan finishes first, and a single
  activity field let it clear the fetch state too. Each kind of work is now
  tracked separately, and the row reports the most consequential one.

### Changed

- Every key that touches a repository is refused while work is already running
  on that row, not just a second push. A fetch during a push would move the refs
  out from under it. Bulk actions skip busy rows and report how many.
- The push confirmation accepts only `y`. `Enter` no longer confirms it: now
  that `Enter` is the main action key in the list, accepting it here would turn
  a habitual keypress into consent for a bulk push. `j` is gone too — it means
  "down" everywhere else.

## [0.1.0] - 2026-10-19

First release.

### Added

- Shell completions for bash, zsh and fish, and a man page, both generated from
  the command line definition.
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

[Unreleased]: https://github.com/pie-ai/unpushed/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/pie-ai/unpushed/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/pie-ai/unpushed/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/pie-ai/unpushed/releases/tag/v0.1.0
