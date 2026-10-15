//! `cargo rusticate upgrade`: bring a newer template's tooling and agent
//! harness into an existing project, keeping the project's own changes.
//!
//! The project's `rusticate.toml` says which template revision and modules it
//! came from. The template is generated twice into scratch directories, at
//! that revision ("base") and at the new one ("theirs"), with the project's
//! name, owner and modules, so both match the project's naming exactly. Each
//! tooling file is then merged three ways with the project's copy ("ours").
//! Product code (apps, crates, migrations) is never touched.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail, ensure};
use serde::Deserialize;

use crate::generate::{self, Plan};
use crate::modules::Request;
use crate::name::{Owner, ProjectName};
use crate::record::{self, RECORD};
use crate::template::{self, Source};
use crate::{git, tree};

/// What an upgrade may change: tooling, checks, scaffolds and the agent
/// harness. Not product code, and not the project's own ADRs.
const UPGRADE_PATHS: [&str; 18] = [
    "xtask",
    ".agents",
    ".claude",
    ".gemini",
    ".github",
    ".githooks",
    ".config",
    ".cargo",
    "scripts",
    "docs/sop",
    "docs/configuration.md",
    "docs/operations.md",
    "AGENTS.md",
    "CLAUDE.md",
    "Cargo.toml",
    "clippy.toml",
    "deny.toml",
    "rustfmt.toml",
];

/// Never merged: owned by the project (or by the user's local settings).
const EXCLUDED: [&str; 2] = [".claude/settings.local.json", RECORD];

#[derive(Debug, Deserialize)]
struct RecordFile {
    template: RecordTemplate,
    project: RecordProject,
}

#[derive(Debug, Deserialize)]
struct RecordTemplate {
    source: String,
    revision: Option<String>,
    modules: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RecordProject {
    name: String,
    owner: Option<String>,
}

/// What happened to one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Unchanged,
    Updated,
    Added,
    Removed,
    Merged,
    Conflict,
    /// Removed from the template but changed locally: left in place.
    KeptLocal,
    /// Added by the template where the project already has a different file.
    AddedConflict,
}

/// Runs the upgrade in `project` to `to` (default: this generator's release).
pub(crate) fn run(project: &Path, to: Option<String>) -> Result<()> {
    let recorded = read_record(project)?;
    ensure_clean(project)?;
    let source = recorded.template.source.as_str();
    ensure!(
        !Path::new(source).is_dir(),
        "this project was generated from a local checkout ({source}); upgrades need a git template"
    );
    let from = recorded
        .template
        .revision
        .clone()
        .context("rusticate.toml records no template revision")?;
    let to = to.unwrap_or_else(|| {
        if source == template::DEFAULT_TEMPLATE {
            template::release_tag()
        } else {
            "main".to_owned()
        }
    });
    ensure!(from != to, "already at {to}");

    let scratch = tempdir()?;
    let base = scratch.join("base");
    let theirs = scratch.join("theirs");
    let outcome = generate_at(&recorded, &from, &base)
        .and_then(|()| generate_at(&recorded, &to, &theirs))
        .and_then(|()| merge_tree(project, &base, &theirs));
    fs::remove_dir_all(&scratch).ok();
    let outcomes = outcome?;

    rewrite_record(project, &recorded, &to)?;
    print_summary(&from, &to, &outcomes);
    Ok(())
}

fn read_record(project: &Path) -> Result<RecordFile> {
    let path = project.join(RECORD);
    let text = fs::read_to_string(&path).with_context(|| {
        format!(
            "{} has no {RECORD}: projects generated before cargo-rusticate 0.3.1 cannot be upgraded automatically",
            project.display()
        )
    })?;
    toml::from_str(&text).with_context(|| format!("parsing {RECORD}"))
}

fn ensure_clean(project: &Path) -> Result<()> {
    let status = git::output(project, &["status", "--porcelain"])
        .context("the project must be a git repository")?;
    ensure!(
        status.is_empty(),
        "commit or stash your changes first: the upgrade must be reviewable with `git diff`"
    );
    Ok(())
}

/// The template at `revision`, generated with the project's identity and modules.
fn generate_at(recorded: &RecordFile, revision: &str, dest: &Path) -> Result<()> {
    let plan = Plan {
        name: ProjectName::parse(&recorded.project.name)?,
        owner: recorded
            .project
            .owner
            .as_deref()
            .map(Owner::parse)
            .transpose()?,
        source: Source::resolve(&recorded.template.source, Some(revision.to_owned()))?,
        init_git: false,
        modules: Request {
            with: recorded.template.modules.clone(),
            without: Vec::new(),
            minimal: true,
        },
        interactive: false,
    };
    generate::into_new_dir(&plan, dest)
        .with_context(|| format!("generating the template at {revision}"))
        .map(drop)
}

fn in_scope(path: &Path) -> bool {
    UPGRADE_PATHS.iter().any(|scope| path.starts_with(scope))
        && !EXCLUDED.iter().any(|excluded| path == Path::new(excluded))
}

fn files_in_scope(root: &Path) -> Result<BTreeSet<PathBuf>> {
    if !root.exists() {
        return Ok(BTreeSet::new());
    }
    // Symlinks (e.g. `.claude/skills`) are structure, not content: left alone.
    Ok(tree::entries(root)?
        .into_iter()
        .filter(|entry| !entry.is_symlink)
        .map(|entry| entry.path)
        .filter(|path| in_scope(path))
        .collect())
}

fn merge_tree(ours: &Path, base: &Path, theirs: &Path) -> Result<Vec<(PathBuf, Outcome)>> {
    let mut paths = files_in_scope(base)?;
    paths.extend(files_in_scope(theirs)?);
    let mut outcomes = Vec::new();
    for path in paths {
        let outcome = merge_file(&ours.join(&path), &base.join(&path), &theirs.join(&path))
            .with_context(|| format!("merging {}", path.display()))?;
        if outcome != Outcome::Unchanged {
            outcomes.push((path, outcome));
        }
    }
    Ok(outcomes)
}

fn read(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(Some(fs::read(path)?)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn merge_file(ours_path: &Path, base_path: &Path, theirs_path: &Path) -> Result<Outcome> {
    let (ours, base, theirs) = (read(ours_path)?, read(base_path)?, read(theirs_path)?);
    let write = |content: &[u8]| -> Result<()> {
        if let Some(parent) = ours_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(ours_path, content)?;
        Ok(())
    };
    Ok(
        match decide(ours.as_deref(), base.as_deref(), theirs.as_deref()) {
            Decision::Keep => Outcome::Unchanged,
            Decision::Take(outcome) => {
                if let Some(content) = &theirs {
                    write(content)?;
                }
                outcome
            }
            Decision::Delete => {
                fs::remove_file(ours_path)?;
                Outcome::Removed
            }
            Decision::KeepLocal => Outcome::KeptLocal,
            Decision::Merge => {
                let (merged, conflicts) = three_way(ours_path, base_path, theirs_path)?;
                write(&merged)?;
                if conflicts {
                    Outcome::Conflict
                } else {
                    Outcome::Merged
                }
            }
            Decision::AddConflict => {
                let empty = std::env::temp_dir().join("cargo-rusticate-empty-base");
                fs::write(&empty, b"")?;
                let (merged, _) = three_way(ours_path, &empty, theirs_path)?;
                write(&merged)?;
                Outcome::AddedConflict
            }
        },
    )
}

#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Keep,
    Take(Outcome),
    Delete,
    KeepLocal,
    Merge,
    AddConflict,
}

/// The rule table of a three-way merge, by content.
fn decide(ours: Option<&[u8]>, base: Option<&[u8]>, theirs: Option<&[u8]>) -> Decision {
    match (ours, base, theirs) {
        // The template did not change it; or the project already has the new
        // version; or the project deleted it (its deletion wins).
        (_, b, t) if b == t => Decision::Keep,
        (Some(o), _, Some(t)) if o == t => Decision::Keep,
        (None, Some(_), _) | (_, None, None) => Decision::Keep,
        // New in the template.
        (None, None, Some(_)) => Decision::Take(Outcome::Added),
        (Some(_), None, Some(_)) => Decision::AddConflict,
        // Removed from the template.
        (Some(o), Some(b), None) if o == b => Decision::Delete,
        (Some(_), Some(_), None) => Decision::KeepLocal,
        // Changed in the template.
        (Some(o), Some(b), Some(_)) if o == b => Decision::Take(Outcome::Updated),
        (Some(_), Some(_), Some(_)) => Decision::Merge,
    }
}

/// `git merge-file`: the merged text, and whether it has conflicts.
fn three_way(ours: &Path, base: &Path, theirs: &Path) -> Result<(Vec<u8>, bool)> {
    let output = Command::new("git")
        .args([
            "merge-file",
            "-p",
            "-L",
            "yours",
            "-L",
            "previous template",
            "-L",
            "new template",
        ])
        .arg(ours)
        .arg(base)
        .arg(theirs)
        .output()
        .context("could not run `git merge-file`")?;
    match output.status.code() {
        Some(0) => Ok((output.stdout, false)),
        Some(code) if code > 0 => Ok((output.stdout, true)),
        _ => bail!(
            "git merge-file failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
    }
}

fn rewrite_record(project: &Path, recorded: &RecordFile, to: &str) -> Result<()> {
    record::write(
        project,
        &record::Origin {
            source: &recorded.template.source,
            revision: Some(to),
            modules: recorded
                .template
                .modules
                .iter()
                .map(String::as_str)
                .collect(),
            name: &recorded.project.name,
            owner: recorded.project.owner.as_deref(),
        },
    )
}

fn print_summary(from: &str, to: &str, outcomes: &[(PathBuf, Outcome)]) {
    let count = |kind: Outcome| outcomes.iter().filter(|(_, o)| *o == kind).count();
    println!(
        "✓ upgraded the tooling from {from} to {to}: {} updated, {} added, {} removed, {} merged",
        count(Outcome::Updated),
        count(Outcome::Added),
        count(Outcome::Removed),
        count(Outcome::Merged),
    );
    let attention: Vec<String> = outcomes
        .iter()
        .filter_map(|(path, outcome)| {
            let why = match outcome {
                Outcome::Conflict => "conflict markers: resolve them",
                Outcome::AddedConflict => "you have a different file here: resolve the markers",
                Outcome::KeptLocal => "removed from the template, kept because you changed it",
                _ => return None,
            };
            Some(format!("  {}  ({why})", path.display()))
        })
        .collect();
    if !attention.is_empty() {
        println!("\nNeeds your attention:\n{}", attention.join("\n"));
    }
    println!(
        "\nReview with `git diff`, then run `cargo rusticate setup` and `cargo rusticate verify`."
    );
}

/// A fresh scratch directory under the system temp dir.
fn tempdir() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!(
        "cargo-rusticate-upgrade-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &[u8] = b"a";
    const B: &[u8] = b"b";
    const C: &[u8] = b"c";

    #[test]
    fn three_way_rules() {
        assert_eq!(decide(Some(A), Some(A), Some(A)), Decision::Keep);
        assert_eq!(
            decide(Some(A), Some(A), Some(B)),
            Decision::Take(Outcome::Updated)
        );
        assert_eq!(decide(Some(B), Some(A), Some(B)), Decision::Keep);
        assert_eq!(decide(Some(C), Some(A), Some(B)), Decision::Merge);
        assert_eq!(decide(None, None, Some(B)), Decision::Take(Outcome::Added));
        assert_eq!(decide(Some(B), None, Some(B)), Decision::Keep);
        assert_eq!(decide(Some(C), None, Some(B)), Decision::AddConflict);
        assert_eq!(decide(Some(A), Some(A), None), Decision::Delete);
        assert_eq!(decide(Some(C), Some(A), None), Decision::KeepLocal);
        assert_eq!(decide(None, Some(A), Some(B)), Decision::Keep);
    }

    #[test]
    fn only_tooling_is_in_scope() {
        assert!(in_scope(Path::new("xtask/src/main.rs")));
        assert!(in_scope(Path::new("AGENTS.md")));
        assert!(in_scope(Path::new("docs/sop/rust-backend-sop-v2.md")));
        assert!(!in_scope(Path::new("docs/adr/0003-ours.md")));
        assert!(!in_scope(Path::new("crates/domains/orders/src/lib.rs")));
        assert!(!in_scope(Path::new("apps/api/src/main.rs")));
        assert!(!in_scope(Path::new("rusticate.toml")));
    }
}
