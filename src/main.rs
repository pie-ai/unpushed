//! CLI entry point.
//!
//! Starts the interface when stdout is a terminal, and falls back to the plain
//! table otherwise, so the command stays usable in a pipe or a script.

mod cli;

use std::io::IsTerminal;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::Parser;

use cli::Cli;
use unpushed::model::{Repo, Summary};
use unpushed::table::{render, View};
use unpushed::{discover, scan, tui};

fn main() -> ExitCode {
    let args = Cli::parse();

    let started = Instant::now();
    let repos = match discover::discover(&args.dir) {
        Ok(repos) => repos,
        Err(err) => {
            eprintln!("unpushed: cannot read {}: {err}", args.dir.display());
            return ExitCode::FAILURE;
        }
    };
    let discovered = started.elapsed();

    if repos.is_empty() {
        eprintln!(
            "unpushed: no git checkouts directly below {}.\n\
             Only one level is searched — does the path point at the directory\n\
             that holds the checkouts?",
            args.dir.display()
        );
        return ExitCode::FAILURE;
    }

    if args.table || !std::io::stdout().is_terminal() {
        return run_table(&args, repos, discovered);
    }

    let base = args.dir.display().to_string();
    if let Err(err) = tui::run(base, repos, args.threads.get(), args.refresh) {
        eprintln!("unpushed: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn run_table(args: &Cli, repos: Vec<Repo>, discovered: Duration) -> ExitCode {
    let view = if args.all {
        View::All
    } else {
        View::NeedsAttention
    };

    let scan_started = Instant::now();
    let outcomes = scan::scan_all(&repos, args.threads.get());
    let scanned = scan_started.elapsed();

    print!("{}", render(&outcomes, view));

    if args.timing {
        println!(
            "\nDiscovery {:.2?} · scan {:.2?} ({} repos, {} threads)",
            discovered,
            scanned,
            repos.len(),
            args.threads
        );
    }

    // A failed scan is a result, not a crash, but it must be visible to a
    // caller that only looks at the exit status.
    if Summary::of(&outcomes).failed > 0 {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        let mut argv = vec!["unpushed"];
        argv.extend_from_slice(args);
        Cli::try_parse_from(argv)
    }

    #[test]
    fn defaults_to_cwd_and_filtered_view() {
        let args = parse(&[]).unwrap();
        assert_eq!(args.dir, PathBuf::from("."));
        assert!(!args.all);
        assert!(!args.table);
        assert!(!args.timing);
    }

    /// The default lives in `cli.rs` as a literal because `build.rs` compiles
    /// that file without the crate. This keeps the two from drifting.
    #[test]
    fn default_thread_count_matches_the_scanner() {
        assert_eq!(parse(&[]).unwrap().threads.get(), scan::DEFAULT_THREADS);
    }

    #[test]
    fn accepts_a_folder_argument() {
        assert_eq!(
            parse(&["/some/where"]).unwrap().dir,
            PathBuf::from("/some/where")
        );
    }

    #[test]
    fn all_flag_switches_the_view() {
        assert!(parse(&["--all"]).unwrap().all);
        assert!(parse(&["-a"]).unwrap().all);
    }

    #[test]
    fn table_can_be_forced() {
        assert!(parse(&["-T"]).unwrap().table);
        assert!(parse(&["--table"]).unwrap().table);
    }

    #[test]
    fn threads_must_be_a_positive_number() {
        assert_eq!(parse(&["-j", "4"]).unwrap().threads.get(), 4);
        assert!(parse(&["-j", "0"]).is_err());
        assert!(parse(&["-j", "x"]).is_err());
        assert!(parse(&["-j"]).is_err());
    }

    #[test]
    fn unknown_options_and_extra_folders_are_rejected() {
        assert!(parse(&["--nope"]).is_err());
        assert!(parse(&["/a", "/b"]).is_err());
    }

    #[test]
    fn version_and_help_are_available() {
        let err = parse(&["--version"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
        assert!(err.to_string().contains(env!("CARGO_PKG_VERSION")));

        let err = parse(&["--help"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp);
    }
}
