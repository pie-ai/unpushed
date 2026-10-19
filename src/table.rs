//! Plain stdout rendering, used until the TUI exists.
//!
//! The layout already mirrors the planned TUI columns so the two stay
//! comparable: marker, name, branch, distance from upstream, distance from the
//! default branch, dirty flag.

use std::fmt::Write as _;

use crate::model::{Divergence, MainState, ScanOutcome, Summary, UpstreamState};

/// Which repositories to print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Default: only repositories with unpushed commits or local changes.
    /// Failed scans are always included — a repository that could not be read
    /// is not a quiet one.
    NeedsAttention,
    All,
}

/// Renders the header line and the table.
pub fn render(outcomes: &[ScanOutcome], view: View) -> String {
    let summary = Summary::of(outcomes);
    let rows: Vec<Row> = outcomes
        .iter()
        .filter(|outcome| match view {
            View::All => true,
            View::NeedsAttention => match outcome {
                ScanOutcome::Failed(_) => true,
                ScanOutcome::Ok(status) => status.needs_attention(),
            },
        })
        .map(Row::of)
        .collect();

    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} need work · {} clean · {} failed   ({} repos{})",
        summary.with_work,
        summary.clean,
        summary.failed,
        summary.total(),
        match view {
            View::NeedsAttention if summary.clean > 0 => format!(", {} hidden", summary.clean),
            _ => String::new(),
        }
    );

    if rows.is_empty() {
        out.push_str("\nNothing to do.\n");
        return out;
    }

    let widths = Widths::of(&rows);
    out.push('\n');
    for row in &rows {
        let mut line = String::new();
        let _ = write!(
            line,
            "{} {:name$}  {:branch$}  {:up$}  {:main$}  {}",
            row.marker,
            row.name,
            row.branch,
            row.upstream,
            row.main,
            row.dirty,
            name = widths.name,
            branch = widths.branch,
            up = widths.upstream,
            main = widths.main,
        );
        // Padding the last column would leave trailing blanks on every row
        // that has no dirty marker.
        out.push_str(line.trim_end());
        out.push('\n');
    }

    out
}

struct Row {
    marker: char,
    name: String,
    branch: String,
    upstream: String,
    main: String,
    dirty: String,
}

impl Row {
    fn of(outcome: &ScanOutcome) -> Self {
        match outcome {
            ScanOutcome::Failed(err) => Row {
                marker: '!',
                name: err.repo.name.clone(),
                branch: "—".into(),
                upstream: "failed".into(),
                main: err.message.clone(),
                dirty: String::new(),
            },
            ScanOutcome::Ok(status) => Row {
                marker: status.marker(),
                name: status.repo.name.clone(),
                branch: status.head.label(),
                upstream: match &status.upstream {
                    UpstreamState::Tracking { divergence, .. } => format_divergence(*divergence),
                    UpstreamState::NotSet => "no upstream".into(),
                    UpstreamState::Gone { .. } => "upstream gone".into(),
                },
                main: match &status.main {
                    MainState::Compared { divergence, .. } => format_divergence(*divergence),
                    MainState::OnMain { .. } => "on default".into(),
                    MainState::Unknown => "?".into(),
                },
                dirty: if status.worktree.is_dirty() {
                    format!("⚠ {}", describe_worktree(status))
                } else {
                    String::new()
                },
            },
        }
    }
}

fn format_divergence(divergence: Divergence) -> String {
    match (divergence.ahead, divergence.behind) {
        (0, 0) => "—".into(),
        (ahead, 0) => format!("↑{ahead}"),
        (0, behind) => format!("↓{behind}"),
        (ahead, behind) => format!("↑{ahead} ↓{behind}"),
    }
}

fn describe_worktree(status: &crate::model::RepoStatus) -> String {
    let w = &status.worktree;
    let mut parts = Vec::new();
    if w.staged > 0 {
        parts.push(format!("{} staged", w.staged));
    }
    if w.unstaged > 0 {
        parts.push(format!("{} modified", w.unstaged));
    }
    if w.untracked > 0 {
        parts.push(format!("{} untracked", w.untracked));
    }
    if w.conflicted > 0 {
        parts.push(format!("{} conflicted", w.conflicted));
    }
    parts.join(", ")
}

struct Widths {
    name: usize,
    branch: usize,
    upstream: usize,
    main: usize,
}

impl Widths {
    fn of(rows: &[Row]) -> Self {
        // Column widths are measured in characters, not bytes: the markers and
        // arrows are multi-byte and would otherwise skew the padding.
        let width = |pick: fn(&Row) -> &str| {
            rows.iter()
                .map(|row| pick(row).chars().count())
                .max()
                .unwrap_or(0)
        };
        Widths {
            name: width(|r| &r.name),
            branch: width(|r| &r.branch),
            upstream: width(|r| &r.upstream),
            main: width(|r| &r.main),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Head, Repo, RepoStatus, ScanError, Worktree};
    use std::path::PathBuf;

    fn ok(name: &str, upstream: UpstreamState, main: MainState, worktree: Worktree) -> ScanOutcome {
        ScanOutcome::Ok(Box::new(RepoStatus {
            repo: Repo {
                name: name.into(),
                path: PathBuf::from("/tmp").join(name),
            },
            head: Head::Branch("feature/login".into()),
            upstream,
            main,
            worktree,
        }))
    }

    fn compared(ahead: usize, behind: usize) -> MainState {
        MainState::Compared {
            name: "origin/main".into(),
            divergence: Divergence { ahead, behind },
        }
    }

    #[test]
    fn default_view_hides_quiet_repositories_but_counts_them() {
        let outcomes = vec![
            ok(
                "backend-api",
                UpstreamState::NotSet,
                compared(11, 0),
                Worktree::default(),
            ),
            ok(
                "web-frontend",
                UpstreamState::NotSet,
                compared(0, 0),
                Worktree::default(),
            ),
        ];

        let out = render(&outcomes, View::NeedsAttention);
        assert!(out.contains("1 need work · 1 clean · 0 failed"));
        assert!(out.contains("2 repos, 1 hidden"));
        assert!(out.contains("backend-api"));
        assert!(!out.contains("web-frontend"));
    }

    /// A failed scan is not a clean repository and must stay visible.
    #[test]
    fn failures_are_shown_even_in_the_filtered_view() {
        let outcomes = vec![
            ok(
                "web-frontend",
                UpstreamState::NotSet,
                compared(0, 0),
                Worktree::default(),
            ),
            ScanOutcome::Failed(ScanError {
                repo: Repo {
                    name: "broken".into(),
                    path: PathBuf::from("/tmp/broken"),
                },
                message: "could not find repository".into(),
            }),
        ];

        let out = render(&outcomes, View::NeedsAttention);
        assert!(out.contains("0 need work · 1 clean · 1 failed"));
        assert!(out.contains("broken"));
    }

    #[test]
    fn all_view_shows_everything() {
        let outcomes = vec![ok(
            "web-frontend",
            UpstreamState::NotSet,
            compared(0, 0),
            Worktree::default(),
        )];
        let out = render(&outcomes, View::All);
        assert!(out.contains("web-frontend"));
    }

    #[test]
    fn divergence_rendering_covers_all_four_shapes() {
        assert_eq!(format_divergence(Divergence::default()), "—");
        assert_eq!(
            format_divergence(Divergence {
                ahead: 3,
                behind: 0
            }),
            "↑3"
        );
        assert_eq!(
            format_divergence(Divergence {
                ahead: 0,
                behind: 4
            }),
            "↓4"
        );
        assert_eq!(
            format_divergence(Divergence {
                ahead: 3,
                behind: 4
            }),
            "↑3 ↓4"
        );
    }

    #[test]
    fn empty_result_says_so() {
        let out = render(&[], View::NeedsAttention);
        assert!(out.contains("Nothing to do."));
    }
}
