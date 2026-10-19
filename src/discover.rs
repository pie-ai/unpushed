//! Finding checkouts below a base directory.
//!
//! Exactly one level is searched: every direct child directory is tested for a
//! `.git` entry. There is deliberately no recursion. In the surveyed layout
//! 79% of repositories sit inside another repository, so stopping at the first
//! `.git` would miss them — and descending through everything would mean
//! walking 93k directories. Pointing the tool at the directory that holds the
//! checkouts sidesteps both problems.

use std::io;
use std::path::Path;

use crate::model::Repo;

/// Collects the checkouts directly below `base`, sorted by name.
///
/// Entries that cannot be read are skipped rather than failing the whole scan;
/// only an unreadable `base` is reported as an error.
pub fn discover(base: &Path) -> io::Result<Vec<Repo>> {
    let mut repos = Vec::new();

    for entry in std::fs::read_dir(base)? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();

        // `file_type` does not follow symlinks, so a symlinked checkout would
        // be missed by it. Fall back to `metadata`, which does.
        let is_dir = match entry.file_type() {
            Ok(ft) if ft.is_dir() => true,
            Ok(ft) if ft.is_symlink() => path.metadata().map(|m| m.is_dir()).unwrap_or(false),
            _ => false,
        };
        if !is_dir || !is_git_checkout(&path) {
            continue;
        }

        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        repos.push(Repo {
            name: name.to_string(),
            path,
        });
    }

    repos.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(repos)
}

/// A checkout has a `.git` entry — a directory normally, or a file when it is
/// a worktree or a submodule.
fn is_git_checkout(path: &Path) -> bool {
    path.join(".git").exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "unpushed-test-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(&base).unwrap();
            TempDir(base)
        }

        fn checkout(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::create_dir_all(path.join(".git")).unwrap();
            path
        }

        fn plain_dir(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::create_dir_all(&path).unwrap();
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn finds_checkouts_and_ignores_plain_directories() {
        let tmp = TempDir::new("basic");
        tmp.checkout("backend-api");
        tmp.checkout("shared-lib");
        tmp.plain_dir("not-a-repo");

        let found = discover(&tmp.0).unwrap();
        let names: Vec<_> = found.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["backend-api", "shared-lib"]);
    }

    #[test]
    fn does_not_recurse() {
        let tmp = TempDir::new("nested");
        let parent = tmp.checkout("workspace");
        std::fs::create_dir_all(parent.join("projects/backend-api/.git")).unwrap();

        let found = discover(&tmp.0).unwrap();
        let names: Vec<_> = found.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["workspace"]);
    }

    /// The nested checkouts become visible by pointing the tool one level
    /// deeper — this is the intended way to reach them.
    #[test]
    fn nested_checkouts_are_found_from_their_own_parent() {
        let tmp = TempDir::new("nested-parent");
        let parent = tmp.checkout("workspace");
        let projects = parent.join("projects");
        std::fs::create_dir_all(projects.join("backend-api/.git")).unwrap();
        std::fs::create_dir_all(projects.join("shared-lib/.git")).unwrap();

        let found = discover(&projects).unwrap();
        let names: Vec<_> = found.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["backend-api", "shared-lib"]);
    }

    /// Worktrees and submodules carry a `.git` file instead of a directory.
    #[test]
    fn accepts_git_as_a_file() {
        let tmp = TempDir::new("gitfile");
        let path = tmp.plain_dir("worktree");
        std::fs::write(path.join(".git"), "gitdir: /elsewhere/.git/worktrees/wt").unwrap();

        let found = discover(&tmp.0).unwrap();
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn missing_base_is_an_error() {
        let tmp = TempDir::new("missing");
        assert!(discover(&tmp.0.join("nope")).is_err());
    }

    #[test]
    fn empty_base_yields_nothing() {
        let tmp = TempDir::new("empty");
        assert!(discover(&tmp.0).unwrap().is_empty());
    }
}
