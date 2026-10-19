//! Generates the man page and shell completions from the command line
//! definition, so they cannot drift from `--help`.
//!
//! Output goes to `OUT_DIR`, and additionally to `man/` and `completions/` next
//! to the manifest, because the Debian package and the release tarballs
//! reference those paths directly. Both directories are generated artefacts and
//! are git-ignored.
//!
//! Writing beside the manifest is skipped when the manifest directory is not
//! writable, and when building from a packaged crate — `cargo publish` verifies
//! that a build script leaves the source tree alone, and rightly rejects one
//! that does not.

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

    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("set by cargo"));
    generate(&out_dir)?;

    // Also beside the manifest, where packaging picks them up — but never from
    // a packaged crate, whose source tree must stay untouched. A read-only
    // checkout is not a reason to fail either: OUT_DIR already has the copies.
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("set by cargo"));
    if !is_packaged_build(&root) {
        let _ = generate(&root);
    }

    Ok(())
}

/// True when building from an unpacked `.crate`, which cargo stages under
/// `target/package/` during `cargo publish` and in the registry cache.
fn is_packaged_build(root: &Path) -> bool {
    root.join(".cargo_vcs_info.json").exists()
        || root
            .components()
            .any(|c| c.as_os_str() == "package" || c.as_os_str() == "registry")
}

fn generate(dir: &Path) -> io::Result<()> {
    let man_dir = dir.join("man");
    let completions_dir = dir.join("completions");
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
