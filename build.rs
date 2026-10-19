//! Generates the man page and shell completions from the command line
//! definition, so they cannot drift from `--help`.
//!
//! Output goes into `man/` and `completions/` next to the manifest rather than
//! `OUT_DIR`: the Debian package and the release tarballs reference those paths
//! directly, and digging them out of a target directory would buy nothing. Both
//! directories are generated artefacts and are git-ignored.

use std::fs;
use std::io;
use std::path::Path;

use clap::CommandFactory;
use clap_complete::{generate_to, Shell};

include!("src/cli.rs");

fn main() -> io::Result<()> {
    // Without this, cargo would watch the whole package and rerun the build
    // script every time it writes its own output.
    println!("cargo:rerun-if-changed=src/cli.rs");
    println!("cargo:rerun-if-changed=build.rs");

    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let man_dir = root.join("man");
    let completions_dir = root.join("completions");
    fs::create_dir_all(&man_dir)?;
    fs::create_dir_all(&completions_dir)?;

    write_man(&man_dir)?;

    let mut command = Cli::command();
    for shell in [Shell::Bash, Shell::Zsh, Shell::Fish] {
        generate_to(shell, &mut command, "unpushed", &completions_dir)?;
    }

    Ok(())
}

fn write_man(dir: &Path) -> io::Result<()> {
    let mut page = Vec::new();
    clap_mangen::Man::new(Cli::command()).render(&mut page)?;
    fs::write(dir.join("unpushed.1"), page)
}
