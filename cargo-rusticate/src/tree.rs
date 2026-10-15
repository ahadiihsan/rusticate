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

/// Files operating systems drop into any folder a file manager opens. They
/// never make a directory "not empty", and they are never removed.
const OS_CLUTTER: [&str; 3] = [".DS_Store", "Thumbs.db", "desktop.ini"];

/// The names in `dir` other than those in `allowed` and OS clutter, sorted.
pub(crate) fn unexpected(dir: &Path, allowed: &[&str]) -> Result<Vec<String>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let name = entry?.file_name();
        if !is_kept(&name, allowed) {
            found.push(name.to_string_lossy().into_owned());
        }
    }
    found.sort();
    Ok(found)
}

fn is_kept(name: &std::ffi::OsStr, allowed: &[&str]) -> bool {
    allowed.iter().chain(&OS_CLUTTER).any(|keep| name == *keep)
}

/// Removes everything in `dir` except the names in `keep` and OS clutter.
pub(crate) fn clear(dir: &Path, keep: &[&str]) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        if is_kept(&entry.file_name(), keep) {
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
