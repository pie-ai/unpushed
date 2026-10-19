//! Terminal setup, the event loop, and the worker pool behind it.
//!
//! Input is polled on the main thread rather than read by a dedicated thread.
//! That matters for one feature in particular: handing the terminal to gitui.
//! A background reader would keep competing for stdin while gitui is running,
//! swallowing the user's keystrokes.

use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;

use crate::app::{Action, App, Job, JobResult, Key};
use crate::git;
use crate::model::Repo;
use crate::scan;
use crate::ui;

/// How long the loop waits for input before redrawing. Short enough that
/// results arriving from background work appear promptly.
const TICK: Duration = Duration::from_millis(100);

pub fn run(base: String, repos: Vec<Repo>, threads: usize, refresh_secs: u64) -> io::Result<()> {
    let repos = Arc::new(repos);
    let mut app = App::new(base, (*repos).clone());

    let (results_tx, results_rx) = channel::<JobResult>();
    let pool = Pool::new(threads, Arc::clone(&repos), results_tx);

    for job in app.startup_jobs() {
        pool.submit(job);
    }

    let mut terminal = ratatui::init();
    let refresh = (refresh_secs > 0).then(|| Duration::from_secs(refresh_secs));
    let outcome = event_loop(&mut terminal, &mut app, &pool, &results_rx, &repos, refresh);
    ratatui::restore();
    outcome
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    pool: &Pool,
    results: &Receiver<JobResult>,
    repos: &[Repo],
    refresh: Option<Duration>,
) -> io::Result<()> {
    let mut last_tick = Instant::now();
    let mut last_refresh = Instant::now();

    loop {
        while let Ok(result) = results.try_recv() {
            app.apply(result);
        }

        // Driven by elapsed time, not by loop iterations: a held-down key makes
        // `poll` return immediately every time, which would otherwise spin the
        // animation far too fast — or, with the tick inside the poll timeout
        // branch, freeze it entirely.
        if last_tick.elapsed() >= TICK {
            app.tick();
            last_tick = Instant::now();
        }

        // Background refresh. Skipped while the pool still has work queued, so
        // a slow scan cannot pile refreshes on top of each other; the timer
        // restarts either way, which spaces them out rather than catching up.
        if let Some(every) = refresh {
            if last_refresh.elapsed() >= every {
                if pool.is_idle() {
                    for job in app.refresh_jobs() {
                        pool.submit(job);
                    }
                }
                last_refresh = Instant::now();
            }
        }

        terminal.draw(|frame| ui::draw(frame, app))?;

        if !event::poll(TICK)? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };
        // Key events fire on press and release on some terminals; acting on
        // both would double every keystroke.
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if is_interrupt(&key) {
            return Ok(());
        }

        match app.on_key(translate(key)) {
            Action::None => {}
            Action::Quit => return Ok(()),
            Action::Start(jobs) => {
                for job in jobs {
                    pool.submit(job);
                }
            }
            Action::OpenGitui(index) => {
                let Some(repo) = repos.get(index) else {
                    continue;
                };

                // Full handover: leave the alternate screen and raw mode so
                // gitui owns the terminal, then take it back and redraw.
                ratatui::restore();
                let result = git::gitui(&repo.path);
                // try_init rather than init: a failure to take the terminal
                // back travels up to the caller, which restores and reports it,
                // instead of panicking out of the middle of the loop.
                *terminal = ratatui::try_init()?;
                terminal.clear()?;

                match result {
                    Ok(()) => pool.submit(Job::Scan(index)),
                    Err(err) => {
                        app.apply(JobResult::Fetched {
                            index,
                            result: Err(err),
                        });
                    }
                }
            }
        }
    }
}

fn is_interrupt(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('d'))
}

fn translate(key: KeyEvent) -> Key {
    match key.code {
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Backspace => Key::Backspace,
        _ => Key::Other,
    }
}

/// A fixed set of worker threads pulling from a shared queue.
///
/// A plain `Arc<Mutex<Receiver>>` would not do: a worker blocked in `recv`
/// holds the lock, so the other workers cannot take any job. The queue plus
/// condvar lets every worker wait without owning the lock.
struct Pool {
    inner: Arc<Shared>,
    workers: Vec<std::thread::JoinHandle<()>>,
}

struct Shared {
    queue: Mutex<VecDeque<Job>>,
    available: Condvar,
    stopping: AtomicBool,
}

impl Pool {
    fn new(threads: usize, repos: Arc<Vec<Repo>>, results: Sender<JobResult>) -> Self {
        let inner = Arc::new(Shared {
            queue: Mutex::new(VecDeque::new()),
            available: Condvar::new(),
            stopping: AtomicBool::new(false),
        });

        let workers = (0..threads.max(1))
            .map(|_| {
                let inner = Arc::clone(&inner);
                let repos = Arc::clone(&repos);
                let results = results.clone();
                std::thread::spawn(move || worker(inner, repos, results))
            })
            .collect();

        Pool { inner, workers }
    }

    /// Whether nothing is waiting to be picked up. Used to hold back a
    /// background refresh rather than queueing work behind a slow one.
    fn is_idle(&self) -> bool {
        self.inner.queue.lock().unwrap().is_empty()
    }

    fn submit(&self, job: Job) {
        self.inner.queue.lock().unwrap().push_back(job);
        self.inner.available.notify_one();
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.inner.stopping.store(true, Ordering::Release);
        self.inner.available.notify_all();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn worker(inner: Arc<Shared>, repos: Arc<Vec<Repo>>, results: Sender<JobResult>) {
    loop {
        let job = {
            let mut queue = inner.queue.lock().unwrap();
            loop {
                if inner.stopping.load(Ordering::Acquire) {
                    return;
                }
                if let Some(job) = queue.pop_front() {
                    break job;
                }
                queue = inner.available.wait(queue).unwrap();
            }
        };

        for result in execute(job, &repos) {
            if results.send(result).is_err() {
                return;
            }
        }
    }
}

/// Runs one job. Returns several results where one piece of work changes more
/// than one thing — fetching and pushing both move refs, so the row has to be
/// re-read afterwards or the table would keep showing pre-fetch numbers.
fn execute(job: Job, repos: &[Repo]) -> Vec<JobResult> {
    let Some(repo) = repos.get(index_of(&job)) else {
        return Vec::new();
    };

    match job {
        Job::Scan(index) => vec![JobResult::Scanned {
            index,
            outcome: Box::new(scan::scan_one(repo)),
        }],
        Job::Fetch(index) => {
            let result = git::fetch(&repo.path);
            let refreshed = result.is_ok();
            let mut results = vec![JobResult::Fetched { index, result }];
            if refreshed {
                results.push(JobResult::Scanned {
                    index,
                    outcome: Box::new(scan::scan_one(repo)),
                });
            }
            results
        }
        Job::Rebase { index, onto } => {
            let outcome = crate::git::rebase(&repo.path, onto);
            let rebased = outcome.is_success();
            let mut results = vec![JobResult::Rebased { index, outcome }];
            // A rebase moves HEAD, so the row's numbers are stale the moment
            // it finishes.
            if rebased {
                results.push(JobResult::Scanned {
                    index,
                    outcome: Box::new(scan::scan_one(repo)),
                });
            }
            results
        }
        Job::Commits(index) => vec![JobResult::Commits {
            index,
            result: crate::commits::unpushed(&repo.path).map_err(|e| e.message().to_string()),
        }],
        Job::Diff {
            index,
            commit,
            file,
        } => vec![JobResult::Diff {
            index,
            result: crate::commits::file_diff(&repo.path, &commit, &file)
                .map_err(|e| e.message().to_string()),
        }],
        Job::Push { index, branch } => {
            let outcome = git::push(&repo.path, &branch);
            let pushed = outcome.is_success();
            let mut results = vec![JobResult::Pushed { index, outcome }];
            if pushed {
                results.push(JobResult::Scanned {
                    index,
                    outcome: Box::new(scan::scan_one(repo)),
                });
            }
            results
        }
    }
}

fn index_of(job: &Job) -> usize {
    match job {
        Job::Scan(index)
        | Job::Fetch(index)
        | Job::Push { index, .. }
        | Job::Rebase { index, .. }
        | Job::Commits(index)
        | Job::Diff { index, .. } => *index,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn keys_are_translated() {
        assert_eq!(translate(key(KeyCode::Char('p'))), Key::Char('p'));
        assert_eq!(translate(key(KeyCode::Up)), Key::Up);
        assert_eq!(translate(key(KeyCode::Esc)), Key::Esc);
        assert_eq!(translate(key(KeyCode::Right)), Key::Right);
        assert_eq!(translate(key(KeyCode::Left)), Key::Left);
        assert_eq!(translate(key(KeyCode::PageDown)), Key::PageDown);
        assert_eq!(translate(key(KeyCode::Tab)), Key::Other);
    }

    #[test]
    fn ctrl_c_and_ctrl_d_interrupt() {
        assert!(is_interrupt(&KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )));
        assert!(is_interrupt(&KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::CONTROL
        )));
        assert!(!is_interrupt(&key(KeyCode::Char('c'))));
    }

    /// Every worker must be able to take a job while the others are busy.
    #[test]
    fn workers_pull_jobs_concurrently() {
        use std::path::PathBuf;

        let repos = Arc::new(vec![
            Repo {
                name: "a".into(),
                path: PathBuf::from("/definitely/not/a/repo/a"),
            },
            Repo {
                name: "b".into(),
                path: PathBuf::from("/definitely/not/a/repo/b"),
            },
        ]);
        let (tx, rx) = channel();
        let pool = Pool::new(4, repos, tx);

        pool.submit(Job::Scan(0));
        pool.submit(Job::Scan(1));

        // Both jobs must come back; the scans fail (the paths do not exist),
        // which is exactly the outcome the model is meant to carry.
        for _ in 0..2 {
            let result = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(matches!(result, JobResult::Scanned { .. }));
        }
    }

    #[test]
    fn out_of_range_jobs_are_ignored() {
        let repos: Vec<Repo> = Vec::new();
        assert!(execute(Job::Scan(7), &repos).is_empty());
        assert!(execute(Job::Fetch(7), &repos).is_empty());
        assert!(execute(
            Job::Push {
                index: 7,
                branch: "main".into()
            },
            &repos
        )
        .is_empty());
    }

    /// A fetch that failed must not pretend the row was refreshed.
    #[test]
    fn a_failed_fetch_produces_no_rescan() {
        use std::path::PathBuf;
        let repos = vec![Repo {
            name: "nope".into(),
            path: PathBuf::from("/definitely/not/a/repo"),
        }];
        let results = execute(Job::Fetch(0), &repos);
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0], JobResult::Fetched { .. }));
    }

    #[test]
    fn dropping_the_pool_stops_the_workers() {
        let repos = Arc::new(Vec::new());
        let (tx, _rx) = channel();
        let pool = Pool::new(2, repos, tx);
        drop(pool); // joins; hangs if shutdown is not observed
    }
}
