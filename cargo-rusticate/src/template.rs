//! Getting the template's files: a released version from git, or a local
//! checkout while working on the template itself.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, ensure};

use crate::links::WebSource;
use crate::{git, tree};

pub(crate) const DEFAULT_TEMPLATE: &str = "https://github.com/ahadiihsan/rusticate.git";

/// Paths that exist only to build and release the template itself: the
/// generator, the workflow that publishes it and the module manifest. Never
/// copied.
const TEMPLATE_ONLY_PATHS: [&str; 3] = [
    "cargo-rusticate",
    ".github/workflows/release.yml",
    crate::modules::MANIFEST,
];

/// Files every rusticate template has. Their absence means a wrong `--template`.
const REQUIRED: [&str; 2] = ["AGENTS.md", "architecture.toml"];

#[derive(Debug)]
pub(crate) enum Source {
    Git {
        url: String,
        git_ref: Option<String>,
    },
    Local(PathBuf),
}

impl Source {
    /// A directory is a local checkout; anything else is a git URL. The default
    /// template is pinned to the release matching this generator's version.
    pub(crate) fn resolve(template: &str, git_ref: Option<String>) -> Result<Self> {
        let path = Path::new(template);
        if path.is_dir() {
            ensure!(
                git_ref.is_none(),
                "--ref applies to git templates; for a local checkout, check the ref out there"
            );
            return Ok(Self::Local(path.to_path_buf()));
        }
        let git_ref = git_ref.or_else(|| (template == DEFAULT_TEMPLATE).then(release_tag));
        Ok(Self::Git {
            url: template.to_owned(),
            git_ref,
        })
    }

    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Git {
                url,
                git_ref: Some(git_ref),
            } => format!("{url} at {git_ref}"),
            Self::Git { url, git_ref: None } => url.clone(),
            Self::Local(dir) => format!("local checkout {}", dir.display()),
        }
    }

    /// Gets the template's files, ready to be copied. A git template is
    /// fetched into a scratch directory that lives as long as the result.
    pub(crate) fn fetch(&self) -> Result<Fetched> {
        match self {
            Self::Local(dir) => Ok(Fetched {
                root: dir.clone(),
                files: listed_files(dir)?,
                web: local_web_source(dir),
                _staging: None,
            }),
            Self::Git { url, git_ref } => {
                let staging = Staging::create()?;
                fetch(url, git_ref.as_deref(), staging.path())?;
                let files = tree::entries(staging.path())?
                    .into_iter()
                    .map(|entry| entry.path)
                    .collect();
                let revision = match git_ref {
                    Some(git_ref) => git_ref.clone(),
                    None => head_commit(staging.path()).unwrap_or_else(|| "HEAD".to_owned()),
                };
                Ok(Fetched {
                    root: staging.path().to_path_buf(),
                    files,
                    web: WebSource::from_remote(url, &revision),
                    _staging: Some(staging),
                })
            }
        }
    }
}

/// A template's files, before they are copied into a project.
#[derive(Debug)]
pub(crate) struct Fetched {
    pub(crate) root: PathBuf,
    files: Vec<PathBuf>,
    /// Where these files can be browsed, if on GitHub.
    pub(crate) web: Option<WebSource>,
    _staging: Option<Staging>,
}

impl Fetched {
    /// Copies every file into `dest` (which must exist) except template-only
    /// paths and the paths in `excluded`. Returns how many were copied.
    pub(crate) fn copy_into(&self, dest: &Path, excluded: &[&str]) -> Result<usize> {
        copy_files(&self.root, &self.files, dest, excluded)
    }
}

/// A local checkout's `origin` on GitHub, at its current commit.
fn local_web_source(dir: &Path) -> Option<WebSource> {
    let remote = git::output(dir, &["remote", "get-url", "origin"]).ok()?;
    let remote = String::from_utf8(remote).ok()?;
    WebSource::from_remote(remote.trim(), &head_commit(dir)?)
}

fn head_commit(dir: &Path) -> Option<String> {
    let sha = git::output(dir, &["rev-parse", "HEAD"]).ok()?;
    Some(String::from_utf8(sha).ok()?.trim().to_owned())
}

/// The template release this generator belongs to.
pub(crate) fn release_tag() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

fn fetch(url: &str, git_ref: Option<&str>, into: &Path) -> Result<()> {
    git::shallow_checkout(url, git_ref, into).with_context(|| {
        let at = git_ref.map(|r| format!(" at `{r}`")).unwrap_or_default();
        format!(
            "could not fetch the template from {url}{at} (pass `--ref main` for the latest \
             unreleased template, or `--template` for another source)"
        )
    })
}

/// Tracked and untracked-but-not-ignored files of a local checkout: its
/// working tree as a commit would see it, without `target/` and the like.
fn listed_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let stdout = git::output(
        dir,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )
    .with_context(|| {
        format!(
            "`{}` is not a git checkout: a local template must be one",
            dir.display()
        )
    })?;
    let listing = String::from_utf8(stdout).context("non UTF-8 path in the template")?;
    Ok(listing
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .collect())
}

fn copy_files(from: &Path, files: &[PathBuf], dest: &Path, excluded: &[&str]) -> Result<usize> {
    for required in REQUIRED {
        ensure!(
            files.iter().any(|file| file == Path::new(required)),
            "{} is not a rusticate template (no {required})",
            from.display()
        );
    }
    let mut copied = 0;
    for file in files {
        if is_template_only(file) || excluded.iter().any(|path| file.starts_with(path)) {
            continue;
        }
        ensure!(
            file.components()
                .all(|part| matches!(part, Component::Normal(_))),
            "refusing to copy `{}`: not a plain relative path",
            file.display()
        );
        let source = from.join(file);
        let Ok(meta) = fs::symlink_metadata(&source) else {
            continue; // listed by git but deleted in the working tree
        };
        let target = dest.join(file);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        if meta.file_type().is_symlink() {
            copy_symlink(&source, &target)?;
        } else {
            fs::copy(&source, &target).with_context(|| format!("copying {}", file.display()))?;
        }
        copied += 1;
    }
    Ok(copied)
}

/// Matches whole path components: `cargo-rusticate/..` but not `cargo-rusticate.md`.
fn is_template_only(file: &Path) -> bool {
    TEMPLATE_ONLY_PATHS
        .iter()
        .any(|path| file.starts_with(path))
}

#[cfg(unix)]
fn copy_symlink(source: &Path, target: &Path) -> Result<()> {
    let link =
        fs::read_link(source).with_context(|| format!("reading link {}", source.display()))?;
    std::os::unix::fs::symlink(&link, target)
        .with_context(|| format!("creating link {}", target.display()))
}

#[cfg(not(unix))]
fn copy_symlink(source: &Path, _target: &Path) -> Result<()> {
    bail!(
        "`{}` is a symlink; copying symlinks from a local template needs a Unix system \
         (the default git template works everywhere)",
        source.display()
    )
}

/// A scratch directory for the clone, removed when dropped.
#[derive(Debug)]
struct Staging(PathBuf);

impl Staging {
    fn create() -> Result<Self> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let dir =
            std::env::temp_dir().join(format!("cargo-rusticate-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(Self(dir))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        // Best effort: a leftover scratch directory in the temp dir is harmless.
        fs::remove_dir_all(&self.0).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_template_is_pinned_to_this_release() {
        let source = Source::resolve(DEFAULT_TEMPLATE, None).unwrap();
        let expected = format!("v{}", env!("CARGO_PKG_VERSION"));
        assert!(matches!(source, Source::Git { git_ref: Some(r), .. } if r == expected));
    }

    #[test]
    fn explicit_ref_wins_and_other_urls_are_not_pinned() {
        let pinned = Source::resolve(DEFAULT_TEMPLATE, Some("main".into())).unwrap();
        assert!(matches!(pinned, Source::Git { git_ref: Some(r), .. } if r == "main"));
        let other = Source::resolve("https://example.com/fork.git", None).unwrap();
        assert!(matches!(other, Source::Git { git_ref: None, .. }));
    }

    #[test]
    fn a_directory_is_a_local_template_without_ref() {
        let here = env!("CARGO_MANIFEST_DIR");
        assert!(matches!(
            Source::resolve(here, None).unwrap(),
            Source::Local(_)
        ));
        assert!(Source::resolve(here, Some("main".into())).is_err());
    }

    #[test]
    fn template_only_paths_match_whole_components() {
        assert!(is_template_only(Path::new("cargo-rusticate/Cargo.toml")));
        assert!(is_template_only(Path::new(".github/workflows/release.yml")));
        assert!(!is_template_only(Path::new(".github/workflows/ci.yml")));
        assert!(!is_template_only(Path::new(
            "docs/cargo-rusticate/notes.md"
        )));
        assert!(!is_template_only(Path::new("cargo-rusticate.md")));
    }
}
