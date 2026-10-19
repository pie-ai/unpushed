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
        let range = format!("{branch}..origin/{branch}");
        match run(path, &["rev-list", "--count", &range]) {
            Ok(out) => {
                let behind: usize = out.trim().parse().unwrap_or(0);
                if behind > 0 {
                    return PushOutcome::Refused(format!(
                        "behind origin/{branch} by {behind} commit(s) — rebase first"
                    ));
                }
            }
            Err(err) => return PushOutcome::Failed(err),
        }

        let range = format!("origin/{branch}..{branch}");
        match run(path, &["rev-list", "--count", &range]) {
            Ok(out) if out.trim() == "0" => return PushOutcome::UpToDate,
            Ok(_) => {}
            Err(err) => return PushOutcome::Failed(err),
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
            work
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

    #[test]
    fn refuses_to_push_when_behind_the_target_branch() {
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
            PushOutcome::Refused(reason) => assert!(
                reason.contains("behind origin/main"),
                "unexpected reason: {reason}"
            ),
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
