//! `cargo rusticate`: start a project from the rusticate boilerplate.
//!
//! Cargo runs `cargo rusticate <args>` as `cargo-rusticate rusticate <args>`;
//! running the binary directly (`cargo-rusticate <args>`) works too.

mod generate;
mod identity;
mod name;
mod template;
mod tree;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::{Args, Parser, Subcommand};

use crate::generate::{Plan, Report};
use crate::name::{Owner, ProjectName};
use crate::template::Source;

#[derive(Debug, Parser)]
#[command(name = "cargo", bin_name = "cargo")]
enum Cargo {
    /// Start a production-ready multi-tenant Rust backend from the rusticate boilerplate.
    #[command(version)]
    Rusticate(Rusticate),
}

#[derive(Debug, Args)]
struct Rusticate {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a project in a new directory.
    New {
        /// Project name: lowercase letters, digits and single hyphens (e.g. `my-service`).
        name: String,
        /// Directory to create [default: ./<name>].
        #[arg(long)]
        path: Option<PathBuf>,
        #[command(flatten)]
        common: Common,
    },
    /// Fill the current directory, which may contain only `.git` (e.g. a freshly cloned empty repository).
    Init {
        /// Project name [default: the directory's name].
        #[arg(long)]
        name: Option<String>,
        #[command(flatten)]
        common: Common,
    },
}

#[derive(Debug, Args)]
struct Common {
    /// GitHub user or organisation that will own the repository (CODEOWNERS, SECURITY.md).
    #[arg(long)]
    owner: Option<String>,
    /// Template to start from: a git URL, or a local rusticate checkout.
    #[arg(long, default_value = template::DEFAULT_TEMPLATE)]
    template: String,
    /// Tag or branch of a git template [default: this version's release tag, for the default template].
    #[arg(long = "ref", value_name = "REF")]
    git_ref: Option<String>,
    /// Do not initialise a git repository.
    #[arg(long)]
    no_git: bool,
}

fn main() -> ExitCode {
    let Cargo::Rusticate(cli) = Cargo::parse_from(as_cargo_subcommand(std::env::args_os()));
    match run(cli.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

/// Inserts the `rusticate` argument cargo would pass, when invoked directly.
fn as_cargo_subcommand(args: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    let mut args: Vec<OsString> = args.into_iter().collect();
    if args.get(1).is_none_or(|first| first != "rusticate") {
        let at = args.len().min(1);
        args.insert(at, "rusticate".into());
    }
    args
}

fn run(command: Command) -> Result<()> {
    match command {
        Command::New { name, path, common } => {
            let name = ProjectName::parse(&name)?;
            let dest = path.unwrap_or_else(|| PathBuf::from(name.as_str()));
            let plan = plan(name, common)?;
            let report = generate::into_new_dir(&plan, &dest)?;
            print_report(&plan, &report, Some(&dest));
        }
        Command::Init { name, common } => {
            let dest = std::env::current_dir().context("reading the current directory")?;
            let name = match name {
                Some(name) => ProjectName::parse(&name)?,
                None => ProjectName::parse(&dir_name(&dest)?)
                    .context("the directory name is not a valid project name; pass --name")?,
            };
            let plan = plan(name, common)?;
            let report = generate::into_existing_dir(&plan, &dest)?;
            print_report(&plan, &report, None);
        }
    }
    Ok(())
}

fn plan(name: ProjectName, common: Common) -> Result<Plan> {
    Ok(Plan {
        name,
        owner: common.owner.as_deref().map(Owner::parse).transpose()?,
        source: Source::resolve(&common.template, common.git_ref)?,
        init_git: !common.no_git,
    })
}

fn dir_name(dir: &Path) -> Result<String> {
    dir.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .context("the current directory has no usable name; pass --name")
}

fn print_report(plan: &Plan, report: &Report, created: Option<&Path>) {
    println!(
        "✓ {} created from {} ({} files, {} renamed{})",
        plan.name.as_str(),
        plan.source.describe(),
        report.files,
        report.renamed,
        if report.git_initialised {
            ", new git repository"
        } else {
            ""
        },
    );
    if plan.owner.is_none() {
        eprintln!(
            "warning: no --owner given: CODEOWNERS and SECURITY.md name `{}`; \
             replace it with your GitHub user or organisation",
            identity::OWNER_PLACEHOLDER
        );
    }
    println!("\nNext:");
    if let Some(dir) = created {
        println!("  cd {}", dir.display());
    }
    println!("  cargo xtask setup     # git hooks; lists any missing tool");
    println!("  cargo xtask db up     # Postgres and NATS in Docker");
    println!("  cargo xtask verify    # the definition of done");
    println!("\nThen read AGENTS.md: it is the operating manual for people and coding agents.");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn cargo_invocation_is_left_as_is() {
        let cargo = args(&["cargo-rusticate", "rusticate", "new", "x"]);
        assert_eq!(as_cargo_subcommand(cargo.clone()), cargo);
    }

    #[test]
    fn direct_invocation_gains_the_subcommand_name() {
        assert_eq!(
            as_cargo_subcommand(args(&["cargo-rusticate", "new", "x"])),
            args(&["cargo-rusticate", "rusticate", "new", "x"])
        );
        assert_eq!(
            as_cargo_subcommand(args(&["cargo-rusticate"])),
            args(&["cargo-rusticate", "rusticate"])
        );
    }
}
