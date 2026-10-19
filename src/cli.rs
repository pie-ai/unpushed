// Command line definition.
//
// This file is the single source for the parser, the help text, the man page
// and the shell completions: `main.rs` declares it as a module and `build.rs`
// pulls it in with `include!`. Regular comments rather than `//!` on purpose —
// inner doc comments are not allowed at an `include!` site. Keep this file free
// of crate-internal types; `build.rs` compiles it without the rest of the crate.

use clap::Parser;
use std::num::NonZeroUsize;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "unpushed",
    version,
    about = "Overview of unpushed commits across many git checkouts",
    long_about = "Reads every git checkout exactly one level below DIR and shows what is \
                  not pushed yet, together with local changes.\n\n\
                  Only one level is searched, so point this at the directory that holds \
                  the checkouts rather than at the root of a tree.\n\n\
                  Without a terminal on stdout a plain table is printed instead of the \
                  interface, so the command stays usable in a pipe."
)]
pub struct Cli {
    /// Directory holding the checkouts
    #[arg(value_name = "DIR", default_value = ".")]
    pub dir: PathBuf,

    /// Print a table instead of the interface
    #[arg(short = 'T', long)]
    pub table: bool,

    /// Also show repositories with nothing to do (table output only)
    #[arg(short, long)]
    pub all: bool,

    /// Number of parallel jobs
    // Kept in step with `scan::DEFAULT_THREADS` by a test in `main.rs`; this file
    // cannot reference the crate because `build.rs` includes it on its own.
    #[arg(short = 'j', long, default_value = "8", value_name = "N")]
    pub threads: NonZeroUsize,

    /// Seconds between background refreshes; 0 turns them off
    #[arg(short = 'R', long, default_value = "5", value_name = "SECONDS")]
    pub refresh: u64,

    /// Print discovery and scan duration (table output only)
    #[arg(short, long)]
    pub timing: bool,
}
