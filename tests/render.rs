//! Rendering tests driving the real widgets through ratatui's test backend.
//!
//! These exist because the interesting rules of the interface are visual: what
//! the header accounts for, whether a failed scan can hide, and whether a bulk
//! push names what it would hit. Compiling is not evidence of any of that.

use ratatui::backend::TestBackend;
use ratatui::style::Color;
use ratatui::Terminal;

use unpushed::app::{Activity, App, Diff, JobResult, Key, Panel};
use unpushed::commits::{Commit, DiffLine, FileChange, LineKind, Status};
use unpushed::model::{
    Divergence, Head, MainState, Repo, RepoStatus, ScanError, ScanOutcome, UpstreamState, Worktree,
};
use unpushed::ui;

fn repo(name: &str) -> Repo {
    Repo {
        name: name.into(),
        path: std::path::PathBuf::from("/tmp").join(name),
    }
}

fn ok(name: &str, branch: &str, ahead: usize, dirty: bool) -> ScanOutcome {
    ScanOutcome::Ok(Box::new(RepoStatus {
        repo: repo(name),
        head: Head::Branch(branch.into()),
        upstream: UpstreamState::Tracking {
            name: format!("origin/{branch}"),
            divergence: Divergence { ahead, behind: 0 },
        },
        main: MainState::Compared {
            name: "origin/main".into(),
            divergence: Divergence { ahead, behind: 0 },
        },
        worktree: if dirty {
            Worktree {
                unstaged: 2,
                ..Worktree::default()
            }
        } else {
            Worktree::default()
        },
    }))
}

/// Renders the app into a buffer and returns it as plain text.
fn screen(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Foreground colour of the first cell on the line containing `needle`.
/// Used to check that busy rows are dimmed rather than just assuming it.
fn fg_of_row(app: &App, needle: &str, width: u16, height: u16) -> Option<Color> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    for y in 0..buffer.area.height {
        let line: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        if let Some(col) = line.find(needle) {
            return Some(buffer[(col as u16, y)].style().fg.unwrap_or(Color::Reset));
        }
    }
    None
}

fn app() -> App {
    let mut app = App::new(
        "/src/projects".into(),
        vec![
            repo("backend-api"),
            repo("web-frontend"),
            repo("api-gateway"),
        ],
    );
    app.apply(JobResult::Scanned {
        index: 0,
        outcome: Box::new(ok("backend-api", "feature/login", 3, false)),
    });
    app.apply(JobResult::Scanned {
        index: 1,
        outcome: Box::new(ok("web-frontend", "main", 0, false)),
    });
    app.apply(JobResult::Scanned {
        index: 2,
        outcome: Box::new(ok("api-gateway", "main", 0, true)),
    });
    app
}

#[test]
fn header_accounts_for_every_repository_even_when_rows_are_hidden() {
    let out = screen(&app(), 100, 12);
    assert!(out.contains("2 need work"), "{out}");
    assert!(out.contains("1 clean"), "{out}");
    assert!(out.contains("0 failed"), "{out}");
    assert!(out.contains("3 repos, 2 visible"), "{out}");
    // The quiet repository is counted but not listed.
    assert!(!out.contains("web-frontend"), "{out}");
}

#[test]
fn rows_show_branch_divergence_and_dirty_state() {
    let out = screen(&app(), 100, 12);
    assert!(out.contains("backend-api"), "{out}");
    assert!(out.contains("feature/login"), "{out}");
    assert!(out.contains("↑3"), "{out}");
    assert!(out.contains("2 modified"), "{out}");
}

#[test]
fn a_failed_scan_is_visible_and_counted_as_an_error() {
    let mut app = app();
    app.apply(JobResult::Scanned {
        index: 1,
        outcome: Box::new(ScanOutcome::Failed(ScanError {
            repo: repo("web-frontend"),
            message: "could not find repository".into(),
        })),
    });
    let out = screen(&app, 110, 12);
    assert!(out.contains("1 failed"), "{out}");
    assert!(out.contains("web-frontend"), "{out}");
    assert!(out.contains("could not find"), "{out}");
}

#[test]
fn background_fetch_progress_is_shown_in_the_header() {
    let mut app = app();
    app.fetching = 7;
    let out = screen(&app, 100, 12);
    assert!(out.contains("fetching"), "{out}");
    assert!(out.contains("7 left"), "{out}");
}

#[test]
fn filter_is_echoed_while_typing_and_narrows_the_list() {
    let mut app = app();
    app.on_key(Key::Char('/'));
    for c in "backend".chars() {
        app.on_key(Key::Char(c));
    }
    let out = screen(&app, 100, 12);
    assert!(out.contains("Filter: backend"), "{out}");
    assert!(out.contains("backend-api"), "{out}");
    assert!(!out.contains("api-gateway"), "{out}");
}

/// The confirmation is the safeguard against confusing `A` with `P`, so it has
/// to name the repositories rather than only count them.
#[test]
fn bulk_push_confirmation_lists_its_targets() {
    let mut app = app();
    app.on_key(Key::Char('A'));
    let out = screen(&app, 100, 20);
    assert!(out.contains("Push"), "{out}");
    assert!(out.contains("Push 2 visible repositories?"), "{out}");
    assert!(out.contains("backend-api"), "{out}");
    assert!(out.contains("api-gateway"), "{out}");
    assert!(out.contains("[y] push"), "{out}");
}

#[test]
fn marked_rows_are_flagged_in_the_list() {
    let mut app = app();
    app.on_key(Key::Char(' '));
    let out = screen(&app, 100, 12);
    let row = out
        .lines()
        .find(|line| line.contains("backend-api"))
        .unwrap();
    assert!(row.trim_start().starts_with('x'), "{row}");
}

#[test]
fn empty_list_explains_how_to_see_everything() {
    let mut app = App::new("/src".into(), vec![repo("quiet")]);
    app.apply(JobResult::Scanned {
        index: 0,
        outcome: Box::new(ok("quiet", "main", 0, false)),
    });
    let out = screen(&app, 80, 10);
    assert!(out.contains("Nothing to do"), "{out}");
}

#[test]
fn rendering_survives_a_very_small_terminal() {
    let mut app = app();
    app.on_key(Key::Char('A'));
    // Must not panic: the confirmation popup is larger than the screen.
    let out = screen(&app, 20, 6);
    assert!(!out.is_empty());
}

#[test]
fn unscanned_rows_render_as_pending_rather_than_clean() {
    let app = App::new("/src".into(), vec![repo("not-read-yet")]);
    let out = screen(&app, 80, 10);
    assert!(out.contains("not-read-yet"), "{out}");
    assert!(out.contains("reading"), "{out}");
    assert!(out.contains("0 clean"), "{out}");
}

/// A push takes seconds. Without a visible sign, it looks like nothing happened.
#[test]
fn a_busy_row_shows_a_spinner_and_says_what_it_is_doing() {
    let mut app = app();
    app.rows[0].set_busy(Activity::Pushing, true);
    let out = screen(&app, 100, 12);

    let row = out.lines().find(|l| l.contains("backend-api")).unwrap();
    assert!(row.contains("pushing …"), "{row}");
    assert!(
        row.chars().any(|c| "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏".contains(c)),
        "no spinner frame in: {row}"
    );
}

#[test]
fn each_activity_names_itself() {
    for (activity, word) in [
        (Activity::Scanning, "reading …"),
        (Activity::Fetching, "fetching …"),
        (Activity::Pushing, "pushing …"),
    ] {
        let mut app = app();
        app.rows[0].set_busy(activity, true);
        let out = screen(&app, 100, 12);
        let row = out.lines().find(|l| l.contains("backend-api")).unwrap();
        assert!(row.contains(word), "expected {word:?} in: {row}");
    }
}

/// The dirty summary is older news than the work running right now.
#[test]
fn running_work_outranks_the_dirty_summary() {
    let mut app = app();
    app.rows[2].set_busy(Activity::Pushing, true);
    let out = screen(&app, 100, 12);
    let row = out.lines().find(|l| l.contains("api-gateway")).unwrap();
    assert!(row.contains("pushing …"), "{row}");
    assert!(!row.contains("2 modified"), "{row}");
}

#[test]
fn a_busy_row_is_dimmed_and_an_idle_one_is_not() {
    let mut busy = app();
    busy.rows[0].set_busy(Activity::Pushing, true);
    assert_eq!(
        fg_of_row(&busy, "backend-api", 100, 12),
        Some(Color::DarkGray)
    );

    let idle = app();
    assert_ne!(
        fg_of_row(&idle, "backend-api", 100, 12),
        Some(Color::DarkGray)
    );
}

#[test]
fn the_spinner_frame_changes_between_ticks() {
    let mut app = app();
    app.rows[0].set_busy(Activity::Pushing, true);
    let first = screen(&app, 100, 12);
    app.tick();
    let second = screen(&app, 100, 12);
    assert_ne!(first, second, "the spinner should have advanced");
}

#[test]
fn the_help_line_offers_enter_for_gitui() {
    let out = screen(&app(), 120, 12);
    assert!(out.contains("Enter gitui"), "{out}");
}

fn commit(id: &str, summary: &str, files: &[(&str, Status)]) -> Commit {
    Commit {
        short_id: id.into(),
        full_id: format!("{id}0000000000000000000000000000000000"),
        summary: summary.into(),
        author: "Ada".into(),
        date: "2026-10-19".into(),
        files: files
            .iter()
            .map(|(path, status)| FileChange {
                status: *status,
                path: (*path).into(),
            })
            .collect(),
    }
}

fn with_panel(commits: Vec<Commit>, expanded: Vec<bool>) -> App {
    let mut app = app();
    app.panel = Some(Panel {
        repo: 0,
        commits,
        expanded,
        cursor: 0,
        loading: false,
        error: None,
    });
    app
}

#[test]
fn the_panel_lists_commits_with_their_files() {
    let app = with_panel(
        vec![commit(
            "a3f21c",
            "bump parent pom",
            &[("pom.xml", Status::Modified)],
        )],
        vec![true],
    );
    let out = screen(&app, 140, 14);
    assert!(out.contains("backend-api — 1 unpushed"), "{out}");
    assert!(out.contains("a3f21c"), "{out}");
    assert!(out.contains("bump parent pom"), "{out}");
    assert!(out.contains("M pom.xml"), "{out}");
}

/// A collapsed commit hides its files and says how many there are, so a
/// commit touching hundreds of them cannot bury the rest.
#[test]
fn a_collapsed_commit_hides_its_files_and_reports_the_count() {
    let app = with_panel(
        vec![commit(
            "7c4d88",
            "big change",
            &[("one.rs", Status::Modified), ("two.rs", Status::Added)],
        )],
        vec![false],
    );
    let out = screen(&app, 140, 14);
    assert!(out.contains("(2 files)"), "{out}");
    assert!(!out.contains("one.rs"), "{out}");
    assert!(out.contains("▸"), "{out}");
}

#[test]
fn space_folds_a_commit_open_and_shut() {
    let mut app = with_panel(
        vec![commit("a3f21c", "x", &[("pom.xml", Status::Modified)])],
        vec![false],
    );
    assert!(!screen(&app, 140, 14).contains("pom.xml"));
    app.on_key(Key::Char(' '));
    assert!(screen(&app, 140, 14).contains("pom.xml"));
    app.on_key(Key::Char(' '));
    assert!(!screen(&app, 140, 14).contains("pom.xml"));
}

/// Opening the panel must not cost the list its identity — both are on
/// screen. Names are truncated to fit the narrower column, which is the
/// accepted price of the split, so the check is on prefixes.
#[test]
fn the_list_stays_visible_beside_the_panel() {
    let app = with_panel(vec![commit("a3f21c", "x", &[])], vec![true]);
    let out = screen(&app, 140, 14);
    assert!(out.contains("backend-ap"), "{out}");
    assert!(out.contains("api-gatewa"), "{out}");
    assert!(
        out.contains("backend-api — 1 unpushed"),
        "panel missing: {out}"
    );
}

#[test]
fn a_repository_with_nothing_unpushed_says_so() {
    let app = with_panel(Vec::new(), Vec::new());
    assert!(screen(&app, 140, 14).contains("Nothing unpushed"));
}

#[test]
fn the_panel_shows_that_it_is_still_reading() {
    let mut app = app();
    app.panel = Some(Panel {
        repo: 0,
        commits: Vec::new(),
        expanded: Vec::new(),
        cursor: 0,
        loading: true,
        error: None,
    });
    assert!(screen(&app, 140, 14).contains("reading"));
}

fn with_diff(path: &str, lines: Vec<DiffLine>) -> App {
    let mut app = with_panel(
        vec![commit("a3f21c", "x", &[(path, Status::Modified)])],
        vec![true],
    );
    app.diff = Some(Diff {
        repo: 0,
        title: format!("backend-api · a3f21c · {path}"),
        path: path.into(),
        lines,
        scroll: 0,
        loading: false,
        error: None,
    });
    app
}

#[test]
fn the_diff_marks_added_and_removed_lines() {
    let app = with_diff(
        "main.rs",
        vec![
            DiffLine {
                kind: LineKind::Header,
                text: "@@ -1,2 +1,2 @@".into(),
            },
            DiffLine {
                kind: LineKind::Context,
                text: "fn main() {".into(),
            },
            DiffLine {
                kind: LineKind::Removed,
                text: "    let x = 1;".into(),
            },
            DiffLine {
                kind: LineKind::Added,
                text: "    let x = 2;".into(),
            },
        ],
    );
    let out = screen(&app, 140, 14);
    assert!(out.contains("backend-api · a3f21c · main.rs"), "{out}");
    assert!(out.contains("@@ -1,2 +1,2 @@"), "{out}");
    assert!(
        out.lines()
            .any(|l| l.trim_start().starts_with("-    let x = 1;")),
        "{out}"
    );
    assert!(
        out.lines()
            .any(|l| l.trim_start().starts_with("+    let x = 2;")),
        "{out}"
    );
}

/// The diff needs the width; half a terminal makes it unreadable.
#[test]
fn the_diff_takes_the_whole_width() {
    let app = with_diff(
        "main.rs",
        vec![DiffLine {
            kind: LineKind::Context,
            text: "fn main() {}".into(),
        }],
    );
    let out = screen(&app, 140, 14);
    assert!(
        !out.contains("backend-api — 1 unpushed"),
        "panel still drawn: {out}"
    );
    assert!(!out.contains("Branch"), "list still drawn: {out}");
}

#[test]
fn scrolling_moves_through_the_diff() {
    let lines: Vec<DiffLine> = (0..60)
        .map(|n| DiffLine {
            kind: LineKind::Context,
            text: format!("line{n}"),
        })
        .collect();
    let mut app = with_diff("main.rs", lines);
    assert!(screen(&app, 140, 14).contains("line0"));
    app.on_key(Key::PageDown);
    let out = screen(&app, 140, 14);
    assert!(!out.contains("line0"), "{out}");
    app.on_key(Key::Home);
    assert!(screen(&app, 140, 14).contains("line0"));
}

#[test]
fn an_empty_diff_explains_itself() {
    let app = with_diff("logo.png", Vec::new());
    assert!(screen(&app, 140, 14).contains("binary"));
}

#[test]
fn the_help_line_follows_the_focus() {
    assert!(screen(&app(), 150, 14).contains("→ commits"));

    let panel = with_panel(vec![commit("a", "x", &[])], vec![true]);
    assert!(screen(&panel, 150, 14).contains("Space fold"));

    let diff = with_diff(
        "main.rs",
        vec![DiffLine {
            kind: LineKind::Context,
            text: "x".into(),
        }],
    );
    assert!(screen(&diff, 150, 14).contains("PgUp/PgDn"));
}
