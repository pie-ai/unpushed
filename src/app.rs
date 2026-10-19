//! Application state and key handling.
//!
//! Deliberately free of terminal and process code: everything here is a pure
//! transition from state plus input to state plus a requested [`Action`]. The
//! runner in [`crate::tui`] is what actually touches the terminal and spawns
//! work, which keeps the interesting rules — what is visible, what a key does,
//! what a bulk push would hit — testable without a tty.

use crate::git::PushOutcome;
use crate::model::{Head, Repo, ScanOutcome, Summary};

/// Which repositories the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Default: unpushed or dirty, plus every failed scan.
    NeedsAttention,
    All,
}

/// What a row is currently busy with, so the UI can show it and key handling
/// can refuse to start the same work twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Scanning,
    Fetching,
    Pushing,
}

/// Background work the runner should start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    Scan(usize),
    Fetch(usize),
    Push { index: usize, branch: String },
}

/// Result of background work, fed back into the state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobResult {
    Scanned {
        index: usize,
        outcome: Box<ScanOutcome>,
    },
    Fetched {
        index: usize,
        result: Result<(), String>,
    },
    Pushed {
        index: usize,
        outcome: PushOutcome,
    },
}

/// What the runner should do after a key was handled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    Start(Vec<Job>),
    /// Hand the terminal to gitui for this repository.
    OpenGitui(usize),
}

/// Modal state. Only one of these can be active at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// Typing a filter. The previous filter is kept so Esc can restore it.
    Filter {
        draft: String,
        previous: String,
    },
    Confirm(Confirm),
}

/// A pending bulk action waiting for confirmation. Bulk pushes are the one
/// place where two keys with different reach (`A` visible, `P` marked) could be
/// confused, so the prompt names the repositories rather than just the count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    pub prompt: String,
    pub targets: Vec<String>,
    pub jobs: Vec<Job>,
}

/// Severity of the status line message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub level: Level,
    pub text: String,
}

/// One repository in the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub repo: Repo,
    pub outcome: Option<ScanOutcome>,
    pub marked: bool,
    pub activity: Activity,
    /// Result of the last action on this row, shown until something replaces it.
    pub note: Option<Message>,
    /// Whether this repository has been fetched during this session.
    pub fetched: bool,
}

impl Row {
    fn new(repo: Repo) -> Self {
        Row {
            repo,
            outcome: None,
            marked: false,
            activity: Activity::Scanning,
            note: None,
            fetched: false,
        }
    }

    /// Branch that a push would target, or `None` when pushing makes no sense.
    pub fn push_branch(&self) -> Option<String> {
        match self.outcome.as_ref()?.status()?.head.clone() {
            Head::Branch(name) => Some(name),
            Head::Detached(_) | Head::Unborn(_) => None,
        }
    }

    fn needs_attention(&self) -> bool {
        match &self.outcome {
            None => true,
            Some(ScanOutcome::Failed(_)) => true,
            Some(ScanOutcome::Ok(status)) => status.needs_attention(),
        }
    }

    fn branch_label(&self) -> String {
        match &self.outcome {
            Some(ScanOutcome::Ok(status)) => status.head.label(),
            _ => String::new(),
        }
    }
}

pub struct App {
    pub rows: Vec<Row>,
    /// Cursor position within the *visible* rows.
    pub cursor: usize,
    pub view: View,
    pub filter: String,
    pub mode: Mode,
    pub message: Option<Message>,
    pub base: String,
    /// Number of background fetches still running, for the header.
    pub fetching: usize,
}

impl App {
    pub fn new(base: String, repos: Vec<Repo>) -> Self {
        App {
            rows: repos.into_iter().map(Row::new).collect(),
            cursor: 0,
            view: View::NeedsAttention,
            filter: String::new(),
            mode: Mode::Normal,
            message: None,
            base,
            fetching: 0,
        }
    }

    /// Jobs to run at startup: scan everything, then fetch everything.
    ///
    /// The scan is local and finishes in well under a second; the fetch is
    /// network-bound and merely updates rows as results arrive, so the list is
    /// usable immediately.
    pub fn startup_jobs(&mut self) -> Vec<Job> {
        let mut jobs: Vec<Job> = (0..self.rows.len()).map(Job::Scan).collect();
        jobs.extend((0..self.rows.len()).map(Job::Fetch));
        self.begin(&jobs);
        jobs
    }

    /// Indices of the rows the list currently shows.
    pub fn visible(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| match self.view {
                View::All => true,
                View::NeedsAttention => row.needs_attention(),
            })
            .filter(|(_, row)| self.matches_filter(row))
            .map(|(index, _)| index)
            .collect()
    }

    fn matches_filter(&self, row: &Row) -> bool {
        if self.filter.is_empty() {
            return true;
        }
        let needle = self.filter.to_lowercase();
        row.branch_label().to_lowercase().contains(&needle)
            || row.repo.name.to_lowercase().contains(&needle)
    }

    /// Row the cursor points at, if any.
    pub fn selected(&self) -> Option<usize> {
        self.visible().get(self.cursor).copied()
    }

    pub fn summary(&self) -> Summary {
        let outcomes: Vec<ScanOutcome> =
            self.rows.iter().filter_map(|r| r.outcome.clone()).collect();
        let mut summary = Summary::of(&outcomes);
        // Rows still being scanned are neither clean nor failed; counting them
        // as clean would understate the work in progress.
        let pending = self.rows.len() - outcomes.len();
        summary.with_work += pending;
        summary
    }

    /// Feeds the result of background work back into the state.
    pub fn apply(&mut self, result: JobResult) {
        match result {
            JobResult::Scanned { index, outcome } => {
                if let Some(row) = self.rows.get_mut(index) {
                    row.outcome = Some(*outcome);
                    row.activity = Activity::Idle;
                }
            }
            JobResult::Fetched { index, result } => {
                self.fetching = self.fetching.saturating_sub(1);
                if let Some(row) = self.rows.get_mut(index) {
                    row.activity = Activity::Idle;
                    match result {
                        Ok(()) => {
                            row.fetched = true;
                            row.note = None;
                        }
                        Err(err) => {
                            row.note = Some(Message {
                                level: Level::Warn,
                                text: format!("fetch: {err}"),
                            })
                        }
                    }
                }
            }
            JobResult::Pushed { index, outcome } => {
                if let Some(row) = self.rows.get_mut(index) {
                    row.activity = Activity::Idle;
                    row.marked = false;
                    row.note = Some(describe_push(&outcome));
                }
            }
        }
        self.clamp_cursor();
    }

    /// Marks a row as busy so the UI shows it and a second keypress is ignored.
    pub fn mark_busy(&mut self, job: &Job) {
        let (index, activity) = match job {
            Job::Scan(index) => (*index, Activity::Scanning),
            Job::Fetch(index) => (*index, Activity::Fetching),
            Job::Push { index, .. } => (*index, Activity::Pushing),
        };
        if let Some(row) = self.rows.get_mut(index) {
            row.activity = activity;
        }
    }

    pub fn on_key(&mut self, key: Key) -> Action {
        self.message = None;
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Filter { draft, previous } => self.key_in_filter(key, draft, previous),
            Mode::Confirm(confirm) => self.key_in_confirm(key, confirm),
            Mode::Normal => self.key_in_normal(key),
        }
    }

    fn key_in_filter(&mut self, key: Key, mut draft: String, previous: String) -> Action {
        match key {
            Key::Enter => {
                self.filter = draft;
                self.clamp_cursor();
            }
            Key::Esc => {
                self.filter = previous;
                self.clamp_cursor();
            }
            Key::Backspace => {
                draft.pop();
                self.filter = draft.clone();
                self.clamp_cursor();
                self.mode = Mode::Filter { draft, previous };
            }
            Key::Char(c) => {
                draft.push(c);
                // Filtering live makes the effect of each character visible,
                // which matters because the filter decides what `A` would push.
                self.filter = draft.clone();
                self.clamp_cursor();
                self.mode = Mode::Filter { draft, previous };
            }
            _ => self.mode = Mode::Filter { draft, previous },
        }
        Action::None
    }

    fn key_in_confirm(&mut self, key: Key, confirm: Confirm) -> Action {
        match key {
            Key::Char('y') | Key::Char('j') | Key::Enter => {
                self.begin(&confirm.jobs);
                Action::Start(confirm.jobs)
            }
            Key::Char('n') | Key::Esc | Key::Char('q') => {
                self.message = Some(Message {
                    level: Level::Info,
                    text: "Cancelled.".into(),
                });
                Action::None
            }
            _ => {
                self.mode = Mode::Confirm(confirm);
                Action::None
            }
        }
    }

    fn key_in_normal(&mut self, key: Key) -> Action {
        match key {
            Key::Char('q') => Action::Quit,
            Key::Down | Key::Char('j') => {
                self.move_cursor(1);
                Action::None
            }
            Key::Up | Key::Char('k') => {
                self.move_cursor(-1);
                Action::None
            }
            Key::Home => {
                self.cursor = 0;
                Action::None
            }
            Key::End => {
                self.cursor = self.visible().len().saturating_sub(1);
                Action::None
            }
            Key::Char(' ') => {
                if let Some(index) = self.selected() {
                    self.rows[index].marked = !self.rows[index].marked;
                    self.move_cursor(1);
                }
                Action::None
            }
            Key::Char('a') => {
                self.view = match self.view {
                    View::NeedsAttention => View::All,
                    View::All => View::NeedsAttention,
                };
                self.clamp_cursor();
                Action::None
            }
            Key::Char('/') => {
                self.mode = Mode::Filter {
                    draft: self.filter.clone(),
                    previous: self.filter.clone(),
                };
                Action::None
            }
            Key::Esc => {
                if !self.filter.is_empty() {
                    self.filter.clear();
                    self.clamp_cursor();
                }
                Action::None
            }
            Key::Char('r') => self.start_on_selected(Job::Scan),
            Key::Char('R') => {
                let jobs: Vec<Job> = self.visible().into_iter().map(Job::Scan).collect();
                self.start(jobs)
            }
            Key::Char('f') => self.start_on_selected(Job::Fetch),
            Key::Char('F') => {
                let jobs: Vec<Job> = self.visible().into_iter().map(Job::Fetch).collect();
                self.start(jobs)
            }
            Key::Char('g') => match self.selected() {
                Some(index) => Action::OpenGitui(index),
                None => self.warn("No row selected."),
            },
            // A single push is immediate: it affects one named repository the
            // cursor is on. Bulk pushes are confirmed instead.
            Key::Char('p') => match self.selected() {
                Some(index) => match self.push_job(index) {
                    Ok(job) => self.start(vec![job]),
                    Err(reason) => self.warn(&reason),
                },
                None => self.warn("No row selected."),
            },
            Key::Char('A') => {
                let targets = self.visible();
                self.confirm_push(targets, "visible")
            }
            Key::Char('P') => {
                let targets: Vec<usize> = self
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, row)| row.marked)
                    .map(|(index, _)| index)
                    .collect();
                self.confirm_push(targets, "marked")
            }
            _ => Action::None,
        }
    }

    fn push_job(&self, index: usize) -> Result<Job, String> {
        let row = self
            .rows
            .get(index)
            .ok_or_else(|| "Row does not exist.".to_string())?;

        if row.activity != Activity::Idle {
            return Err(format!("{} is busy.", row.repo.name));
        }
        match &row.outcome {
            None => Err(format!("{} has not been read yet.", row.repo.name)),
            Some(ScanOutcome::Failed(_)) => Err(format!("{}: scan failed.", row.repo.name)),
            Some(ScanOutcome::Ok(_)) => row
                .push_branch()
                .map(|branch| Job::Push { index, branch })
                .ok_or_else(|| format!("{}: no branch (detached HEAD).", row.repo.name)),
        }
    }

    /// Builds a confirmation for a bulk push, listing what it would hit.
    fn confirm_push(&mut self, targets: Vec<usize>, what: &str) -> Action {
        let mut jobs = Vec::new();
        let mut names = Vec::new();
        let mut skipped = 0usize;

        for index in targets {
            match self.push_job(index) {
                Ok(job) => {
                    names.push(self.rows[index].repo.name.clone());
                    jobs.push(job);
                }
                Err(_) => skipped += 1,
            }
        }

        if jobs.is_empty() {
            return self.warn(&format!("No {what} repositories can be pushed."));
        }

        let mut prompt = format!("Push {} {what} repositories?", jobs.len());
        if skipped > 0 {
            prompt.push_str(&format!(" ({skipped} skipped)"));
        }
        self.mode = Mode::Confirm(Confirm {
            prompt,
            targets: names,
            jobs,
        });
        Action::None
    }

    fn start_on_selected(&mut self, make: fn(usize) -> Job) -> Action {
        match self.selected() {
            Some(index) => self.start(vec![make(index)]),
            None => self.warn("No row selected."),
        }
    }

    fn start(&mut self, jobs: Vec<Job>) -> Action {
        if jobs.is_empty() {
            return Action::None;
        }
        self.begin(&jobs);
        Action::Start(jobs)
    }

    /// Single place where work is booked: marks rows busy and keeps the
    /// outstanding-fetch counter in step, so no caller can forget either half.
    fn begin(&mut self, jobs: &[Job]) {
        for job in jobs {
            if matches!(job, Job::Fetch(_)) {
                self.fetching += 1;
            }
            self.mark_busy(job);
        }
    }

    fn warn(&mut self, text: &str) -> Action {
        self.message = Some(Message {
            level: Level::Warn,
            text: text.to_string(),
        });
        Action::None
    }

    fn move_cursor(&mut self, delta: isize) {
        let len = self.visible().len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        let next = self.cursor as isize + delta;
        self.cursor = next.clamp(0, len as isize - 1) as usize;
    }

    fn clamp_cursor(&mut self) {
        let len = self.visible().len();
        self.cursor = self.cursor.min(len.saturating_sub(1));
    }
}

fn describe_push(outcome: &PushOutcome) -> Message {
    match outcome {
        PushOutcome::Pushed { created: true } => Message {
            level: Level::Info,
            text: "pushed, created branch on origin".into(),
        },
        PushOutcome::Pushed { created: false } => Message {
            level: Level::Info,
            text: "pushed".into(),
        },
        PushOutcome::UpToDate => Message {
            level: Level::Info,
            text: "already up to date".into(),
        },
        PushOutcome::Refused(reason) => Message {
            level: Level::Warn,
            text: reason.clone(),
        },
        PushOutcome::Failed(err) => Message {
            level: Level::Error,
            text: err.clone(),
        },
    }
}

/// Keys the application reacts to, kept independent of crossterm so the state
/// machine can be driven from tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Up,
    Down,
    Home,
    End,
    Enter,
    Esc,
    Backspace,
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Divergence, MainState, RepoStatus, ScanError, UpstreamState, Worktree};
    use std::path::PathBuf;

    fn repo(name: &str) -> Repo {
        Repo {
            name: name.into(),
            path: PathBuf::from("/tmp").join(name),
        }
    }

    fn status(name: &str, branch: &str, ahead: usize, dirty: bool) -> ScanOutcome {
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
                    unstaged: 1,
                    ..Worktree::default()
                }
            } else {
                Worktree::default()
            },
        }))
    }

    /// Three repositories: one with work, one quiet, one dirty.
    fn app() -> App {
        let mut app = App::new(
            "/tmp".into(),
            vec![
                repo("backend-api"),
                repo("web-frontend"),
                repo("api-gateway"),
            ],
        );
        app.apply(JobResult::Scanned {
            index: 0,
            outcome: Box::new(status("backend-api", "feature/login", 3, false)),
        });
        app.apply(JobResult::Scanned {
            index: 1,
            outcome: Box::new(status("web-frontend", "main", 0, false)),
        });
        app.apply(JobResult::Scanned {
            index: 2,
            outcome: Box::new(status("api-gateway", "main", 0, true)),
        });
        app
    }

    #[test]
    fn default_view_hides_quiet_rows() {
        let app = app();
        assert_eq!(app.visible(), vec![0, 2]);
    }

    #[test]
    fn toggling_the_view_shows_everything() {
        let mut app = app();
        app.on_key(Key::Char('a'));
        assert_eq!(app.visible(), vec![0, 1, 2]);
    }

    #[test]
    fn unscanned_rows_stay_visible_and_are_not_counted_as_clean() {
        let mut app = App::new("/tmp".into(), vec![repo("a"), repo("b")]);
        assert_eq!(app.visible(), vec![0, 1]);
        assert_eq!(app.summary().clean, 0);
        assert_eq!(app.summary().with_work, 2);

        app.apply(JobResult::Scanned {
            index: 0,
            outcome: Box::new(status("a", "main", 0, false)),
        });
        assert_eq!(app.summary().clean, 1);
    }

    /// A failed scan is not a clean repository and must survive the filter.
    #[test]
    fn failed_scans_stay_visible() {
        let mut app = app();
        app.apply(JobResult::Scanned {
            index: 1,
            outcome: Box::new(ScanOutcome::Failed(ScanError {
                repo: repo("web-frontend"),
                message: "kaputt".into(),
            })),
        });
        assert!(app.visible().contains(&1));
        assert_eq!(app.summary().failed, 1);
    }

    #[test]
    fn filter_narrows_by_branch_and_by_name() {
        let mut app = app();
        app.on_key(Key::Char('a')); // show all
        app.on_key(Key::Char('/'));
        for c in "feature/login".chars() {
            app.on_key(Key::Char(c));
        }
        app.on_key(Key::Enter);
        assert_eq!(app.visible(), vec![0]);

        app.on_key(Key::Esc);
        assert_eq!(app.visible(), vec![0, 1, 2]);
    }

    #[test]
    fn escape_during_filtering_restores_the_previous_filter() {
        let mut app = app();
        app.on_key(Key::Char('a'));
        app.on_key(Key::Char('/'));
        app.on_key(Key::Char('v'));
        app.on_key(Key::Enter);
        assert_eq!(app.filter, "v");

        app.on_key(Key::Char('/'));
        app.on_key(Key::Char('x'));
        app.on_key(Key::Esc);
        assert_eq!(app.filter, "v");
    }

    #[test]
    fn cursor_stays_inside_the_visible_rows() {
        let mut app = app();
        app.on_key(Key::End);
        assert_eq!(app.cursor, 1);
        // Switching to the narrower view must not leave the cursor dangling.
        app.on_key(Key::Char('/'));
        for c in "backend-api".chars() {
            app.on_key(Key::Char(c));
        }
        app.on_key(Key::Enter);
        assert_eq!(app.visible().len(), 1);
        assert_eq!(app.cursor, 0);
        assert_eq!(app.selected(), Some(0));
    }

    #[test]
    fn single_push_starts_immediately_without_confirmation() {
        let mut app = app();
        let action = app.on_key(Key::Char('p'));
        assert_eq!(
            action,
            Action::Start(vec![Job::Push {
                index: 0,
                branch: "feature/login".into()
            }])
        );
        assert_eq!(app.rows[0].activity, Activity::Pushing);
    }

    #[test]
    fn bulk_push_asks_first_and_names_its_targets() {
        let mut app = app();
        let action = app.on_key(Key::Char('A'));
        assert_eq!(action, Action::None);
        match &app.mode {
            Mode::Confirm(confirm) => {
                assert_eq!(confirm.targets, vec!["backend-api", "api-gateway"]);
                assert!(confirm.prompt.contains("visible"));
            }
            other => panic!("expected Confirm, got {other:?}"),
        }

        let action = app.on_key(Key::Char('y'));
        match action {
            Action::Start(jobs) => assert_eq!(jobs.len(), 2),
            other => panic!("expected Start, got {other:?}"),
        }
    }

    /// `A` and `P` have different reach; the distinction has to hold.
    #[test]
    fn marked_push_only_hits_marked_rows() {
        let mut app = app();
        app.on_key(Key::Char(' ')); // mark backend-api, cursor moves on
        let action = app.on_key(Key::Char('P'));
        assert_eq!(action, Action::None);
        match &app.mode {
            Mode::Confirm(confirm) => {
                assert_eq!(confirm.targets, vec!["backend-api"]);
                assert!(confirm.prompt.contains("marked"));
            }
            other => panic!("expected Confirm, got {other:?}"),
        }
    }

    #[test]
    fn declining_a_bulk_push_starts_nothing() {
        let mut app = app();
        app.on_key(Key::Char('A'));
        let action = app.on_key(Key::Char('n'));
        assert_eq!(action, Action::None);
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.rows.iter().all(|r| r.activity == Activity::Idle));
    }

    #[test]
    fn push_is_refused_on_a_detached_head() {
        let mut app = app();
        app.apply(JobResult::Scanned {
            index: 0,
            outcome: Box::new(ScanOutcome::Ok(Box::new(RepoStatus {
                repo: repo("backend-api"),
                head: Head::Detached("abc1234".into()),
                upstream: UpstreamState::NotSet,
                main: MainState::Compared {
                    name: "origin/main".into(),
                    divergence: Divergence {
                        ahead: 2,
                        behind: 0,
                    },
                },
                worktree: Worktree::default(),
            }))),
        });
        let action = app.on_key(Key::Char('p'));
        assert_eq!(action, Action::None);
        assert!(app.message.as_ref().unwrap().text.contains("detached HEAD"));
    }

    #[test]
    fn push_is_refused_while_the_row_is_busy() {
        let mut app = app();
        app.on_key(Key::Char('p'));
        let action = app.on_key(Key::Char('p'));
        assert_eq!(action, Action::None);
        assert!(app.message.as_ref().unwrap().text.contains("is busy"));
    }

    #[test]
    fn push_result_is_recorded_on_the_row_and_clears_the_mark() {
        let mut app = app();
        app.rows[0].marked = true;
        app.apply(JobResult::Pushed {
            index: 0,
            outcome: PushOutcome::Pushed { created: true },
        });
        assert!(!app.rows[0].marked);
        assert_eq!(app.rows[0].activity, Activity::Idle);
        assert!(app.rows[0]
            .note
            .as_ref()
            .unwrap()
            .text
            .contains("created branch"));
    }

    #[test]
    fn refused_push_is_shown_as_a_warning_not_an_error() {
        let mut app = app();
        app.apply(JobResult::Pushed {
            index: 0,
            outcome: PushOutcome::Refused("behind origin/main by 4 commit(s)".into()),
        });
        assert_eq!(app.rows[0].note.as_ref().unwrap().level, Level::Warn);
    }

    #[test]
    fn startup_schedules_a_scan_and_a_fetch_for_every_repository() {
        let mut app = App::new("/tmp".into(), vec![repo("a"), repo("b")]);
        let jobs = app.startup_jobs();
        assert_eq!(jobs.len(), 4);
        assert_eq!(app.fetching, 2);

        app.apply(JobResult::Fetched {
            index: 0,
            result: Ok(()),
        });
        assert_eq!(app.fetching, 1);
        assert!(app.rows[0].fetched);
    }

    #[test]
    fn a_failed_fetch_is_noted_on_the_row() {
        let mut app = app();
        app.apply(JobResult::Fetched {
            index: 0,
            result: Err("Network is unreachable".into()),
        });
        assert!(!app.rows[0].fetched);
        assert_eq!(app.rows[0].note.as_ref().unwrap().level, Level::Warn);
    }

    #[test]
    fn gitui_is_requested_for_the_selected_row() {
        let mut app = app();
        assert_eq!(app.on_key(Key::Char('g')), Action::OpenGitui(0));
    }

    #[test]
    fn quit_is_reported() {
        let mut app = app();
        assert_eq!(app.on_key(Key::Char('q')), Action::Quit);
    }

    #[test]
    fn keys_do_nothing_when_the_list_is_empty() {
        let mut app = App::new("/tmp".into(), Vec::new());
        assert_eq!(app.on_key(Key::Char('p')), Action::None);
        assert_eq!(app.on_key(Key::Char('g')), Action::None);
        assert_eq!(app.on_key(Key::Down), Action::None);
        assert_eq!(app.selected(), None);
    }
}
