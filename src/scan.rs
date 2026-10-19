//! Reading the local status of discovered checkouts.
//!
//! Everything here is local: no ref is resolved over the network. The default
//! branch is taken from `refs/remotes/origin/HEAD`, which was present in 82 of
//! 85 surveyed repositories; the remaining ones report [`MainState::Unknown`]
//! and are resolved later by the fetch step rather than blocking the scan.
//!
//! Measured on 47 checkouts with a warm cache: 2.2s sequentially, 0.7s across
//! eight threads. The gain is larger on a cold cache (17.2s for 85 repos),
//! where the work is I/O bound.

use std::sync::atomic::{AtomicUsize, Ordering};

use git2::{BranchType, ErrorCode, Oid, Repository, StatusOptions};

use crate::model::{
    Divergence, Head, MainState, Repo, RepoStatus, ScanError, ScanOutcome, UpstreamState, Worktree,
};

/// Default worker count. Eight threads already saturate the measured workload;
/// sixteen bought little beyond it.
pub const DEFAULT_THREADS: usize = 8;

/// Scans every repository, distributing them over `threads` workers.
///
/// Results keep the order of `repos` so the table does not reshuffle between
/// runs. Scanning never fails as a whole — a repository that cannot be read
/// becomes a [`ScanOutcome::Failed`].
pub fn scan_all(repos: &[Repo], threads: usize) -> Vec<ScanOutcome> {
    if repos.is_empty() {
        return Vec::new();
    }

    let workers = threads.max(1).min(repos.len());
    let mut results: Vec<Option<ScanOutcome>> = (0..repos.len()).map(|_| None).collect();
    let next = AtomicUsize::new(0);

    // Workers pull indices off a shared cursor and report back over a channel;
    // the caller reassembles them into input order. The channel is also what a
    // later TUI will consume to fill rows as they arrive, instead of waiting
    // for the whole scan.
    let (tx, rx) = std::sync::mpsc::channel::<(usize, ScanOutcome)>();

    std::thread::scope(|scope| {
        for _ in 0..workers {
            let tx = tx.clone();
            let next = &next;
            scope.spawn(move || loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(repo) = repos.get(index) else { break };
                if tx.send((index, scan_one(repo))).is_err() {
                    break;
                }
            });
        }
        // Drop the original sender so the receive loop below terminates once
        // the last worker is done.
        drop(tx);

        for (index, outcome) in rx {
            results[index] = Some(outcome);
        }
    });

    results
        .into_iter()
        .map(|slot| slot.expect("every index is visited exactly once"))
        .collect()
}

/// Reads one repository. Errors are captured rather than propagated so a
/// single broken checkout cannot hide the other rows.
pub fn scan_one(repo: &Repo) -> ScanOutcome {
    match read_status(repo) {
        Ok(status) => ScanOutcome::Ok(Box::new(status)),
        Err(err) => ScanOutcome::Failed(ScanError {
            repo: repo.clone(),
            message: err.message().to_string(),
        }),
    }
}

fn read_status(repo: &Repo) -> Result<RepoStatus, git2::Error> {
    let git = Repository::open(&repo.path)?;

    let head = read_head(&git)?;
    let main_ref = default_branch(&git);
    let upstream = read_upstream(&git, &head)?;
    let main = read_main(&git, &head, main_ref.as_deref())?;
    let worktree = read_worktree(&git)?;

    Ok(RepoStatus {
        repo: repo.clone(),
        head,
        upstream,
        main,
        worktree,
    })
}

fn read_head(git: &Repository) -> Result<Head, git2::Error> {
    match git.head() {
        Ok(reference) => {
            if reference.is_branch() {
                if let Ok(name) = reference.shorthand() {
                    return Ok(Head::Branch(name.to_string()));
                }
            }
            let oid = reference
                .target()
                .map(|oid| {
                    let text = oid.to_string();
                    text[..7.min(text.len())].to_string()
                })
                .unwrap_or_else(|| "unknown".to_string());
            Ok(Head::Detached(oid))
        }
        // An unborn branch is a valid state, not a failure: the repository was
        // initialised but carries no commit yet.
        Err(err) if err.code() == ErrorCode::UnbornBranch => {
            let name = git
                .find_reference("HEAD")
                .ok()
                .and_then(|r| r.symbolic_target().ok().flatten().map(str::to_string))
                .and_then(|target| target.strip_prefix("refs/heads/").map(str::to_string))
                .unwrap_or_else(|| "HEAD".to_string());
            Ok(Head::Unborn(name))
        }
        Err(err) => Err(err),
    }
}

/// The commit that "unpushed" is measured against: the branch's upstream when
/// one is configured, the remote default branch otherwise.
///
/// Shared with [`crate::commits`] on purpose. If the commit panel resolved its
/// own reference, the list could report `↑21` while the panel listed a
/// different set — and there would be no way to tell which one was right.
pub fn base_ref(git: &Repository) -> Result<Option<Oid>, git2::Error> {
    let head = read_head(git)?;

    if let Some(branch_name) = head.branch_name() {
        if let Ok(branch) = git.find_branch(branch_name, BranchType::Local) {
            if let Ok(upstream) = branch.upstream() {
                return Ok(upstream.get().target());
            }
        }
    }

    let Some(main_ref) = default_branch(git) else {
        return Ok(None);
    };
    match git.find_branch(&main_ref, BranchType::Remote) {
        Ok(branch) => Ok(branch.get().target()),
        Err(err) if err.code() == ErrorCode::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Resolves the remote default branch from the local `origin/HEAD` ref.
/// Returns `None` when it is absent; no network lookup is attempted.
fn default_branch(git: &Repository) -> Option<String> {
    let reference = git.find_reference("refs/remotes/origin/HEAD").ok()?;
    let target = reference.symbolic_target().ok().flatten()?;
    target
        .strip_prefix("refs/remotes/")
        .map(|name| name.to_string())
}

fn read_upstream(git: &Repository, head: &Head) -> Result<UpstreamState, git2::Error> {
    let Some(branch_name) = head.branch_name() else {
        return Ok(UpstreamState::NotSet);
    };

    let branch = git.find_branch(branch_name, BranchType::Local)?;
    let upstream = match branch.upstream() {
        Ok(upstream) => upstream,
        Err(err) if err.code() == ErrorCode::NotFound => return Ok(UpstreamState::NotSet),
        Err(err) => return Err(err),
    };

    let name = upstream.name()?.unwrap_or("origin/?").to_string();

    let (Some(local), Some(remote)) = (branch.get().target(), upstream.get().target()) else {
        return Ok(UpstreamState::Gone { name });
    };

    let (ahead, behind) = git.graph_ahead_behind(local, remote)?;
    Ok(UpstreamState::Tracking {
        name,
        divergence: Divergence { ahead, behind },
    })
}

fn read_main(
    git: &Repository,
    head: &Head,
    main_ref: Option<&str>,
) -> Result<MainState, git2::Error> {
    let Some(main_ref) = main_ref else {
        return Ok(MainState::Unknown);
    };

    // `origin/main` is reported relative to the remote, so a local branch of
    // the same name means HEAD already is the default branch.
    if let Some(branch) = head.branch_name() {
        if main_ref.strip_prefix("origin/") == Some(branch) {
            return Ok(MainState::OnMain {
                name: main_ref.to_string(),
            });
        }
    }

    let remote = match git.find_branch(main_ref, BranchType::Remote) {
        Ok(branch) => branch,
        Err(err) if err.code() == ErrorCode::NotFound => return Ok(MainState::Unknown),
        Err(err) => return Err(err),
    };

    let (Some(local_oid), Some(remote_oid)) = (
        git.head().ok().and_then(|h| h.target()),
        remote.get().target(),
    ) else {
        return Ok(MainState::Unknown);
    };

    let (ahead, behind) = git.graph_ahead_behind(local_oid, remote_oid)?;
    Ok(MainState::Compared {
        name: main_ref.to_string(),
        divergence: Divergence { ahead, behind },
    })
}

fn read_worktree(git: &Repository) -> Result<Worktree, git2::Error> {
    if git.is_bare() {
        return Ok(Worktree::default());
    }

    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        // Listing every file inside an untracked directory is pure cost here;
        // the directory itself is enough to mark the repository dirty.
        .recurse_untracked_dirs(false)
        .include_ignored(false)
        .exclude_submodules(true);

    let statuses = git.statuses(Some(&mut options))?;
    let mut worktree = Worktree::default();

    for entry in statuses.iter() {
        let status = entry.status();
        if status.is_conflicted() {
            worktree.conflicted += 1;
            continue;
        }
        if status.is_wt_new() {
            worktree.untracked += 1;
            continue;
        }
        if status.is_index_new()
            || status.is_index_modified()
            || status.is_index_deleted()
            || status.is_index_renamed()
            || status.is_index_typechange()
        {
            worktree.staged += 1;
        }
        if status.is_wt_modified()
            || status.is_wt_deleted()
            || status.is_wt_renamed()
            || status.is_wt_typechange()
        {
            worktree.unstaged += 1;
        }
    }

    Ok(worktree)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(tag: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "unpushed-scan-{tag}-{}-{:?}",
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
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        /// Builds a bare "remote" with a `main` branch and a clone of it whose
        /// `origin/HEAD` is set, mirroring the surveyed checkouts.
        fn clone_with_main(&self, name: &str) -> PathBuf {
            let remote = self.0.join(format!("{name}.git"));
            let seed = self.0.join(format!("{name}-seed"));
            std::fs::create_dir_all(&seed).unwrap();
            self.git(&seed, &["init", "-q", "-b", "main"]);
            std::fs::write(seed.join("README"), "seed\n").unwrap();
            self.git(&seed, &["add", "."]);
            self.git(&seed, &["commit", "-qm", "seed"]);
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
            self.git(&work, &["remote", "set-head", "origin", "main"]);
            work
        }

        fn commit(&self, work: &Path, file: &str, body: &str) {
            std::fs::write(work.join(file), body).unwrap();
            self.git(work, &["add", "."]);
            self.git(work, &["commit", "-qm", &format!("add {file}")]);
        }

        fn repo(&self, path: PathBuf) -> Repo {
            Repo {
                name: path.file_name().unwrap().to_string_lossy().into_owned(),
                path,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn status_of(outcome: ScanOutcome) -> RepoStatus {
        match outcome {
            ScanOutcome::Ok(status) => *status,
            ScanOutcome::Failed(err) => panic!("scan failed: {}", err.message),
        }
    }

    #[test]
    fn fresh_clone_is_on_main_and_clean() {
        let fx = Fixture::new("clean");
        let work = fx.clone_with_main("demo");
        let status = status_of(scan_one(&fx.repo(work)));

        assert_eq!(status.head, Head::Branch("main".into()));
        assert_eq!(status.unpushed(), Some(0));
        assert!(!status.worktree.is_dirty());
        assert!(!status.needs_attention());
    }

    #[test]
    fn commits_on_tracking_branch_count_as_unpushed() {
        let fx = Fixture::new("ahead");
        let work = fx.clone_with_main("demo");
        fx.commit(&work, "a.txt", "a\n");
        fx.commit(&work, "b.txt", "b\n");

        let status = status_of(scan_one(&fx.repo(work)));
        assert_eq!(status.unpushed(), Some(2));
        assert_eq!(status.behind(), Some(0));
        assert_eq!(status.marker(), '↑');
    }

    /// The dominant shape in the surveyed checkouts: a ticket branch created
    /// locally, never pushed, carrying work.
    #[test]
    fn local_branch_without_upstream_measures_against_main() {
        let fx = Fixture::new("noupstream");
        let work = fx.clone_with_main("demo");
        fx.git(&work, &["checkout", "-q", "-b", "feature/login"]);
        fx.commit(&work, "a.txt", "a\n");
        fx.commit(&work, "b.txt", "b\n");
        fx.commit(&work, "c.txt", "c\n");

        let status = status_of(scan_one(&fx.repo(work)));
        assert_eq!(status.upstream, UpstreamState::NotSet);
        assert_eq!(status.unpushed(), Some(3));
        assert!(status.needs_attention());
    }

    /// The other 30 of those 32: branch created, nothing done.
    #[test]
    fn local_branch_without_upstream_and_without_work_is_quiet() {
        let fx = Fixture::new("noupstream-empty");
        let work = fx.clone_with_main("demo");
        fx.git(&work, &["checkout", "-q", "-b", "feature/login"]);

        let status = status_of(scan_one(&fx.repo(work)));
        assert_eq!(status.upstream, UpstreamState::NotSet);
        assert_eq!(status.unpushed(), Some(0));
        assert!(!status.needs_attention());
    }

    #[test]
    fn untracked_and_modified_files_are_counted_apart() {
        let fx = Fixture::new("dirty");
        let work = fx.clone_with_main("demo");
        std::fs::write(work.join("README"), "changed\n").unwrap();
        std::fs::write(work.join("new.txt"), "new\n").unwrap();

        let status = status_of(scan_one(&fx.repo(work)));
        assert_eq!(status.worktree.unstaged, 1);
        assert_eq!(status.worktree.untracked, 1);
        assert_eq!(status.worktree.tracked_changes(), 1);
        assert!(status.needs_attention());
    }

    #[test]
    fn detached_head_is_reported_not_failed() {
        let fx = Fixture::new("detached");
        let work = fx.clone_with_main("demo");
        fx.commit(&work, "a.txt", "a\n");
        fx.git(&work, &["checkout", "-q", "--detach", "HEAD"]);

        let status = status_of(scan_one(&fx.repo(work)));
        assert!(matches!(status.head, Head::Detached(_)));
        assert_eq!(status.upstream, UpstreamState::NotSet);
    }

    #[test]
    fn repository_without_origin_head_reports_unknown_main() {
        let fx = Fixture::new("standalone");
        let work = fx.0.join("solo");
        std::fs::create_dir_all(&work).unwrap();
        fx.git(&work, &["init", "-q", "-b", "main"]);
        std::fs::write(work.join("a.txt"), "a\n").unwrap();
        fx.git(&work, &["add", "."]);
        fx.git(&work, &["commit", "-qm", "one"]);

        let status = status_of(scan_one(&fx.repo(work)));
        assert_eq!(status.main, MainState::Unknown);
        // Unknown must not be reported as "nothing to push".
        assert_eq!(status.unpushed(), None);
        assert!(!status.needs_attention());
    }

    #[test]
    fn unreadable_repository_fails_without_poisoning_the_run() {
        let fx = Fixture::new("broken");
        let path = fx.0.join("broken");
        std::fs::create_dir_all(path.join(".git")).unwrap();

        let repos = vec![fx.repo(path), fx.repo(fx.clone_with_main("demo"))];
        let outcomes = scan_all(&repos, 4);

        assert!(matches!(outcomes[0], ScanOutcome::Failed(_)));
        assert!(matches!(outcomes[1], ScanOutcome::Ok(_)));
    }

    #[test]
    fn scan_all_preserves_input_order() {
        let fx = Fixture::new("order");
        let repos: Vec<Repo> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|name| fx.repo(fx.clone_with_main(name)))
            .collect();

        let outcomes = scan_all(&repos, 4);
        let names: Vec<&str> = outcomes.iter().map(|o| o.repo().name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c", "d", "e"]);
    }

    #[test]
    fn scanning_nothing_is_not_an_error() {
        assert!(scan_all(&[], DEFAULT_THREADS).is_empty());
    }
}
