//! Rendering tests driving the real widgets through ratatui's test backend.
//!
//! These exist because the interesting rules of the interface are visual: what
//! the header accounts for, whether a failed scan can hide, and whether a bulk
//! push names what it would hit. Compiling is not evidence of any of that.

use ratatui::backend::TestBackend;
use ratatui::Terminal;

use unpushed::app::{App, JobResult, Key};
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
