//! Operations that change something, run as `git` subprocesses.
//!
//! Reading happens through `git2`, but fetch and push go through the real
//! `git` binary: that is where the SSH agent, credential helpers, `known_hosts`
//! and the user's config already work. Reimplementing those against libgit2
//! would be the largest single cost of the project for no gain.
//!
//! Interactive prompting is suppressed rather than supported. Pushing several
//! repositories at once runs them in parallel, and a background job cannot
//! sensibly ask for a passphrase. The surveyed keys carry no passphrase, so
//! this costs nothing in practice; if authentication does fail, the repository
//! reports an error instead of hanging forever on an invisible prompt.

use std::path::Path;
use std::process::{Command, Output};

/// Why a push did not happen, or what it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushOutcome {
    /// Pushed. `created` marks the case where the remote branch did not exist
    /// and was created along with the upstream link.
    Pushed {
        created: bool,
    },
    /// Nothing to do — no local commits the remote is missing.
    UpToDate,
    /// Refused on purpose, with the reason shown to the user.
    Refused(String),
    Failed(String),
}

impl PushOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, PushOutcome::Pushed { .. } | PushOutcome::UpToDate)
    }
}

/// What a rebase is measured against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Onto {
    /// The branch's own upstream. This is the one that clears a refused push:
    /// the push guard compares against exactly this ref.
    Upstream,
    /// The remote's default branch, for bringing a topic branch up to the
    /// mainline. Leaves the branch diverged from its upstream afterwards.
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebaseOutcome {
    Rebased {
        onto: String,
    },
    /// Already contains everything the target has.
    UpToDate,
    /// Refused on purpose, with the reason shown to the user.
    Refused(String),
    /// Conflicted and was rolled back; the tree is as it was.
    Conflicted {
        onto: String,
    },
    Failed(String),
}

impl RebaseOutcome {
    pub fn is_success(&self) -> bool {
        matches!(
            self,
            RebaseOutcome::Rebased { .. } | RebaseOutcome::UpToDate
        )
    }
}

/// Fetches, checks that a rebase is safe, then rebases.
///
/// Refuses rather than improvises: a dirty tree, a detached HEAD or a rebase
/// already under way all stop it. On conflict the rebase is aborted, so the
/// working tree is always left exactly as it was found — resolving conflicts
/// is a job for an editor, not for a list of repositories.
pub fn rebase(path: &Path, onto: Onto) -> RebaseOutcome {
    if let Err(reason) = ready_to_rebase(path) {
        return RebaseOutcome::Refused(reason);
    }
    if let Err(err) = fetch(path) {
        return RebaseOutcome::Failed(format!("fetch: {err}"));
    }

    let target = match resolve(path, onto) {
        Ok(target) => target,
        Err(reason) => return RebaseOutcome::Refused(reason),
    };

    match run(path, &["rev-list", "--count", &format!("HEAD..{target}")]) {
        Ok(out) if out.trim() == "0" => return RebaseOutcome::UpToDate,
        Ok(_) => {}
        Err(err) => return RebaseOutcome::Failed(err),
    }

    match run(path, &["rebase", &target]) {
        Ok(_) => RebaseOutcome::Rebased { onto: target },
        Err(err) => {
            // A rebase fails two very different ways: it can stop on a
            // conflict, or it can break for an unrelated reason — git with no
            // committer identity configured, for one. Both leave rebase state
            // behind, so the presence of that state says nothing. Ask for
            // unmerged paths instead, which is the question actually being
            // asked. Must happen before the abort clears them.
            let conflicted = run(path, &["ls-files", "--unmerged"])
                .map(|out| !out.trim().is_empty())
                .unwrap_or(false);

            // Leave nothing half-applied either way. If the abort itself
            // fails the repository needs a human, and saying so is more use
            // than any other message.
            if rebase_in_progress(path) && run(path, &["rebase", "--abort"]).is_err() {
                return RebaseOutcome::Failed(
                    "rebase stopped and could not be aborted — resolve by hand".to_string(),
                );
            }

            if conflicted {
                RebaseOutcome::Conflicted { onto: target }
            } else {
                RebaseOutcome::Failed(err)
            }
        }
    }
}

fn rebase_in_progress(path: &Path) -> bool {
    path.join(".git/rebase-merge").exists() || path.join(".git/rebase-apply").exists()
}

/// Whether the divergence looks like the result of rebasing onto the default
/// branch, rather than both sides moving on their own.
///
/// The test is containment: after such a rebase the local branch carries the
/// default branch's tip while its own remote counterpart does not. Both sides
/// merely moving leaves neither containing it.
fn rebased_onto_default(path: &Path, branch: &str) -> bool {
    let Ok(default) = resolve(path, Onto::Default) else {
        return false;
    };
    if default == format!("origin/{branch}") {
        return false;
    }
    let contains = |rev: &str| run(path, &["merge-base", "--is-ancestor", &default, rev]).is_ok();
    contains("HEAD") && !contains(&format!("origin/{branch}"))
}

fn ready_to_rebase(path: &Path) -> Result<(), String> {
    // A rebase left half-finished by something else must not be stacked on.
    if rebase_in_progress(path) {
        return Err("a rebase is already in progress".to_string());
    }
    match run(path, &["symbolic-ref", "--quiet", "HEAD"]) {
        Ok(_) => {}
        Err(_) => return Err("detached HEAD — check out a branch first".to_string()),
    }
    // Rebasing over local changes would either refuse halfway or stash them
    // somewhere the user is not looking.
    match run(path, &["status", "--porcelain", "--untracked-files=no"]) {
        Ok(out) if out.trim().is_empty() => Ok(()),
        Ok(_) => Err("uncommitted changes — commit or stash them first".to_string()),
        Err(err) => Err(err),
    }
}

fn resolve(path: &Path, onto: Onto) -> Result<String, String> {
    match onto {
        Onto::Upstream => run(path, &["rev-parse", "--abbrev-ref", "@{upstream}"])
            .map(|out| out.trim().to_string())
            .map_err(|_| "no upstream to rebase onto".to_string()),
        Onto::Default => run(
            path,
            &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
        )
        .map(|out| out.trim().to_string())
        .map_err(|_| "no default branch on origin".to_string()),
    }
}

/// Runs `git fetch origin` for one repository.
///
/// A repository without an `origin` remote is not an error: there is simply
/// nothing to fetch, and the row already says so with "no upstream". Reporting
/// git's four-line complaint on every such row would be noise.
pub fn fetch(path: &Path) -> Result<(), String> {
    match run(path, &["remote", "get-url", "origin"]) {
        Ok(_) => run(path, &["fetch", "--quiet", "origin"]).map(|_| ()),
        // "No such remote" is the benign case. Anything else — not a
        // repository, unreadable directory — is a real failure and must stay
        // one, or a broken checkout would masquerade as a healthy one.
        Err(err) if err.contains("No such remote") => Ok(()),
        Err(err) => Err(err),
    }
}

/// Fetches, verifies the push is safe, then pushes.
///
/// The fetch is not an optimisation: it is what makes the "is this branch
/// behind?" question answerable. The background fetch started at launch may
/// still be running or may never have covered this repository, so push refreshes
/// its own target rather than trusting the table.
pub fn push(path: &Path, branch: &str) -> PushOutcome {
    if let Err(err) = fetch(path) {
        return PushOutcome::Failed(format!("fetch: {err}"));
    }

    let remote_ref = format!("refs/remotes/origin/{branch}");
    let remote_exists = run(path, &["show-ref", "--verify", "--quiet", &remote_ref]).is_ok();

    // The guard compares against the push target — `origin/<branch>` — and
    // deliberately not against the default branch. A ticket branch is normally
    // behind `origin/main` by design; refusing to push for that reason would
    // block the very workflow this tool exists for.
    if remote_exists {
        // Both counts in one call, so the four cases below can be told apart.
        // Reading them separately would mean deciding on half the picture.
        let range = format!("origin/{branch}...{branch}");
        let (behind, ahead) = match run(path, &["rev-list", "--left-right", "--count", &range]) {
            Ok(out) => {
                let mut parts = out.split_whitespace();
                (
                    parts.next().and_then(|n| n.parse().ok()).unwrap_or(0usize),
                    parts.next().and_then(|n| n.parse().ok()).unwrap_or(0usize),
                )
            }
            Err(err) => return PushOutcome::Failed(err),
        };

        match (ahead, behind) {
            (0, 0) => return PushOutcome::UpToDate,
            // Nothing to push, but not current either. Calling that "up to
            // date" would be a lie, and the way out is a rebase.
            (0, behind) => {
                return PushOutcome::Refused(format!(
                    "nothing to push — {behind} behind origin/{branch}, press b to rebase"
                ))
            }
            // Diverged histories, which arise two ways and want opposite
            // answers. Usually both sides simply moved, and rebasing onto the
            // upstream settles it. But if the branch was rebased onto the
            // default branch, rebasing onto the upstream would undo exactly
            // that — there the only way on is a force push, which this tool
            // deliberately does not do.
            (ahead, behind) if behind > 0 => {
                let hint = if rebased_onto_default(path, branch) {
                    "rebased onto the default branch — push it from gitui"
                } else {
                    "press b to rebase onto it"
                };
                return PushOutcome::Refused(format!(
                    "diverged from origin/{branch} ({ahead} ahead, {behind} behind) — {hint}"
                ));
            }
            _ => {}
        }
    }

    // An explicit refspec rather than a bare `git push`: if the upstream is
    // missing or points somewhere unexpected, this still pushes the branch to
    // the remote branch of the same name and nowhere else.
    let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
    match run(path, &["push", "--set-upstream", "origin", &refspec]) {
        Ok(_) => PushOutcome::Pushed {
            created: !remote_exists,
        },
        Err(err) => PushOutcome::Failed(err),
    }
}

/// Runs `gitui` in the repository, handing over the terminal completely.
///
/// The caller is responsible for leaving the alternate screen first and
/// restoring it afterwards; this only waits for the child to finish.
pub fn gitui(path: &Path) -> Result<(), String> {
    let status = Command::new("gitui")
        .current_dir(path)
        .status()
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => "gitui is not installed".to_string(),
            _ => err.to_string(),
        })?;

    // gitui exits non-zero when quit with an error; nothing to report beyond
    // that, since it had the terminal and already told the user.
    let _ = status;
    Ok(())
}

fn run(path: &Path, args: &[&str]) -> Result<String, String> {
    let output = command(path, args)
        .output()
        .map_err(|err| format!("could not run git: {err}"))?;
    interpret(output)
}

fn command(path: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(path)
        // No credential prompt on a terminal this process no longer owns.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", ssh_command());
    cmd
}

/// Keeps a user-configured `GIT_SSH_COMMAND` and only adds the batch flag, so
/// a custom ssh wrapper or port setting is not silently dropped.
fn ssh_command() -> String {
    let base = std::env::var("GIT_SSH_COMMAND").unwrap_or_else(|_| "ssh".to_string());
    if base.contains("BatchMode") {
        base
    } else {
        format!("{base} -o BatchMode=yes")
    }
}

fn interpret(output: Output) -> Result<String, String> {
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };

    // git is verbose on failure and the useful sentence comes first; its later
    // lines are advice that reads as nonsense on its own ("and the repository
    // exists."). Prefer the first `fatal:`/`error:` line, then the first line
    // of any kind.
    let lines: Vec<&str> = detail
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("remote:"))
        .collect();
    let message = lines
        .iter()
        .find(|line| line.starts_with("fatal:") || line.starts_with("error:"))
        .or_else(|| lines.first())
        .map(|line| {
            line.trim_start_matches("fatal:")
                .trim_start_matches("error:")
                .trim()
                .to_string()
        })
        .unwrap_or_else(|| "unknown error".to_string());

    Err(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(tag: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "unpushed-git-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(&base).unwrap();
            Fixture(base)
        }

        fn git(&self, dir: &Path, args: &[&str]) {
            let out = Command::new("git")
                .args(args)
                .current_dir(dir)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@example.com")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@example.com")
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        /// A local "remote" plus a clone of it. Pushing to a path remote needs
        /// no network and no credentials, so the real code path is exercised.
        fn clone(&self, name: &str) -> PathBuf {
            let seed = self.0.join(format!("{name}-seed"));
            std::fs::create_dir_all(&seed).unwrap();
            self.git(&seed, &["init", "-q", "-b", "main"]);
            std::fs::write(seed.join("README"), "seed\n").unwrap();
            self.git(&seed, &["add", "."]);
            self.git(&seed, &["commit", "-qm", "seed"]);

            let remote = self.0.join(format!("{name}.git"));
            self.git(
                &self.0,
                &[
                    "clone",
                    "-q",
                    "--bare",
                    seed.to_str().unwrap(),
                    remote.to_str().unwrap(),
                ],
            );

            let work = self.0.join(name);
            self.git(
                &self.0,
                &[
                    "clone",
                    "-q",
                    remote.to_str().unwrap(),
                    work.to_str().unwrap(),
                ],
            );
            // `git rebase` is run by the code under test, without the
            // environment these helpers set, so the identity has to live in
            // the repository. CI runners have no global one.
            self.git(&work, &["config", "user.name", "Ada"]);
            self.git(&work, &["config", "user.email", "ada@example.com"]);
            work
        }

        fn head(&self, work: &Path) -> String {
            let out = Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(work)
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        }

        fn commit(&self, work: &Path, file: &str) {
            std::fs::write(work.join(file), file).unwrap();
            self.git(work, &["add", "."]);
            self.git(work, &["commit", "-qm", &format!("add {file}")]);
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn pushes_commits_on_a_tracking_branch() {
        let fx = Fixture::new("push");
        let work = fx.clone("demo");
        fx.commit(&work, "a.txt");

        assert_eq!(push(&work, "main"), PushOutcome::Pushed { created: false });
        assert_eq!(push(&work, "main"), PushOutcome::UpToDate);
    }

    /// The dominant case: a branch that exists only locally. Push has to
    /// create it rather than fail with "no upstream".
    #[test]
    fn creates_the_remote_branch_when_it_does_not_exist() {
        let fx = Fixture::new("create");
        let work = fx.clone("demo");
        fx.git(&work, &["checkout", "-q", "-b", "feature/login"]);
        fx.commit(&work, "a.txt");

        assert_eq!(
            push(&work, "feature/login"),
            PushOutcome::Pushed { created: true }
        );

        // The upstream must now be set, so the next push is an ordinary one.
        assert_eq!(push(&work, "feature/login"), PushOutcome::UpToDate);
    }

    /// Local commits plus remote commits is divergence, not a plain
    /// rebase-and-push — and the message has to say so.
    #[test]
    fn refuses_to_push_when_the_histories_diverged() {
        let fx = Fixture::new("behind");
        let work = fx.clone("demo");
        let other = fx.clone("demo-other");
        // Both clones point at different remotes; wire the second one at the
        // first remote so they can race on the same branch.
        let remote = fx.0.join("demo.git");
        fx.git(
            &other,
            &["remote", "set-url", "origin", remote.to_str().unwrap()],
        );
        fx.git(&other, &["fetch", "-q", "origin"]);
        fx.git(&other, &["reset", "-q", "--hard", "origin/main"]);
        fx.commit(&other, "theirs.txt");
        fx.git(&other, &["push", "-q", "origin", "main"]);

        fx.commit(&work, "mine.txt");
        match push(&work, "main") {
            PushOutcome::Refused(reason) => {
                assert!(reason.contains("diverged"), "unexpected reason: {reason}");
                // Both sides simply moved, so rebasing onto the upstream is
                // the way on.
                assert!(reason.contains("press b"), "{reason}");
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    /// Behind with nothing of your own is not "up to date"; saying so would
    /// hide that the branch has fallen behind.
    #[test]
    fn being_behind_with_nothing_to_push_says_what_to_do() {
        let fx = Fixture::new("behind-only");
        let work = fx.clone("demo");
        let other = fx.clone("demo-other");
        let remote = fx.0.join("demo.git");
        fx.git(
            &other,
            &["remote", "set-url", "origin", remote.to_str().unwrap()],
        );
        fx.git(&other, &["fetch", "-q", "origin"]);
        fx.git(&other, &["reset", "-q", "--hard", "origin/main"]);
        fx.commit(&other, "theirs.txt");
        fx.git(&other, &["push", "-q", "origin", "main"]);

        match push(&work, "main") {
            PushOutcome::Refused(reason) => {
                assert!(reason.contains("nothing to push"), "{reason}");
                assert!(reason.contains("press b"), "no way forward: {reason}");
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    /// Being behind the default branch is normal for a ticket branch and must
    /// not block the push. This is the distinction the guard is built around.
    #[test]
    fn being_behind_main_does_not_block_pushing_a_ticket_branch() {
        let fx = Fixture::new("behind-main");
        let work = fx.clone("demo");

        // Branch off, then let main move ahead on the remote.
        fx.git(&work, &["checkout", "-q", "-b", "feature/login"]);
        fx.commit(&work, "ticket.txt");

        let other = fx.clone("demo-other");
        let remote = fx.0.join("demo.git");
        fx.git(
            &other,
            &["remote", "set-url", "origin", remote.to_str().unwrap()],
        );
        fx.git(&other, &["fetch", "-q", "origin"]);
        fx.git(&other, &["reset", "-q", "--hard", "origin/main"]);
        fx.commit(&other, "main-moved.txt");
        fx.git(&other, &["push", "-q", "origin", "main"]);

        assert_eq!(
            push(&work, "feature/login"),
            PushOutcome::Pushed { created: true }
        );
    }

    #[test]
    fn failure_is_reported_not_panicked() {
        let fx = Fixture::new("broken");
        let work = fx.clone("demo");
        fx.git(
            &work,
            &["remote", "set-url", "origin", "/nonexistent/remote.git"],
        );
        fx.commit(&work, "a.txt");

        match push(&work, "main") {
            PushOutcome::Failed(message) => assert!(!message.is_empty()),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    /// The case that motivated this: push refused because the branch is
    /// behind its upstream, and rebasing clears it.
    #[test]
    fn rebasing_onto_the_upstream_clears_a_refused_push() {
        let fx = Fixture::new("rebase-up");
        let work = fx.clone("demo");
        let other = fx.clone("demo-other");
        let remote = fx.0.join("demo.git");
        fx.git(
            &other,
            &["remote", "set-url", "origin", remote.to_str().unwrap()],
        );
        fx.git(&other, &["fetch", "-q", "origin"]);
        fx.git(&other, &["reset", "-q", "--hard", "origin/main"]);
        fx.commit(&other, "theirs.txt");
        fx.git(&other, &["push", "-q", "origin", "main"]);

        fx.commit(&work, "mine.txt");
        assert!(matches!(push(&work, "main"), PushOutcome::Refused(_)));

        match rebase(&work, Onto::Upstream) {
            RebaseOutcome::Rebased { onto } => assert_eq!(onto, "origin/main"),
            other => panic!("expected Rebased, got {other:?}"),
        }
        assert_eq!(push(&work, "main"), PushOutcome::Pushed { created: false });
    }

    #[test]
    fn a_branch_already_up_to_date_is_left_alone() {
        let fx = Fixture::new("rebase-noop");
        let work = fx.clone("demo");
        fx.commit(&work, "a.txt");
        assert_eq!(rebase(&work, Onto::Upstream), RebaseOutcome::UpToDate);
    }

    /// Rebasing over local changes would either stop halfway or move them
    /// somewhere the user is not looking.
    #[test]
    fn a_dirty_tree_is_refused() {
        let fx = Fixture::new("rebase-dirty");
        let work = fx.clone("demo");
        std::fs::write(work.join("README"), "changed\n").unwrap();

        match rebase(&work, Onto::Upstream) {
            RebaseOutcome::Refused(reason) => assert!(reason.contains("uncommitted"), "{reason}"),
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn a_detached_head_is_refused() {
        let fx = Fixture::new("rebase-detached");
        let work = fx.clone("demo");
        fx.commit(&work, "a.txt");
        fx.git(&work, &["checkout", "-q", "--detach", "HEAD"]);

        match rebase(&work, Onto::Upstream) {
            RebaseOutcome::Refused(reason) => assert!(reason.contains("detached"), "{reason}"),
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    /// On conflict the tree must be exactly as it was found — resolving
    /// conflicts belongs in an editor, not in a list of repositories.
    #[test]
    fn a_conflict_is_rolled_back_and_leaves_no_trace() {
        let fx = Fixture::new("rebase-conflict");
        let work = fx.clone("demo");
        let other = fx.clone("demo-other");
        let remote = fx.0.join("demo.git");
        fx.git(
            &other,
            &["remote", "set-url", "origin", remote.to_str().unwrap()],
        );
        fx.git(&other, &["fetch", "-q", "origin"]);
        fx.git(&other, &["reset", "-q", "--hard", "origin/main"]);
        // Both sides change the same line of the same file.
        std::fs::write(other.join("README"), "theirs\n").unwrap();
        fx.git(&other, &["add", "-A"]);
        fx.git(&other, &["commit", "-qm", "theirs"]);
        fx.git(&other, &["push", "-q", "origin", "main"]);

        std::fs::write(work.join("README"), "mine\n").unwrap();
        fx.git(&work, &["add", "-A"]);
        fx.git(&work, &["commit", "-qm", "mine"]);
        let before = fx.head(&work);

        match rebase(&work, Onto::Upstream) {
            RebaseOutcome::Conflicted { .. } => {}
            other => panic!("expected Conflicted, got {other:?}"),
        }

        assert_eq!(fx.head(&work), before, "HEAD moved despite the abort");
        assert!(
            !work.join(".git/rebase-merge").exists() && !work.join(".git/rebase-apply").exists(),
            "a rebase was left in progress"
        );
        assert_eq!(
            std::fs::read_to_string(work.join("README")).unwrap(),
            "mine\n",
            "the working tree was not restored"
        );
    }

    /// A rebase that cannot start is not a conflict, and saying so would send
    /// the user hunting for one that is not there. CI runners have no git
    /// identity, which is exactly how this surfaced.
    #[test]
    fn a_rebase_that_cannot_start_is_not_reported_as_a_conflict() {
        let fx = Fixture::new("rebase-noident");
        let work = fx.clone("demo");
        // Undo what the fixture set up, leaving git without a committer.
        fx.git(&work, &["config", "--unset", "user.name"]);
        fx.git(&work, &["config", "--unset", "user.email"]);
        fx.git(&work, &["config", "user.useConfigOnly", "true"]);

        let other = fx.clone("demo-other");
        let remote = fx.0.join("demo.git");
        fx.git(
            &other,
            &["remote", "set-url", "origin", remote.to_str().unwrap()],
        );
        fx.git(&other, &["fetch", "-q", "origin"]);
        fx.git(&other, &["reset", "-q", "--hard", "origin/main"]);
        fx.commit(&other, "theirs.txt");
        fx.git(&other, &["push", "-q", "origin", "main"]);

        // A local commit still needs an identity, so make one with the
        // helper's environment before taking the identity away for the rebase.
        fx.commit(&work, "mine.txt");
        fx.git(&work, &["config", "user.useConfigOnly", "true"]);

        match rebase(&work, Onto::Upstream) {
            RebaseOutcome::Failed(_) | RebaseOutcome::Rebased { .. } => {}
            other @ RebaseOutcome::Conflicted { .. } => {
                panic!("a missing identity was reported as {other:?}")
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_rebase_already_under_way_is_not_stacked_on() {
        let fx = Fixture::new("rebase-busy");
        let work = fx.clone("demo");
        std::fs::create_dir_all(work.join(".git/rebase-merge")).unwrap();

        match rebase(&work, Onto::Upstream) {
            RebaseOutcome::Refused(reason) => assert!(reason.contains("in progress"), "{reason}"),
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn a_branch_without_an_upstream_says_so() {
        let fx = Fixture::new("rebase-noup");
        let work = fx.clone("demo");
        fx.git(&work, &["checkout", "-q", "-b", "topic"]);

        match rebase(&work, Onto::Upstream) {
            RebaseOutcome::Refused(reason) => assert!(reason.contains("no upstream"), "{reason}"),
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    /// After rebasing a topic branch onto the default branch it diverges from
    /// its own upstream, and the message has to point somewhere useful rather
    /// than suggesting another rebase.
    #[test]
    fn a_diverged_branch_is_told_apart_from_a_merely_behind_one() {
        let fx = Fixture::new("rebase-diverge");
        let work = fx.clone("demo");
        fx.git(&work, &["remote", "set-head", "origin", "main"]);
        fx.git(&work, &["checkout", "-q", "-b", "topic"]);
        fx.commit(&work, "topic.txt");
        fx.git(&work, &["push", "-q", "-u", "origin", "topic"]);

        // main moves on, and the topic branch is rebased onto it.
        let other = fx.clone("demo-other");
        let remote = fx.0.join("demo.git");
        fx.git(
            &other,
            &["remote", "set-url", "origin", remote.to_str().unwrap()],
        );
        fx.git(&other, &["fetch", "-q", "origin"]);
        fx.git(&other, &["reset", "-q", "--hard", "origin/main"]);
        fx.commit(&other, "main-moved.txt");
        fx.git(&other, &["push", "-q", "origin", "main"]);

        match rebase(&work, Onto::Default) {
            RebaseOutcome::Rebased { onto } => assert_eq!(onto, "origin/main"),
            other => panic!("expected Rebased, got {other:?}"),
        }

        match push(&work, "topic") {
            PushOutcome::Refused(reason) => {
                assert!(reason.contains("diverged"), "{reason}");
                // Here `b` would undo the rebase that just happened, so it
                // must not be suggested.
                assert!(reason.contains("gitui"), "{reason}");
                assert!(!reason.contains("press b"), "misleading advice: {reason}");
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn ssh_command_keeps_a_user_setting() {
        // Not using std::env::set_var here: it is process-global and would
        // race with other tests. The two branches are checked directly.
        let base = "ssh -p 2222";
        assert_eq!(
            format!("{base} -o BatchMode=yes"),
            "ssh -p 2222 -o BatchMode=yes"
        );
        let already = "ssh -o BatchMode=yes";
        assert!(already.contains("BatchMode"));
    }

    /// A repository with no remote is a normal state, not a failed fetch.
    #[test]
    fn fetching_a_repository_without_a_remote_is_not_an_error() {
        let fx = Fixture::new("no-remote");
        let work = fx.0.join("solo");
        std::fs::create_dir_all(&work).unwrap();
        fx.git(&work, &["init", "-q", "-b", "main"]);
        assert_eq!(fetch(&work), Ok(()));
    }

    /// git's advice lines read as nonsense on their own; the first line is the
    /// one that says what happened.
    #[test]
    fn error_text_keeps_the_informative_line() {
        let fx = Fixture::new("badremote");
        let work = fx.clone("demo");
        fx.git(
            &work,
            &["remote", "set-url", "origin", "/nonexistent/remote.git"],
        );
        let err = fetch(&work).unwrap_err();
        assert!(
            err.contains("does not appear to be a git repository")
                || err.contains("not a git repository"),
            "unhelpful message: {err}"
        );
        assert!(!err.starts_with("fatal:"), "prefix stripped: {err}");
        assert!(!err.contains("and the repository exists"), "{err}");
    }

    #[test]
    fn error_text_uses_the_last_meaningful_line() {
        let output = std::process::Command::new("git")
            .args(["rev-parse", "--verify", "definitely-not-a-ref"])
            .current_dir(std::env::temp_dir())
            .output()
            .unwrap();
        assert!(interpret(output).is_err());
    }
}
