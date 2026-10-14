//! Listing the files of a directory tree.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

#[derive(Debug)]
pub(crate) struct Entry {
    /// Relative to the listed root.
    pub(crate) path: PathBuf,
    pub(crate) is_symlink: bool,
}

/// Every file and symlink under `root`, sorted, skipping the top-level `.git`.
/// Symlinked directories are listed as links, not followed.
pub(crate) fn entries(root: &Path) -> Result<Vec<Entry>> {
    let mut found = Vec::new();
    collect(root, Path::new(""), &mut found)?;
    found.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}

/// Whether `dir` holds nothing but the names in `allowed`.
pub(crate) fn only_contains(dir: &Path, allowed: &[&str]) -> Result<bool> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let name = entry?.file_name();
        if !allowed.iter().any(|keep| name == *keep) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Removes everything in `dir` except the names in `keep`.
pub(crate) fn clear(dir: &Path, keep: &[&str]) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        if keep.iter().any(|name| entry.file_name() == *name) {
            continue;
        }
        let path = entry.path();
        let removed = if entry.file_type()?.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        removed.with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}

fn collect(root: &Path, relative: &Path, found: &mut Vec<Entry>) -> Result<()> {
    let dir = root.join(relative);
    for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let name = entry.file_name();
        if relative.as_os_str().is_empty() && name == ".git" {
            continue;
        }
        let path = relative.join(&name);
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect(root, &path, found)?;
        } else {
            found.push(Entry {
                path,
                is_symlink: kind.is_symlink(),
            });
        }
    }
    Ok(())
}
