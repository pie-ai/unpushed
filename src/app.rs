//! Application state and key handling.
//!
//! Deliberately free of terminal and process code: everything here is a pure
//! transition from state plus input to state plus a requested [`Action`]. The
//! runner in [`crate::tui`] is what actually touches the terminal and spawns
//! work, which keeps the interesting rules — what is visible, what a key does,
//! what a bulk push would hit — testable without a tty.

use crate::commits::{Commit, DiffLine};
use crate::git::PushOutcome;
use crate::model::{Head, Repo, ScanOutcome, Summary};

/// Which repositories the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Default: unpushed or dirty, plus every failed scan.
    NeedsAttention,
    All,
}

/// Which kinds of work are running on a row. Several can be true at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Busy {
    scanning: bool,
    fetching: bool,
    pushing: bool,
}

/// What a row is currently busy with, as shown to the user. Derived from
/// [`Busy`] by picking the most consequential of the running kinds.
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
    Push {
        index: usize,
        branch: String,
    },
    /// Unpushed commits of one repository, for the panel.
    Commits(usize),
    /// One file's diff within one commit.
    Diff {
        index: usize,
        commit: String,
        file: String,
    },
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
    Commits {
        index: usize,
        result: Result<Vec<Commit>, String>,
    },
    Diff {
        index: usize,
        result: Result<Vec<DiffLine>, String>,
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

/// One line of the commit tree, flattened for cursor movement and drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Commit(usize),
    /// Commit index, then file index within that commit.
    File(usize, usize),
}

/// Commits with few files are opened straight away — in a survey of 82
/// unpushed commits, 59 touched exactly one file, and making those cost a
/// keypress would be pure friction. Larger ones stay shut so a commit touching
/// hundreds of files cannot bury the rest.
pub const AUTO_EXPAND_UP_TO: usize = 8;

/// The commit panel for one repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Panel {
    pub repo: usize,
    pub commits: Vec<Commit>,
    pub expanded: Vec<bool>,
    /// Position within [`Panel::items`].
    pub cursor: usize,
    pub loading: bool,
    pub error: Option<String>,
}

impl Panel {
    fn loading(repo: usize) -> Self {
        Panel {
            repo,
            commits: Vec::new(),
            expanded: Vec::new(),
            cursor: 0,
            loading: true,
            error: None,
        }
    }

    /// The tree as a flat list, which is what the cursor moves through and
    /// what the renderer draws.
    pub fn items(&self) -> Vec<Item> {
        let mut items = Vec::new();
        for (c, commit) in self.commits.iter().enumerate() {
            items.push(Item::Commit(c));
            if self.expanded.get(c).copied().unwrap_or(false) {
                for f in 0..commit.files.len() {
                    items.push(Item::File(c, f));
                }
            }
        }
        items
    }

    pub fn selected(&self) -> Option<Item> {
        self.items().get(self.cursor).copied()
    }

    fn toggle(&mut self, commit: usize) {
        if let Some(flag) = self.expanded.get_mut(commit) {
            *flag = !*flag;
        }
        self.clamp();
    }

    fn move_cursor(&mut self, delta: isize) {
        let len = self.items().len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        self.cursor = (self.cursor as isize + delta).clamp(0, len as isize - 1) as usize;
    }

    fn clamp(&mut self) {
        self.cursor = self.cursor.min(self.items().len().saturating_sub(1));
    }
}

/// A single file's diff, shown over the whole width.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diff {
    pub repo: usize,
    pub title: String,
    /// Kept for picking the syntax definition.
    pub path: String,
    pub lines: Vec<DiffLine>,
    pub scroll: usize,
    pub loading: bool,
    pub error: Option<String>,
}

/// Which part of the interface the keys apply to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    List,
    Panel,
    Diff,
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
    /// One flag per kind of work, because a row can be doing several at once:
    /// startup schedules a scan *and* a fetch for every repository. A single
    /// field would let the first result to land clear the other one, and the
    /// row would look idle while a fetch was still running.
    busy: Busy,
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
            busy: Busy::default(),
            note: None,
            fetched: false,
        }
    }

    /// The kind of work shown for this row. Pushing outranks fetching, which
    /// outranks reading: the user cares most about the one with consequences.
    pub fn activity(&self) -> Activity {
        if self.busy.pushing {
            Activity::Pushing
        } else if self.busy.fetching {
            Activity::Fetching
        } else if self.busy.scanning {
            Activity::Scanning
        } else {
            Activity::Idle
        }
    }

    /// Marks a kind of work as running or finished. Public so integration
    /// tests can put a row into a given state without driving real jobs.
    pub fn set_busy(&mut self, activity: Activity, running: bool) {
        match activity {
            Activity::Scanning => self.busy.scanning = running,
            Activity::Fetching => self.busy.fetching = running,
            Activity::Pushing => self.busy.pushing = running,
            Activity::Idle => self.busy = Busy::default(),
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
    /// Advances on a timer so busy rows can animate. Nothing else depends on
    /// it, so wrapping is harmless.
    tick: u64,
    /// Commit tree for one repository, when opened.
    pub panel: Option<Panel>,
    /// A file's diff, on top of the panel.
    pub diff: Option<Diff>,
}

/// Frames for the busy indicator. Ten of them at the 100 ms tick make one
/// turn per second — fast enough to read as "working", slow enough not to
/// flicker.
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

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
            tick: 0,
            panel: None,
            diff: None,
        }
    }

    /// Where the keys go. Derived rather than stored, so it cannot drift from
    /// what is actually on screen.
    pub fn focus(&self) -> Focus {
        if self.diff.is_some() {
            Focus::Diff
        } else if self.panel.is_some() {
            Focus::Panel
        } else {
            Focus::List
        }
    }

    /// Advances the busy animation. Driven by the event loop on a timer rather
    /// than per redraw, so holding a key down does not freeze the spinner.
    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /// Current frame of the busy indicator.
    pub fn spinner(&self) -> char {
        SPINNER[(self.tick % SPINNER.len() as u64) as usize]
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

    /// Moves the cursor back onto `anchor` after the visible set changed.
    ///
    /// Without this a background refresh could slide a row out from under the
    /// cursor — you line up a push, a repository two lines above turns clean
    /// and disappears, and the key lands on a different repository.
    fn restore_cursor(&mut self, anchor: Option<usize>) {
        if let Some(anchor) = anchor {
            if let Some(position) = self.visible().iter().position(|&index| index == anchor) {
                self.cursor = position;
                return;
            }
        }
        self.clamp_cursor();
    }

    /// Feeds the result of background work back into the state.
    pub fn apply(&mut self, result: JobResult) {
        let anchor = self.selected();
        match result {
            JobResult::Scanned { index, outcome } => {
                if let Some(row) = self.rows.get_mut(index) {
                    row.outcome = Some(*outcome);
                    row.set_busy(Activity::Scanning, false);
                }
            }
            JobResult::Fetched { index, result } => {
                self.fetching = self.fetching.saturating_sub(1);
                if let Some(row) = self.rows.get_mut(index) {
                    row.set_busy(Activity::Fetching, false);
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
            JobResult::Commits { index, result } => {
                // Only accept it if the panel still shows the repository that
                // was asked for: the cursor may have moved on while it loaded.
                if let Some(panel) = self.panel.as_mut().filter(|p| p.repo == index) {
                    panel.loading = false;
                    match result {
                        Ok(commits) => {
                            panel.expanded = commits
                                .iter()
                                .map(|c| c.files.len() <= AUTO_EXPAND_UP_TO)
                                .collect();
                            panel.commits = commits;
                            panel.clamp();
                        }
                        Err(err) => panel.error = Some(err),
                    }
                }
            }
            JobResult::Diff { index, result } => {
                if let Some(diff) = self.diff.as_mut().filter(|d| d.repo == index) {
                    diff.loading = false;
                    match result {
                        Ok(lines) => diff.lines = lines,
                        Err(err) => diff.error = Some(err),
                    }
                }
            }
            JobResult::Pushed { index, outcome } => {
                if let Some(row) = self.rows.get_mut(index) {
                    row.set_busy(Activity::Pushing, false);
                    row.marked = false;
                    row.note = Some(describe_push(&outcome));
                }
            }
        }
        self.restore_cursor(anchor);
    }

    /// Marks a row as busy so the UI shows it and a second keypress is ignored.
    pub fn mark_busy(&mut self, job: &Job) {
        let (index, activity) = match job {
            Job::Scan(index) => (*index, Activity::Scanning),
            Job::Fetch(index) => (*index, Activity::Fetching),
            Job::Push { index, .. } => (*index, Activity::Pushing),
            // Reading commits or a diff is read-only and takes milliseconds.
            // Marking the row busy would flash a spinner and lock keys for no
            // reason; the panel shows its own loading state.
            Job::Commits(_) | Job::Diff { .. } => return,
        };
        if let Some(row) = self.rows.get_mut(index) {
            row.set_busy(activity, true);
        }
    }

    pub fn on_key(&mut self, key: Key) -> Action {
        self.message = None;
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Filter { draft, previous } => self.key_in_filter(key, draft, previous),
            Mode::Confirm(confirm) => self.key_in_confirm(key, confirm),
            Mode::Normal => match self.focus() {
                Focus::Diff => self.key_in_diff(key),
                Focus::Panel => self.key_in_panel(key),
                Focus::List => self.key_in_normal(key),
            },
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
            // Deliberately only `y`. Enter is the main action key in the list,
            // so accepting it here would turn a habitual keypress into consent
            // for a bulk push. `j` is "down" everywhere else.
            Key::Char('y') => {
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
            Key::Char('R') => self.start_bulk(Job::Scan, "read"),
            Key::Char('f') => self.start_on_selected(Job::Fetch),
            Key::Char('F') => self.start_bulk(Job::Fetch, "fetch"),
            // Enter is the primary way in — it matches the muscle memory of
            // drilling into a repository in gitui itself. `g` stays as a
            // second binding.
            Key::Enter | Key::Char('g') => match self.selected() {
                Some(index) => self.open_gitui(index),
                None => self.warn("No row selected."),
            },
            // Rightwards drills in, the same direction as everywhere else.
            Key::Right | Key::Char('l') => match self.selected() {
                Some(index) => self.open_panel(index),
                None => self.warn("No row selected."),
            },
            // A single push is immediate: it affects one named repository the
            // cursor is on. Bulk pushes are confirmed instead.
            Key::Char('p') => match self.selected() {
                Some(index) => self.push_row(index),
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

    /// Refuses anything that touches a repository while work is already running
    /// on it. A fetch during a push would move the refs out from under it, and
    /// the outcome of that is not worth reasoning about.
    fn require_idle(&self, index: usize) -> Result<(), String> {
        let row = self
            .rows
            .get(index)
            .ok_or_else(|| "Row does not exist.".to_string())?;
        match row.activity() {
            Activity::Idle => Ok(()),
            Activity::Scanning => Err(format!("{} is being read.", row.repo.name)),
            Activity::Fetching => Err(format!("{} is fetching.", row.repo.name)),
            Activity::Pushing => Err(format!("{} is pushing.", row.repo.name)),
        }
    }

    fn open_gitui(&mut self, index: usize) -> Action {
        match self.require_idle(index) {
            Ok(()) => Action::OpenGitui(index),
            Err(reason) => self.warn(&reason),
        }
    }

    fn push_row(&mut self, index: usize) -> Action {
        match self.push_job(index) {
            Ok(job) => self.start(vec![job]),
            Err(reason) => self.warn(&reason),
        }
    }

    fn job_on_row(&mut self, index: usize, make: fn(usize) -> Job) -> Action {
        match self.require_idle(index) {
            Ok(()) => self.start(vec![make(index)]),
            Err(reason) => self.warn(&reason),
        }
    }

    /// Opens the commit tree for a repository and asks for its commits.
    fn open_panel(&mut self, index: usize) -> Action {
        let Some(row) = self.rows.get(index) else {
            return Action::None;
        };
        if matches!(row.outcome, Some(ScanOutcome::Failed(_))) {
            return self.warn(&format!("{}: scan failed.", row.repo.name));
        }
        self.panel = Some(Panel::loading(index));
        Action::Start(vec![Job::Commits(index)])
    }

    fn key_in_panel(&mut self, key: Key) -> Action {
        let Some(panel) = self.panel.as_mut() else {
            return Action::None;
        };
        let repo = panel.repo;

        match key {
            Key::Down | Key::Char('j') => {
                panel.move_cursor(1);
                Action::None
            }
            Key::Up | Key::Char('k') => {
                panel.move_cursor(-1);
                Action::None
            }
            Key::Home => {
                panel.cursor = 0;
                Action::None
            }
            Key::End => {
                panel.cursor = panel.items().len().saturating_sub(1);
                Action::None
            }
            Key::Char(' ') => {
                if let Some(Item::Commit(c)) = panel.selected() {
                    panel.toggle(c);
                }
                Action::None
            }
            Key::Right | Key::Char('l') => match panel.selected() {
                // On a commit the rightward move opens it; on a file it goes
                // one level further, into the diff.
                Some(Item::Commit(c)) => {
                    if !panel.expanded.get(c).copied().unwrap_or(false) {
                        panel.toggle(c);
                    }
                    Action::None
                }
                Some(Item::File(c, f)) => self.open_diff(c, f),
                None => Action::None,
            },
            Key::Left | Key::Char('h') | Key::Esc => {
                self.panel = None;
                Action::None
            }
            // Repo actions keep working, so you can look at the commits and
            // then push without leaving. They target the panel's repository
            // explicitly, not the list cursor — a background refresh can move
            // that out from under the panel.
            Key::Char('p') => self.push_row(repo),
            Key::Enter | Key::Char('g') => self.open_gitui(repo),
            Key::Char('f') => self.job_on_row(repo, Job::Fetch),
            Key::Char('r') => self.job_on_row(repo, Job::Scan),
            Key::Char('q') => Action::Quit,
            _ => Action::None,
        }
    }

    fn open_diff(&mut self, commit: usize, file: usize) -> Action {
        let Some(panel) = self.panel.as_ref() else {
            return Action::None;
        };
        let Some(c) = panel.commits.get(commit) else {
            return Action::None;
        };
        let Some(f) = c.files.get(file) else {
            return Action::None;
        };
        let name = self
            .rows
            .get(panel.repo)
            .map(|r| r.repo.name.as_str())
            .unwrap_or("?");

        self.diff = Some(Diff {
            repo: panel.repo,
            title: format!("{name} · {} · {}", c.short_id, f.path),
            path: f.path.clone(),
            lines: Vec::new(),
            scroll: 0,
            loading: true,
            error: None,
        });
        Action::Start(vec![Job::Diff {
            index: panel.repo,
            commit: c.full_id.clone(),
            file: f.path.clone(),
        }])
    }

    fn key_in_diff(&mut self, key: Key) -> Action {
        let Some(diff) = self.diff.as_mut() else {
            return Action::None;
        };
        // One short of a screen, so a line of context carries over.
        let page = 20;
        let last = diff.lines.len().saturating_sub(1);

        match key {
            Key::Down | Key::Char('j') => {
                diff.scroll = (diff.scroll + 1).min(last);
                Action::None
            }
            Key::Up | Key::Char('k') => {
                diff.scroll = diff.scroll.saturating_sub(1);
                Action::None
            }
            Key::PageDown | Key::Char(' ') => {
                diff.scroll = (diff.scroll + page).min(last);
                Action::None
            }
            Key::PageUp => {
                diff.scroll = diff.scroll.saturating_sub(page);
                Action::None
            }
            Key::Home => {
                diff.scroll = 0;
                Action::None
            }
            Key::End => {
                diff.scroll = last;
                Action::None
            }
            Key::Left | Key::Char('h') | Key::Esc => {
                self.diff = None;
                Action::None
            }
            Key::Char('q') => Action::Quit,
            _ => Action::None,
        }
    }

    fn push_job(&self, index: usize) -> Result<Job, String> {
        self.require_idle(index)?;
        let row = self
            .rows
            .get(index)
            .ok_or_else(|| "Row does not exist.".to_string())?;

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
            Some(index) => match self.require_idle(index) {
                Ok(()) => self.start(vec![make(index)]),
                Err(reason) => self.warn(&reason),
            },
            None => self.warn("No row selected."),
        }
    }

    /// Scan jobs for a background refresh.
    ///
    /// Deliberately **not** passed through `begin`: these rows are not marked
    /// busy, so no spinner flashes every few seconds and nothing is locked
    /// while you are about to press a key. Rows that are already working are
    /// left alone — re-reading a repository mid-push is wasted effort.
    pub fn refresh_jobs(&self) -> Vec<Job> {
        (0..self.rows.len())
            .filter(|&index| self.require_idle(index).is_ok())
            .map(Job::Scan)
            .collect()
    }

    /// Indices that are free to take work, for the bulk keys.
    ///
    /// Reading covers **every** row, not just the visible ones. The default
    /// view hides repositories with nothing to do; scoping a refresh to what is
    /// on screen would mean a repository that has since become dirty could
    /// never reappear. Writing stays scoped to what you can see — that is what
    /// `A` is for.
    fn idle_rows(&self) -> (Vec<usize>, usize) {
        let idle: Vec<usize> = (0..self.rows.len())
            .filter(|&index| self.require_idle(index).is_ok())
            .collect();
        let skipped = self.rows.len() - idle.len();
        (idle, skipped)
    }

    fn start_bulk(&mut self, make: fn(usize) -> Job, what: &str) -> Action {
        let (targets, skipped) = self.idle_rows();
        if targets.is_empty() {
            return self.warn(&format!("No rows available to {what}."));
        }
        let jobs: Vec<Job> = targets.into_iter().map(make).collect();
        let started = jobs.len();
        let action = self.start(jobs);
        if skipped > 0 {
            self.message = Some(Message {
                level: Level::Info,
                text: format!("{what}: {started} started, {skipped} busy and skipped."),
            });
        }
        action
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
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
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
        assert_eq!(app.rows[0].activity(), Activity::Pushing);
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
        assert!(app.rows.iter().all(|r| r.activity() == Activity::Idle));
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
        assert!(app.message.as_ref().unwrap().text.contains("is pushing"));
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
        assert_eq!(app.rows[0].activity(), Activity::Idle);
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

    /// Startup schedules a scan *and* a fetch for every row. The scan finishes
    /// first, and with a single activity field it used to clear the fetch state
    /// too — the row looked idle while the fetch was still running, so no
    /// spinner appeared and nothing stopped a second action.
    #[test]
    fn a_finished_scan_does_not_clear_a_running_fetch() {
        let mut app = App::new("/tmp".into(), vec![repo("a")]);
        app.startup_jobs();
        assert_eq!(app.rows[0].activity(), Activity::Fetching);

        app.apply(JobResult::Scanned {
            index: 0,
            outcome: Box::new(status("a", "main", 0, false)),
        });
        assert_eq!(
            app.rows[0].activity(),
            Activity::Fetching,
            "the fetch is still running"
        );
        assert!(app.require_idle(0).is_err(), "row must stay locked");

        app.apply(JobResult::Fetched {
            index: 0,
            result: Ok(()),
        });
        assert_eq!(app.rows[0].activity(), Activity::Idle);
    }

    /// A push outranks a background fetch in what the row reports.
    #[test]
    fn the_most_consequential_kind_of_work_is_shown() {
        let mut app = app();
        app.rows[0].set_busy(Activity::Fetching, true);
        assert_eq!(app.rows[0].activity(), Activity::Fetching);
        app.rows[0].set_busy(Activity::Pushing, true);
        assert_eq!(app.rows[0].activity(), Activity::Pushing);
        app.rows[0].set_busy(Activity::Pushing, false);
        assert_eq!(app.rows[0].activity(), Activity::Fetching);
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

    /// Enter is the binding the hand reaches for; `g` stays as a second one.
    #[test]
    fn enter_opens_gitui_just_like_g() {
        let mut app = app();
        assert_eq!(app.on_key(Key::Enter), Action::OpenGitui(0));
        assert_eq!(app.on_key(Key::Char('g')), Action::OpenGitui(0));
    }

    /// Enter is the main action key in the list, so it must not double as
    /// consent in the confirmation dialog.
    #[test]
    fn enter_does_not_confirm_a_bulk_push() {
        let mut app = app();
        app.on_key(Key::Char('A'));
        assert_eq!(app.on_key(Key::Enter), Action::None);
        assert!(
            matches!(app.mode, Mode::Confirm(_)),
            "dialog must stay open"
        );

        match app.on_key(Key::Char('y')) {
            Action::Start(jobs) => assert_eq!(jobs.len(), 2),
            other => panic!("expected Start, got {other:?}"),
        }
    }

    #[test]
    fn every_action_on_a_busy_row_is_refused() {
        for key in [
            Key::Char('p'),
            Key::Char('f'),
            Key::Char('r'),
            Key::Enter,
            Key::Char('g'),
        ] {
            let mut app = app();
            app.rows[0].set_busy(Activity::Pushing, true);
            let action = app.on_key(key);
            assert_eq!(action, Action::None, "key {key:?} should do nothing");
            assert!(
                app.message.as_ref().unwrap().text.contains("is pushing"),
                "key {key:?} should explain why"
            );
        }
    }

    /// Moving around and marking stay available — only work that touches the
    /// repository is held back.
    #[test]
    fn navigation_and_marking_still_work_on_a_busy_row() {
        let mut app = app();
        app.rows[0].set_busy(Activity::Fetching, true);
        app.on_key(Key::Char(' '));
        assert!(app.rows[0].marked);
        app.on_key(Key::Down);
        assert_eq!(app.cursor, 1);
    }

    #[test]
    fn bulk_actions_skip_busy_rows_instead_of_failing() {
        let mut app = app();
        app.rows[0].set_busy(Activity::Pushing, true);
        match app.on_key(Key::Char('F')) {
            Action::Start(jobs) => assert_eq!(jobs, vec![Job::Fetch(1), Job::Fetch(2)]),
            other => panic!("expected Start, got {other:?}"),
        }
        assert!(app
            .message
            .as_ref()
            .unwrap()
            .text
            .contains("1 busy and skipped"));
    }

    #[test]
    fn bulk_action_with_nothing_available_says_so() {
        let mut app = app();
        for row in &mut app.rows {
            row.set_busy(Activity::Pushing, true);
        }
        assert_eq!(app.on_key(Key::Char('R')), Action::None);
        assert!(app
            .message
            .as_ref()
            .unwrap()
            .text
            .contains("No rows available"));
    }

    /// The default view hides repositories with nothing to do. If a refresh
    /// only covered what is on screen, one that has since become dirty could
    /// never come back into view.
    #[test]
    fn refreshing_covers_rows_the_view_is_hiding() {
        let mut app = app();
        assert_eq!(app.visible(), vec![0, 2], "row 1 is clean and hidden");

        match app.on_key(Key::Char('R')) {
            Action::Start(jobs) => {
                assert_eq!(jobs, vec![Job::Scan(0), Job::Scan(1), Job::Scan(2)]);
            }
            other => panic!("expected Start, got {other:?}"),
        }
    }

    /// Pushing stays scoped to what is on screen — that distinction is the
    /// whole point of `A`.
    #[test]
    fn pushing_stays_scoped_to_the_visible_rows() {
        let mut app = app();
        app.on_key(Key::Char('A'));
        match &app.mode {
            Mode::Confirm(confirm) => {
                assert_eq!(confirm.targets, vec!["backend-api", "api-gateway"]);
            }
            other => panic!("expected Confirm, got {other:?}"),
        }
    }

    #[test]
    fn a_background_refresh_neither_locks_nor_marks_rows() {
        let mut app = app();
        app.rows[0].set_busy(Activity::Pushing, true);

        let jobs = app.refresh_jobs();
        assert_eq!(jobs, vec![Job::Scan(1), Job::Scan(2)], "busy row skipped");
        assert_eq!(
            app.rows[1].activity(),
            Activity::Idle,
            "no spinner for a background refresh"
        );
        assert!(app.require_idle(1).is_ok(), "and nothing is locked");
    }

    /// A refresh must not slide a row out from under the cursor.
    #[test]
    fn the_cursor_stays_on_the_same_repository_across_a_refresh() {
        let mut app = app();
        app.on_key(Key::Down);
        assert_eq!(app.selected(), Some(2));

        app.apply(JobResult::Scanned {
            index: 0,
            outcome: Box::new(status("backend-api", "feature/login", 0, false)),
        });

        assert_eq!(app.visible(), vec![2], "backend-api left the view");
        assert_eq!(app.selected(), Some(2), "cursor still on api-gateway");
    }

    #[test]
    fn the_spinner_advances_with_each_tick() {
        let mut app = app();
        let first = app.spinner();
        app.tick();
        assert_ne!(first, app.spinner());
        // One full turn returns to the start.
        for _ in 1..10 {
            app.tick();
        }
        assert_eq!(first, app.spinner());
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
