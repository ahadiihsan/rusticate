//! Inside a project, `cargo rusticate <command>` is the project's own
//! `cargo xtask <command>`.
//!
//! The commands themselves (verify, db, new domain, ...) live in each
//! project's `xtask`, versioned with its code and CI: a project keeps the rules
//! it was generated with, whatever version of this tool is installed. This
//! module only decides where a command goes and forwards it.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context as _, Result};

/// `cargo rusticate new <kind> ..` inside a project scaffolds instead of
/// creating a project.
const SCAFFOLD_KINDS: [&str; 3] = ["domain", "feature", "migration"];

/// Commands this binary answers itself.
const GENERATOR_COMMANDS: [&str; 4] = ["new", "init", "upgrade", "help"];

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Route {
    /// Creating projects (and `--version`): handled here.
    Generator,
    /// Run the project's `cargo xtask` with the same arguments.
    Project,
    /// A project command used outside any project.
    NotInProject(String),
}

/// The root of the rusticate project containing `dir`, if any: the nearest
/// ancestor with both `architecture.toml` and `xtask/Cargo.toml`.
pub(crate) fn find_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|candidate| {
            candidate.join("architecture.toml").is_file()
                && candidate.join("xtask").join("Cargo.toml").is_file()
        })
        .map(Path::to_path_buf)
}

/// Where `args` (everything after `cargo rusticate`) should go.
pub(crate) fn route(args: &[OsString], in_project: bool) -> Route {
    let word = |at: usize| args.get(at).and_then(|arg| arg.to_str());
    match word(0) {
        Some("-V" | "--version" | "init" | "upgrade") => Route::Generator,
        Some("new") => {
            // Inside a project, `new` alone or with a flag (`new --help`)
            // means scaffolding too; only a project name creates a project.
            let scaffolds = match args.get(1) {
                None => true,
                Some(next) => next
                    .to_str()
                    .is_none_or(|next| next.starts_with('-') || SCAFFOLD_KINDS.contains(&next)),
            };
            if in_project && scaffolds {
                Route::Project
            } else {
                Route::Generator
            }
        }
        _ if in_project => Route::Project,
        Some(command) if !command.starts_with('-') && !GENERATOR_COMMANDS.contains(&command) => {
            Route::NotInProject(command.to_owned())
        }
        _ => Route::Generator,
    }
}

/// Runs `cargo xtask <args>` from the current directory and returns its exit
/// code. Plain `cargo` (not `$CARGO`) so rustup honours the project's
/// `rust-toolchain.toml`.
pub(crate) fn forward(args: &[OsString]) -> Result<ExitCode> {
    let status = Command::new("cargo")
        .arg("xtask")
        .args(args)
        .status()
        .context("could not run `cargo xtask`")?;
    Ok(status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .map_or(ExitCode::FAILURE, ExitCode::from))
}

pub(crate) fn not_in_project_message(command: &str, cwd: &Path) -> String {
    format!(
        "`{command}` is a project command: run it inside a rusticate project \
         (no directory with architecture.toml and xtask/ from {} upwards). \
         To create one: cargo rusticate new <name>",
        cwd.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn inside_a_project_commands_go_to_its_xtask() {
        for command in [
            &["db", "up"][..],
            &["verify", "--fast"],
            &["new", "domain", "shipping"],
            &[
                "new",
                "feature",
                "shipping",
                "dispatch_parcel",
                "--kind",
                "command",
            ],
            &["new", "migration", "shipping", "create shipments"],
            &["new", "--help"],
            &["new"],
            &["run", "api"],
            &[],
            &["--help"],
        ] {
            assert_eq!(route(&args(command), true), Route::Project, "{command:?}");
        }
    }

    #[test]
    fn creating_projects_and_the_version_stay_here() {
        for (command, in_project) in [
            (&["new", "my-service"][..], false),
            (&["new", "my-service"], true),
            (&["new", "domain"], false),
            (&["init"], true),
            (&["upgrade"], true),
            (&["upgrade", "--to", "v9"], true),
            (&["init", "--owner", "acme"], false),
            (&["--version"], true),
            (&["-V"], false),
            (&["--help"], false),
            (&["help"], false),
            (&[], false),
        ] {
            assert_eq!(
                route(&args(command), in_project),
                Route::Generator,
                "{command:?}"
            );
        }
    }

    #[test]
    fn project_commands_outside_a_project_are_explained() {
        assert_eq!(
            route(&args(&["db", "up"]), false),
            Route::NotInProject("db".into())
        );
        let message = not_in_project_message("db", Path::new("/tmp"));
        assert!(message.contains("inside a rusticate project"), "{message}");
    }

    #[test]
    fn the_nearest_ancestor_with_the_markers_is_the_root() {
        let base =
            std::env::temp_dir().join(format!("cargo-rusticate-root-{}", std::process::id()));
        let nested = base.join("crates/domains/billing/src");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(base.join("xtask")).unwrap();
        std::fs::write(base.join("architecture.toml"), "").unwrap();
        assert_eq!(find_root(&nested), None, "xtask/Cargo.toml is missing");
        std::fs::write(base.join("xtask/Cargo.toml"), "").unwrap();
        assert_eq!(find_root(&nested), Some(base.clone()));
        std::fs::remove_dir_all(&base).unwrap();
    }
}
