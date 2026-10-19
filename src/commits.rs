//! Loading the commits a repository has not pushed, and the diffs inside them.
//!
//! Read-only, through `git2`, like the rest of the reading path. Loaded on
//! demand rather than during the scan: a scan touches every repository, while
//! this only ever concerns the one row you opened. Measured at roughly 20 ms
//! for 21 commits including their file lists.
//!
//! The comparison point is resolved by [`crate::scan::base_ref`], the same
//! function the scanner uses. That is deliberate — if the panel picked its own
//! reference, the list could say `↑21` while the panel showed a different set.

use git2::{Delta, DiffOptions, Oid, Repository};

/// A commit that exists locally and not on the remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// Abbreviated id, as git would print it.
    pub short_id: String,
    pub full_id: String,
    /// First line of the message.
    pub summary: String,
    pub author: String,
    /// Committer date, already formatted for display.
    pub date: String,
    pub files: Vec<FileChange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub status: Status,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Added,
    Modified,
    Deleted,
    Renamed,
    Other,
}

impl Status {
    /// Single letter, matching `git status --short`.
    pub fn letter(self) -> char {
        match self {
            Status::Added => 'A',
            Status::Modified => 'M',
            Status::Deleted => 'D',
            Status::Renamed => 'R',
            Status::Other => '?',
        }
    }
}

/// Upper bound on how many commits are listed.
///
/// The surveyed repositories topped out at 27 unpushed commits, so this is far
/// above the real cases; it exists so a repository that has drifted thousands
/// of commits from its remote cannot stall the interface.
pub const MAX_COMMITS: usize = 500;

/// Reads the unpushed commits of one repository, newest first.
pub fn unpushed(path: &std::path::Path) -> Result<Vec<Commit>, git2::Error> {
    let git = Repository::open(path)?;
    let Some(base) = crate::scan::base_ref(&git)? else {
        // No reference point — the same state the list shows as `?`. Nothing
        // can be called unpushed without one.
        return Ok(Vec::new());
    };
    let Some(head) = git.head().ok().and_then(|h| h.target()) else {
        return Ok(Vec::new());
    };

    let mut walk = git.revwalk()?;
    walk.push(head)?;
    walk.hide(base)?;

    let mut commits = Vec::new();
    for oid in walk.take(MAX_COMMITS) {
        commits.push(read_commit(&git, oid?)?);
    }
    Ok(commits)
}

fn read_commit(git: &Repository, oid: Oid) -> Result<Commit, git2::Error> {
    let commit = git.find_commit(oid)?;
    let short_id = commit
        .as_object()
        .short_id()
        .ok()
        .and_then(|buf| buf.as_str().ok().map(str::to_string))
        .unwrap_or_else(|| oid.to_string()[..7].to_string());

    // Bound to a local: the signature borrows from the commit, so calling
    // `.name()` on a temporary would not outlive the expression.
    let author = commit.author();

    Ok(Commit {
        short_id,
        full_id: oid.to_string(),
        summary: commit
            .summary()
            .ok()
            .flatten()
            .unwrap_or("(no message)")
            .to_string(),
        author: author.name().unwrap_or("?").to_string(),
        date: format_time(commit.time().seconds()),
        files: changed_files(git, &commit)?,
    })
}

fn changed_files(git: &Repository, commit: &git2::Commit) -> Result<Vec<FileChange>, git2::Error> {
    let tree = commit.tree()?;
    // A merge is compared against its first parent, which is what `git show`
    // does by default; a root commit against nothing.
    let parent = commit.parent(0).ok().and_then(|p| p.tree().ok());

    let mut options = DiffOptions::new();
    options.include_typechange(true);
    let diff = git.diff_tree_to_tree(parent.as_ref(), Some(&tree), Some(&mut options))?;

    let mut files = Vec::new();
    diff.foreach(
        &mut |delta, _| {
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "?".to_string());
            files.push(FileChange {
                status: match delta.status() {
                    Delta::Added => Status::Added,
                    Delta::Deleted => Status::Deleted,
                    Delta::Modified | Delta::Typechange => Status::Modified,
                    Delta::Renamed | Delta::Copied => Status::Renamed,
                    _ => Status::Other,
                },
                path,
            });
            true
        },
        None,
        None,
        None,
    )?;
    Ok(files)
}

/// One line of a diff, already classified for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// `@@ … @@` and file headers.
    Header,
    Added,
    Removed,
    Context,
}

/// Reads the diff of one file within one commit.
pub fn file_diff(
    path: &std::path::Path,
    commit_id: &str,
    file: &str,
) -> Result<Vec<DiffLine>, git2::Error> {
    let git = Repository::open(path)?;
    let commit = git.find_commit(Oid::from_str(commit_id)?)?;
    let tree = commit.tree()?;
    let parent = commit.parent(0).ok().and_then(|p| p.tree().ok());

    let mut options = DiffOptions::new();
    options.pathspec(file);
    // Three lines either side, the same as git's default.
    options.context_lines(3);
    let diff = git.diff_tree_to_tree(parent.as_ref(), Some(&tree), Some(&mut options))?;

    let mut lines = Vec::new();
    diff.print(git2::DiffFormat::Patch, |_, _, line| {
        let raw = String::from_utf8_lossy(line.content());
        let kind = match line.origin() {
            '+' => LineKind::Added,
            '-' => LineKind::Removed,
            ' ' => LineKind::Context,
            // 'F' file header, 'H' hunk header, and the binary markers.
            _ => LineKind::Header,
        };

        if kind == LineKind::Header {
            // A file header arrives as one chunk holding several lines. Left
            // whole it would render as a single run-on line.
            for part in raw.lines() {
                // `diff --git` and `index` carry nothing the title bar does
                // not already say; the rest — `---`, `+++`, `@@`, and notices
                // such as "Binary files differ" — stays.
                if part.starts_with("diff --git") || part.starts_with("index ") {
                    continue;
                }
                lines.push(DiffLine {
                    kind,
                    text: part.to_string(),
                });
            }
        } else {
            lines.push(DiffLine {
                kind,
                text: raw.trim_end_matches(['\n', '\r']).to_string(),
            });
        }
        true
    })?;
    Ok(lines)
}

/// `YYYY-MM-DD` from a unix timestamp, without pulling in a date library.
///
/// Days are counted from the epoch with the civil-from-days algorithm; the
/// commit's own timezone offset is ignored, so this is the date in UTC.
fn format_time(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
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
                "unpushed-commits-{tag}-{}-{:?}",
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
                .env("GIT_AUTHOR_NAME", "Ada")
                .env("GIT_AUTHOR_EMAIL", "ada@example.com")
                .env("GIT_COMMITTER_NAME", "Ada")
                .env("GIT_COMMITTER_EMAIL", "ada@example.com")
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        /// A clone whose local branch is ahead of its upstream.
        fn ahead(&self, name: &str) -> PathBuf {
            let seed = self.0.join(format!("{name}-seed"));
            std::fs::create_dir_all(&seed).unwrap();
            self.git(&seed, &["init", "-q", "-b", "main"]);
            std::fs::write(seed.join("README"), "one\n").unwrap();
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

        fn commit(&self, work: &Path, files: &[(&str, &str)], message: &str) {
            for (name, body) in files {
                let path = work.join(name);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).unwrap();
                }
                std::fs::write(path, body).unwrap();
            }
            self.git(work, &["add", "-A"]);
            self.git(work, &["commit", "-qm", message]);
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn lists_only_what_is_not_pushed_newest_first() {
        let fx = Fixture::new("list");
        let work = fx.ahead("demo");
        fx.commit(&work, &[("a.txt", "a\n")], "first");
        fx.commit(&work, &[("b.txt", "b\n")], "second");

        let commits = unpushed(&work).unwrap();
        let summaries: Vec<&str> = commits.iter().map(|c| c.summary.as_str()).collect();
        assert_eq!(summaries, vec!["second", "first"], "newest first");
        // "seed" is on the remote and must not appear.
        assert!(!summaries.contains(&"seed"));
    }

    /// The panel must agree with the number the list shows, or one of them is
    /// lying.
    #[test]
    fn the_count_matches_what_the_scanner_reports() {
        let fx = Fixture::new("agree");
        let work = fx.ahead("demo");
        for n in 0..4 {
            fx.commit(&work, &[("f.txt", &format!("{n}\n"))], &format!("c{n}"));
        }

        let repo = crate::model::Repo {
            name: "demo".into(),
            path: work.clone(),
        };
        let status = match crate::scan::scan_one(&repo) {
            crate::model::ScanOutcome::Ok(status) => status,
            other => panic!("scan failed: {other:?}"),
        };
        assert_eq!(unpushed(&work).unwrap().len(), status.unpushed().unwrap());
    }

    #[test]
    fn a_commit_carries_its_changed_files() {
        let fx = Fixture::new("files");
        let work = fx.ahead("demo");
        fx.commit(
            &work,
            &[
                ("src/one.rs", "fn one() {}\n"),
                ("src/two.rs", "fn two() {}\n"),
            ],
            "two files",
        );

        let commits = unpushed(&work).unwrap();
        let paths: Vec<&str> = commits[0].files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["src/one.rs", "src/two.rs"]);
        assert!(commits[0].files.iter().all(|f| f.status == Status::Added));
    }

    #[test]
    fn deletions_and_modifications_are_told_apart() {
        let fx = Fixture::new("status");
        let work = fx.ahead("demo");
        fx.commit(&work, &[("gone.txt", "x\n")], "add");
        std::fs::remove_file(work.join("gone.txt")).unwrap();
        std::fs::write(work.join("README"), "changed\n").unwrap();
        fx.git(&work, &["add", "-A"]);
        fx.git(&work, &["commit", "-qm", "remove and change"]);

        let commits = unpushed(&work).unwrap();
        let by_path: Vec<(char, &str)> = commits[0]
            .files
            .iter()
            .map(|f| (f.status.letter(), f.path.as_str()))
            .collect();
        assert!(by_path.contains(&('D', "gone.txt")), "{by_path:?}");
        assert!(by_path.contains(&('M', "README")), "{by_path:?}");
    }

    #[test]
    fn a_repository_with_nothing_unpushed_yields_nothing() {
        let fx = Fixture::new("empty");
        let work = fx.ahead("demo");
        assert!(unpushed(&work).unwrap().is_empty());
    }

    /// Without a reference point nothing can be called unpushed — the same
    /// state the list renders as `?`.
    #[test]
    fn a_repository_without_a_reference_point_yields_nothing() {
        let fx = Fixture::new("noref");
        let work = fx.0.join("solo");
        std::fs::create_dir_all(&work).unwrap();
        fx.git(&work, &["init", "-q", "-b", "main"]);
        fx.commit(&work, &[("a.txt", "a\n")], "one");
        assert!(unpushed(&work).unwrap().is_empty());
    }

    #[test]
    fn a_diff_is_classified_into_added_removed_and_context() {
        let fx = Fixture::new("diff");
        let work = fx.ahead("demo");
        fx.commit(&work, &[("code.rs", "one\ntwo\nthree\n")], "add");
        fx.commit(&work, &[("code.rs", "one\nTWO\nthree\n")], "change");

        let commits = unpushed(&work).unwrap();
        let lines = file_diff(&work, &commits[0].full_id, "code.rs").unwrap();

        let added: Vec<&str> = lines
            .iter()
            .filter(|l| l.kind == LineKind::Added)
            .map(|l| l.text.as_str())
            .collect();
        let removed: Vec<&str> = lines
            .iter()
            .filter(|l| l.kind == LineKind::Removed)
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(added, vec!["TWO"]);
        assert_eq!(removed, vec!["two"]);
        assert!(lines.iter().any(|l| l.kind == LineKind::Context));
        assert!(lines.iter().any(|l| l.kind == LineKind::Header));
    }

    /// A commit touching several files must show only the one asked for.
    /// git2 hands the file header over as one chunk containing several
    /// lines; left whole it renders as one run-on line.
    #[test]
    fn the_file_header_is_split_into_separate_lines() {
        let fx = Fixture::new("header");
        let work = fx.ahead("demo");
        fx.commit(&work, &[("code.rs", "one\n")], "add");

        let commits = unpushed(&work).unwrap();
        let lines = file_diff(&work, &commits[0].full_id, "code.rs").unwrap();

        assert!(
            lines.iter().all(|l| !l.text.contains('\n')),
            "a line still holds an embedded newline"
        );
        // Noise the title bar already covers is left out.
        assert!(lines.iter().all(|l| !l.text.starts_with("diff --git")));
        assert!(lines.iter().all(|l| !l.text.starts_with("index ")));
        // The hunk header is the one that orients you, and it stays.
        assert!(lines.iter().any(|l| l.text.starts_with("@@")));
    }

    #[test]
    fn a_diff_is_scoped_to_the_requested_file() {
        let fx = Fixture::new("scope");
        let work = fx.ahead("demo");
        fx.commit(
            &work,
            &[("keep.txt", "keep\n"), ("other.txt", "other\n")],
            "two files",
        );

        let commits = unpushed(&work).unwrap();
        let lines = file_diff(&work, &commits[0].full_id, "keep.txt").unwrap();
        let body: String = lines.iter().map(|l| l.text.as_str()).collect();
        assert!(body.contains("keep"), "{body}");
        assert!(!body.contains("other"), "{body}");
    }

    #[test]
    fn dates_are_formatted_from_the_epoch() {
        assert_eq!(format_time(0), "1970-01-01");
        assert_eq!(format_time(1_000_000_000), "2001-09-09");
        // A leap day, where a naive calculation goes wrong.
        assert_eq!(format_time(1_583_020_800), "2020-03-01");
    }
}
