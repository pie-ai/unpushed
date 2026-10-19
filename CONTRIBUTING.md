# Contributing

Thanks for taking a look. This is a small, focused tool — the quickest way to
get a change merged is to keep it in that spirit.

## Scope

`unpushed` answers one question: *what have I not pushed, across all these
checkouts?* Features that serve that question are welcome. Features better
served by opening GitUI on a single repository (`g`) generally are not.

## Before opening a pull request

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

All three run in CI and must pass.

## Conventions

- **English** for code, comments, documentation and commit messages.
- **Tests for behaviour, not for coverage.** The existing tests build real git
  repositories in a temporary directory — including a local bare remote, so
  pushing is exercised without a network. Follow that pattern rather than
  mocking git.
- **Keep the two distinctions intact.** A repository with no upstream is not the
  same as one with nothing to push, and a failed scan is not a clean repository.
  Several tests exist solely to pin this down; if a change makes them awkward,
  the change is probably wrong.
- `src/cli.rs` is the single source for the parser, help text, man page and
  completions. Add options there, not in `main.rs`.
- Update `CHANGELOG.md` under `## [Unreleased]`.

## Reporting a problem

Include the output of `unpushed --version`, your OS, and — if it is about what
the table shows — the relevant line from `unpushed -T <dir>`. Please redact
repository names if they are not yours to share.
