//! Status model for a single git checkout.
//!
//! Two invariants drive the shape of these types:
//!
//! 1. A repository whose scan failed must never be mistaken for a clean one.
//!    That is why scanning yields [`ScanOutcome`] with an explicit
//!    [`ScanError`] variant instead of a status with default values.
//! 2. "Has no upstream" and "has work waiting" are different things. In the
//!    surveyed checkouts, 32 repositories had no upstream but 30 of those had
//!    nothing to push. [`UpstreamState`] and [`RepoStatus::unpushed`] keep the
//!    two apart.

use std::path::PathBuf;

/// A git checkout found by discovery, before any status was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    /// Directory name, used as the display label.
    pub name: String,
    pub path: PathBuf,
}

/// How many commits two refs differ by, as reported by `graph_ahead_behind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Divergence {
    /// Commits present locally but not on the other ref.
    pub ahead: usize,
    /// Commits present on the other ref but not locally.
    pub behind: usize,
}

impl Divergence {
    pub fn is_even(&self) -> bool {
        self.ahead == 0 && self.behind == 0
    }
}

/// What HEAD points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    Branch(String),
    /// Detached HEAD, holding the abbreviated object id.
    Detached(String),
    /// A branch exists but carries no commit yet (fresh `git init`).
    Unborn(String),
}

impl Head {
    /// Label for the branch column.
    pub fn label(&self) -> String {
        match self {
            Head::Branch(name) => name.clone(),
            Head::Detached(oid) => format!("({oid})"),
            Head::Unborn(name) => format!("{name} (unborn)"),
        }
    }

    pub fn branch_name(&self) -> Option<&str> {
        match self {
            Head::Branch(name) => Some(name),
            _ => None,
        }
    }
}

/// Relation of the current branch to its configured upstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpstreamState {
    Tracking {
        name: String,
        divergence: Divergence,
    },
    /// No upstream configured. The dominant case in the surveyed checkouts:
    /// a ticket branch created locally and never pushed.
    NotSet,
    /// An upstream is configured but its ref is gone, e.g. the remote branch
    /// was deleted after a merge.
    Gone { name: String },
}

/// Relation of the current branch to the remote's default branch.
///
/// For branches without an upstream this is the only meaningful distance, so
/// it is read for every repository rather than on demand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainState {
    Compared {
        name: String,
        divergence: Divergence,
    },
    /// HEAD already is the default branch, so the comparison is trivial.
    OnMain { name: String },
    /// The default branch could not be determined locally. Resolving it over
    /// the network is deferred to the fetch step.
    Unknown,
}

/// Working tree state. Counts are cheap to collect and let the UI explain the
/// dirty marker, even though a single flag would be enough to render it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Worktree {
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
    pub conflicted: usize,
}

impl Worktree {
    pub fn is_dirty(&self) -> bool {
        self.staged + self.unstaged + self.untracked + self.conflicted > 0
    }

    /// Changes to tracked files, ignoring untracked clutter.
    pub fn tracked_changes(&self) -> usize {
        self.staged + self.unstaged + self.conflicted
    }
}

/// Everything read from one repository during a local scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoStatus {
    pub repo: Repo,
    pub head: Head,
    pub upstream: UpstreamState,
    pub main: MainState,
    pub worktree: Worktree,
}

impl RepoStatus {
    /// Commits that exist locally but not on the remote.
    ///
    /// Uses the upstream when one is configured, and falls back to the
    /// distance from the default branch otherwise — for a branch that was
    /// never pushed, everything since it forked off is unpushed.
    ///
    /// Returns `None` when neither reference point is available, which is not
    /// the same as zero: it means unknown.
    pub fn unpushed(&self) -> Option<usize> {
        match &self.upstream {
            UpstreamState::Tracking { divergence, .. } => Some(divergence.ahead),
            UpstreamState::NotSet | UpstreamState::Gone { .. } => match &self.main {
                MainState::Compared { divergence, .. } => Some(divergence.ahead),
                MainState::OnMain { .. } => Some(0),
                MainState::Unknown => None,
            },
        }
    }

    /// Commits on the remote that are missing locally. Push is refused while
    /// this is non-zero, so it is read separately from [`Self::unpushed`].
    pub fn behind(&self) -> Option<usize> {
        match &self.upstream {
            UpstreamState::Tracking { divergence, .. } => Some(divergence.behind),
            UpstreamState::NotSet | UpstreamState::Gone { .. } => match &self.main {
                MainState::Compared { divergence, .. } => Some(divergence.behind),
                MainState::OnMain { .. } => Some(0),
                MainState::Unknown => None,
            },
        }
    }

    /// Whether the repository belongs in the default view.
    pub fn needs_attention(&self) -> bool {
        self.worktree.is_dirty() || self.unpushed().unwrap_or(0) > 0
    }

    /// Single-character marker for the status column.
    pub fn marker(&self) -> char {
        let ahead = self.unpushed().unwrap_or(0);
        let behind = self.behind().unwrap_or(0);
        match (ahead > 0, behind > 0) {
            (true, true) => '⇅',
            (true, false) => '↑',
            (false, true) => '↓',
            (false, false) if self.worktree.is_dirty() => '⚠',
            (false, false) => '·',
        }
    }
}

/// Why a repository could not be scanned. Kept separate from a status so a
/// failure can never be rendered as "clean".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanError {
    pub repo: Repo,
    pub message: String,
}

/// Result of scanning one repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanOutcome {
    Ok(Box<RepoStatus>),
    Failed(ScanError),
}

impl ScanOutcome {
    pub fn repo(&self) -> &Repo {
        match self {
            ScanOutcome::Ok(status) => &status.repo,
            ScanOutcome::Failed(err) => &err.repo,
        }
    }

    pub fn status(&self) -> Option<&RepoStatus> {
        match self {
            ScanOutcome::Ok(status) => Some(status),
            ScanOutcome::Failed(_) => None,
        }
    }
}

/// Counts for the header line, so every discovered repository is accounted
/// for even when the view hides the quiet ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Summary {
    pub with_work: usize,
    pub clean: usize,
    pub failed: usize,
}

impl Summary {
    pub fn of(outcomes: &[ScanOutcome]) -> Self {
        let mut summary = Summary::default();
        for outcome in outcomes {
            match outcome {
                ScanOutcome::Failed(_) => summary.failed += 1,
                ScanOutcome::Ok(status) if status.needs_attention() => summary.with_work += 1,
                ScanOutcome::Ok(_) => summary.clean += 1,
            }
        }
        summary
    }

    pub fn total(&self) -> usize {
        self.with_work + self.clean + self.failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> Repo {
        Repo {
            name: "demo".into(),
            path: PathBuf::from("/tmp/demo"),
        }
    }

    fn status(upstream: UpstreamState, main: MainState, worktree: Worktree) -> RepoStatus {
        RepoStatus {
            repo: repo(),
            head: Head::Branch("feature/login".into()),
            upstream,
            main,
            worktree,
        }
    }

    fn compared(ahead: usize, behind: usize) -> MainState {
        MainState::Compared {
            name: "origin/main".into(),
            divergence: Divergence { ahead, behind },
        }
    }

    #[test]
    fn upstream_wins_over_main_when_configured() {
        let s = status(
            UpstreamState::Tracking {
                name: "origin/feature/login".into(),
                divergence: Divergence {
                    ahead: 2,
                    behind: 0,
                },
            },
            compared(9, 4),
            Worktree::default(),
        );
        assert_eq!(s.unpushed(), Some(2));
        assert_eq!(s.behind(), Some(0));
    }

    #[test]
    fn branch_without_upstream_falls_back_to_main() {
        let s = status(UpstreamState::NotSet, compared(11, 0), Worktree::default());
        assert_eq!(s.unpushed(), Some(11));
        assert!(s.needs_attention());
    }

    /// The case that motivated splitting the two concepts: 30 of 47 surveyed
    /// repositories sat on a never-pushed branch with nothing on it.
    #[test]
    fn branch_without_upstream_and_without_commits_is_quiet() {
        let s = status(UpstreamState::NotSet, compared(0, 0), Worktree::default());
        assert_eq!(s.unpushed(), Some(0));
        assert!(!s.needs_attention());
        assert_eq!(s.marker(), '·');
    }

    #[test]
    fn unknown_main_is_not_reported_as_zero() {
        let s = status(
            UpstreamState::NotSet,
            MainState::Unknown,
            Worktree::default(),
        );
        assert_eq!(s.unpushed(), None);
        assert!(!s.needs_attention());
    }

    #[test]
    fn dirty_alone_is_enough_to_need_attention() {
        let s = status(
            UpstreamState::NotSet,
            compared(0, 0),
            Worktree {
                unstaged: 1,
                ..Worktree::default()
            },
        );
        assert!(s.needs_attention());
        assert_eq!(s.marker(), '⚠');
    }

    #[test]
    fn untracked_only_still_counts_as_dirty_but_not_as_tracked_change() {
        let w = Worktree {
            untracked: 3,
            ..Worktree::default()
        };
        assert!(w.is_dirty());
        assert_eq!(w.tracked_changes(), 0);
    }

    #[test]
    fn summary_accounts_for_every_outcome() {
        let outcomes = vec![
            ScanOutcome::Ok(Box::new(status(
                UpstreamState::NotSet,
                compared(3, 0),
                Worktree::default(),
            ))),
            ScanOutcome::Ok(Box::new(status(
                UpstreamState::NotSet,
                compared(0, 0),
                Worktree::default(),
            ))),
            ScanOutcome::Failed(ScanError {
                repo: repo(),
                message: "boom".into(),
            }),
        ];
        let summary = Summary::of(&outcomes);
        assert_eq!(summary.with_work, 1);
        assert_eq!(summary.clean, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.total(), outcomes.len());
    }
}
