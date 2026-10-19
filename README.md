# unpushed

Overview of unpushed commits across many git checkouts.

GitUI and friends work on one repository at a time. `unpushed` sits a level
above: point it at the directory that holds your checkouts and it shows, for all
of them at once, what is not pushed yet and what is dirty — and lets you push
from the list.

```
2 need work · 44 clean · 0 failed   (47 repos, 2 visible)
/home/you/src

      Repo          Branch         Upstream     Main
   ↑  backend-api   feature/login  no upstream  ↑3
   ⚠  shared-lib    main           —            on default  ⚠ 2 modified

j/k select · Space mark · p push · A visible · P marked · g gitui · f/F fetch · …
```

Only **one level** is searched. Point it at the directory that *holds* the
checkouts, not at the root of a tree.

## Install

**Homebrew** (macOS):

```bash
brew tap pie-ai/unpushed
brew install unpushed
```

**Debian / Ubuntu**:

```bash
curl -fsSL https://pie-ai.github.io/unpushed/key.gpg \
  | sudo tee /etc/apt/keyrings/unpushed.asc > /dev/null
echo "deb [signed-by=/etc/apt/keyrings/unpushed.asc] https://pie-ai.github.io/unpushed stable main" \
  | sudo tee /etc/apt/sources.list.d/unpushed.list
sudo apt update && sudo apt install unpushed
```

**Prebuilt binaries** for Linux (x86_64, aarch64, armv7) and macOS (Apple
silicon, Intel) are attached to every
[release](https://github.com/pie-ai/unpushed/releases). They are statically
linked and have no runtime dependencies.

On a Raspberry Pi, apt picks `arm64` on a 64-bit OS and `armhf` on a 32-bit one.
The `armhf` build targets ARMv7, so it covers the Pi 2 and later; the Pi 1 and
Zero are ARMv6 and are not supported.

**Cargo** — needs Rust 1.88 or newer:

```bash
cargo install unpushed
```

Or straight from the repository, for an unreleased state:

```bash
cargo install --git https://github.com/pie-ai/unpushed
```

## Usage

```bash
unpushed                  # current directory
unpushed ~/src/checkouts  # explicit directory
unpushed -T               # plain table instead of the interface
```

| Flag | Meaning |
|------|---------|
| `-T`, `--table` | print a table instead of the interface |
| `-a`, `--all` | also show repositories with nothing to do (table only) |
| `-j`, `--threads N` | parallel jobs (default 8) |
| `-R`, `--refresh SECONDS` | background refresh interval, `0` turns it off (default 5) |
| `-t`, `--timing` | print discovery and scan duration (table only) |
| `-V`, `--version` | print version |
| `-h`, `--help` | usage |

Without a terminal on stdout the table is printed automatically, so the command
works in a pipe. Its exit status is non-zero when a repository could not be
scanned.

### Keys

| Key | Action |
|-----|--------|
| `j`/`k`, `↑`/`↓` | move the selection |
| `Enter` (or `g`) | open GitUI in the repository |
| `Space` | mark a row |
| `p` | push the selected repository |
| `A` | push every **visible** repository (asks first) |
| `P` | push every **marked** repository (asks first) |
| `f` / `F` | fetch (selected / all visible) |
| `r` / `R` | re-read (selected / all visible) |
| `/` | filter by branch or name |
| `a` | show all repositories, not just those needing attention |
| `q` | quit |

`Enter` requires [gitui](https://github.com/gitui-org/gitui) on your `PATH`;
without it the row simply reports that it is missing. Quitting gitui returns you
to the list with the same row selected, ready for `p`.

While a repository is being fetched, read or pushed, its row shows a spinner and
says what it is doing, and every key that would touch that repository is refused
until it settles — a fetch in the middle of a push would move the refs out from
under it. Moving the cursor and marking rows stay available, as do all other
repositories.

## What it does on start, and while it runs

The local scan finishes in well under a second — 47 checkouts take around 200 ms
across eight threads once the filesystem cache is warm. A fetch for every
repository then runs in the background and each row is re-read as its fetch
lands, so the numbers update underneath you rather than being stale until you
ask.

From then on the list keeps itself current: every few seconds every repository
is read again, so a file you change in another window shows up on its own. This
costs nothing noticeable and is deliberately invisible — background refreshes
show no spinner and lock nothing, because a row must never be busy at the moment
you reach for a key. Repositories that are already working are skipped, and the
cursor stays on the repository you had selected even when the list reorders
around it. `--refresh 0` turns it off.

`r` and `R` re-read **every** repository, including those the current view
hides. Pushing (`A`) stays scoped to what you can see: read everything, write
only what is in front of you.

## Pushing

Push is deliberate about three things:

- **It fetches the repository first.** The behind-check is only meaningful
  against fresh refs, and the background fetch may not have reached this
  repository yet.
- **It refuses when the branch is behind `origin/<branch>`** and says so, rather
  than forcing. Being behind the *default* branch does not block anything — that
  is normal for a topic branch, and blocking on it would defeat the purpose.
- **It uses an explicit refspec** (`refs/heads/B:refs/heads/B`), so a missing or
  wrong upstream cannot send commits to the wrong remote branch. A branch that
  does not exist on the remote is created, with the upstream set.

Interactive prompts are suppressed (`GIT_TERMINAL_PROMPT=0`,
`ssh -o BatchMode=yes`): several pushes run at once, and a background job cannot
answer a passphrase prompt. Authentication failures are reported on the row.

## Reading the table

Markers: `↑` unpushed · `↓` behind · `⇅` both · `⚠` only local changes ·
`·` nothing to do · `!` scan failed.

Two distinctions the output keeps apart, because conflating them is the mistake
this tool exists to avoid:

- **`no upstream` vs. `—`** — a branch with no upstream was never pushed; a `—`
  under *Upstream* means it is in sync with one.
- **Unpushed vs. ahead of the default branch** — "unpushed" is measured against
  the upstream when there is one. A branch can be far ahead of `origin/main` and
  still be fully pushed.

A repository whose scan failed is never counted as clean and never hidden by the
default view.

## Development

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

The scan and push tests build real repositories with `git` in a temporary
directory, including a local bare remote, so pushing is exercised for real
without a network. `tests/render.rs` draws the widgets into ratatui's test
backend and asserts on the resulting text.

`src/cli.rs` is the single source for the parser, the help text, the man page
and the shell completions; `build.rs` generates the latter two into `man/` and
`completions/`.

Reading uses `git2` with default features off and libgit2 linked statically;
fetch and push run as `git` subprocesses, where the SSH agent and credential
helpers behave exactly as they do in your shell.

## License

MIT or Apache-2.0, at your option. See [LICENSE-MIT](LICENSE-MIT),
[LICENSE-APACHE](LICENSE-APACHE) and
[THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md).
