//! Creating a project: copy the template, give it the new identity, start its
//! git history. A failure leaves the destination as it was found.

use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result, ensure};

use crate::identity::Identity;
use crate::name::{Owner, ProjectName};
use crate::template::Source;
use crate::{git, tree};

#[derive(Debug)]
pub(crate) struct Plan {
    pub(crate) name: ProjectName,
    pub(crate) owner: Option<Owner>,
    pub(crate) source: Source,
    pub(crate) init_git: bool,
}

#[derive(Debug)]
pub(crate) struct Report {
    pub(crate) files: usize,
    pub(crate) renamed: usize,
    pub(crate) git_initialised: bool,
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
        generate_or_restore(plan, dest, Restore::Clear)
    } else {
        fs::create_dir_all(dest).with_context(|| format!("creating {}", dest.display()))?;
        generate_or_restore(plan, dest, Restore::Remove)
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
    generate_or_restore(plan, dest, Restore::Clear)
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

fn generate_or_restore(plan: &Plan, dest: &Path, restore: Restore) -> Result<Report> {
    generate(plan, dest).map_err(|err| {
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

fn generate(plan: &Plan, dest: &Path) -> Result<Report> {
    let files = plan.source.materialize(dest)?;
    let renamed = Identity::new(&plan.name, plan.owner.as_ref()).apply_to_tree(dest)?;
    let git_initialised = plan.init_git && !dest.join(".git").exists();
    if git_initialised {
        git::init(dest)?;
    }
    Ok(Report {
        files,
        renamed,
        git_initialised,
    })
}
