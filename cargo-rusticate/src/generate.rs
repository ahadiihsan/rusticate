//! Creating a project: copy the template, give it the new identity, start its
//! git history. A failure leaves the destination as it was found.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, ensure};

use crate::identity::Identity;
use crate::modules::{self, Modules, Request, Selection};
use crate::name::{Owner, ProjectName};
use crate::template::{Fetched, Source};
use crate::{git, tree};

#[derive(Debug)]
pub(crate) struct Plan {
    pub(crate) name: ProjectName,
    pub(crate) owner: Option<Owner>,
    pub(crate) source: Source,
    pub(crate) init_git: bool,
    pub(crate) modules: Request,
    /// Ask which modules to include when `modules` is empty.
    pub(crate) interactive: bool,
}

#[derive(Debug)]
pub(crate) struct Report {
    pub(crate) files: usize,
    pub(crate) renamed: usize,
    pub(crate) git_initialised: bool,
    /// e.g. "core + postgres"; empty for templates without modules.
    pub(crate) modules: String,
    /// Whether the project has local services to start (`db up`).
    pub(crate) has_database: bool,
    /// Whether rustfmt tidied the Rust files the generator changed.
    pub(crate) formatted: bool,
}

/// The template, fetched, and the modules chosen from it.
#[derive(Debug)]
struct Prepared {
    fetched: Fetched,
    modules: Modules,
    selection: Selection,
}

/// Fetches the template and decides the modules (asking, if interactive),
/// before anything is written to the destination.
fn prepare(plan: &Plan) -> Result<Prepared> {
    let fetched = plan.source.fetch()?;
    let modules = Modules::load(&fetched.root)?;
    let selection = modules::choose(&modules, &plan.modules, plan.interactive)?;
    Ok(Prepared {
        fetched,
        modules,
        selection,
    })
}

/// `cargo rusticate new`: `dest` must not exist, or be an empty directory.
pub(crate) fn into_new_dir(plan: &Plan, dest: &Path) -> Result<Report> {
    if dest.exists() {
        ensure!(
            dest.is_dir(),
            "`{}` already exists and is not a directory",
            dest.display()
        );
        let found = tree::unexpected(dest, &[])?;
        ensure!(
            found.is_empty(),
            "`{}` already exists and is not empty (it contains {})",
            dest.display(),
            listing(&found)
        );
        let prepared = prepare(plan)?;
        generate_or_restore(plan, &prepared, dest, Restore::Clear)
    } else {
        let prepared = prepare(plan)?;
        fs::create_dir_all(dest).with_context(|| format!("creating {}", dest.display()))?;
        generate_or_restore(plan, &prepared, dest, Restore::Remove)
    }
}

/// `cargo rusticate init`: `dest` may contain nothing but `.git` (a freshly
/// cloned empty repository), which is kept as it is.
pub(crate) fn into_existing_dir(plan: &Plan, dest: &Path) -> Result<Report> {
    let found = tree::unexpected(dest, &[".git"])?;
    ensure!(
        found.is_empty(),
        "`{}` is not empty (it contains {}): `cargo rusticate init` needs a directory with \
         nothing but `.git` (use `cargo rusticate new <name>` to create a new directory)",
        dest.display(),
        listing(&found)
    );
    let prepared = prepare(plan)?;
    generate_or_restore(plan, &prepared, dest, Restore::Clear)
}

/// `a`, `b`, `c` and 2 more: enough to see what is in the way.
fn listing(names: &[String]) -> String {
    const SHOWN: usize = 3;
    let shown: Vec<String> = names
        .iter()
        .take(SHOWN)
        .map(|name| format!("`{name}`"))
        .collect();
    match names.len().saturating_sub(SHOWN) {
        0 => shown.join(", "),
        more => format!("{} and {more} more", shown.join(", ")),
    }
}

/// How to undo a failed generation.
#[derive(Debug, Clone, Copy)]
enum Restore {
    /// We created the directory: remove it.
    Remove,
    /// It existed (empty, or with only `.git`): empty it again, keeping `.git`.
    Clear,
}

fn generate_or_restore(
    plan: &Plan,
    prepared: &Prepared,
    dest: &Path,
    restore: Restore,
) -> Result<Report> {
    generate(plan, prepared, dest).map_err(|err| {
        let restored = match restore {
            Restore::Remove => fs::remove_dir_all(dest).map_err(anyhow::Error::from),
            Restore::Clear => tree::clear(dest, &[".git"]),
        };
        match restored {
            Ok(()) => err,
            Err(cleanup) => err.context(format!(
                "and cleaning up `{}` failed too: {cleanup:#}",
                dest.display()
            )),
        }
    })
}

fn generate(plan: &Plan, prepared: &Prepared, dest: &Path) -> Result<Report> {
    let Prepared {
        fetched,
        modules,
        selection,
    } = prepared;
    let files = fetched.copy_into(dest, &modules::dropped_paths(modules, selection))?;
    let keep = |block: &_| match modules::check_block(modules, block) {
        Some(problem) => Err(problem),
        None => Ok(selection.keeps(block)),
    };
    let changed = Identity::new(&plan.name, plan.owner.as_ref()).apply_to_tree(dest, &keep)?;
    let formatted = format_rust(dest, &changed);
    let git_initialised = plan.init_git && !dest.join(".git").exists();
    if git_initialised {
        git::init(dest)?;
    }
    Ok(Report {
        files,
        renamed: changed.len(),
        formatted,
        git_initialised,
        modules: selection.describe(modules),
        has_database: modules.is_empty() || selection.includes("postgres"),
    })
}

/// Removing a module's lines can leave code that rustfmt lays out
/// differently, so the changed Rust files are formatted with the project's
/// `rustfmt.toml`. Best effort: without rustfmt the user is asked to run it.
fn format_rust(dest: &Path, changed: &[PathBuf]) -> bool {
    let files: Vec<&PathBuf> = changed
        .iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    files.is_empty()
        || Command::new("rustfmt")
            .current_dir(dest)
            .args(&files)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
}
