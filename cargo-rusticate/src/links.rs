//! Links into files a dropped module took with it.
//!
//! The template's documents link to code as evidence (the standard cites the
//! example, the platform crates, ...). When a module is left out, those
//! targets are gone; such relative links are pointed at the same file in the
//! template repository, at the exact version the project came from, so they
//! keep working forever.

use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::tree;

/// Where the template's files are browsable: `https://github.com/<owner>/<repo>`
/// and the commit or tag the project was generated from.
#[derive(Debug, Clone)]
pub(crate) struct WebSource {
    pub(crate) repository: String,
    pub(crate) revision: String,
}

impl WebSource {
    /// From a git remote URL (`https://github.com/o/r(.git)` or
    /// `git@github.com:o/r(.git)`); `None` for anything not on GitHub.
    pub(crate) fn from_remote(url: &str, revision: &str) -> Option<Self> {
        let path = url
            .strip_prefix("https://github.com/")
            .or_else(|| url.strip_prefix("git@github.com:"))?;
        let path = path.trim_end_matches('/').trim_end_matches(".git");
        let valid = path.split('/').count() == 2 && !path.split('/').any(str::is_empty);
        valid.then(|| Self {
            repository: format!("https://github.com/{path}"),
            revision: revision.to_owned(),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/blob/{}/{path}", self.repository, self.revision)
    }
}

/// Repoints dangling relative links in every Markdown file under `root`.
/// Returns how many links changed.
pub(crate) fn repoint_dangling(root: &Path, web: &WebSource) -> Result<usize> {
    let mut changed = 0;
    for entry in tree::entries(root)? {
        let is_markdown = entry.path.extension().is_some_and(|ext| ext == "md");
        if entry.is_symlink || !is_markdown {
            continue;
        }
        let file = root.join(&entry.path);
        let text = fs::read_to_string(&file)
            .with_context(|| format!("reading {}", entry.path.display()))?;
        let dir = entry.path.parent().unwrap_or_else(|| Path::new(""));
        let (rewritten, count) = rewrite(
            &text,
            |target| {
                let path = resolve(dir, target)?;
                (!root.join(&path).exists()).then(|| path.to_string_lossy().replace('\\', "/"))
            },
            web,
        );
        if count > 0 {
            fs::write(&file, rewritten)
                .with_context(|| format!("writing {}", entry.path.display()))?;
            changed += count;
        }
    }
    Ok(changed)
}

/// Rewrites `](target)` links for which `dangling` returns the target's path
/// from the root. Fragments (`#...`) are kept.
fn rewrite(
    text: &str,
    dangling: impl Fn(&str) -> Option<String>,
    web: &WebSource,
) -> (String, usize) {
    let mut out = String::with_capacity(text.len());
    let mut count = 0;
    let mut rest = text;
    while let Some(at) = rest.find("](") {
        let (before, after) = rest.split_at(at + 2);
        out.push_str(before);
        let end = after.find([')', ' ', '\n']).unwrap_or(after.len());
        let (target, tail) = after.split_at(end);
        let (path, fragment) = target.split_once('#').map_or((target, ""), |(p, f)| (p, f));
        let relative = !path.is_empty() && !path.contains("://") && !path.starts_with("mailto:");
        match relative.then(|| dangling(path)).flatten() {
            Some(from_root) => {
                out.push_str(&web.url(&from_root));
                if !fragment.is_empty() {
                    out.push('#');
                    out.push_str(fragment);
                }
                count += 1;
            }
            None => out.push_str(target),
        }
        rest = tail;
    }
    out.push_str(rest);
    (out, count)
}

/// `target` relative to `dir`, as a path from the root; `None` if it would
/// leave the root.
fn resolve(dir: &Path, target: &str) -> Option<PathBuf> {
    let joined = dir.join(target);
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    for component in joined.components() {
        match component {
            Component::Normal(part) => parts.push(part),
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(parts.iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn web() -> WebSource {
        WebSource::from_remote("git@github.com:acme/template.git", "v1.2.3").unwrap()
    }

    #[test]
    fn github_remotes_become_browsable_bases() {
        let https = WebSource::from_remote("https://github.com/acme/template.git", "abc").unwrap();
        assert_eq!(
            https.url("a/b.rs"),
            "https://github.com/acme/template/blob/abc/a/b.rs"
        );
        assert!(WebSource::from_remote("https://gitlab.com/acme/template.git", "abc").is_none());
        assert!(WebSource::from_remote("/local/checkout", "abc").is_none());
    }

    #[test]
    fn only_dangling_relative_links_are_repointed() {
        let text = "See [money](../../crates/billing/src/money.rs#L10), [kept](kept.md), \
                    [web](https://example.com/x), [anchor](#top) and [skill](../x/SKILL.md).";
        let dangling = |target: &str| {
            let path = resolve(Path::new("docs/sop"), target)?;
            let path = path.to_string_lossy().into_owned();
            (path != "docs/sop/kept.md").then_some(path)
        };

        let (out, count) = rewrite(text, dangling, &web());

        assert_eq!(count, 2);
        assert_eq!(
            out,
            "See [money](https://github.com/acme/template/blob/v1.2.3/crates/billing/src/money.rs#L10), \
             [kept](kept.md), [web](https://example.com/x), [anchor](#top) and \
             [skill](https://github.com/acme/template/blob/v1.2.3/docs/x/SKILL.md)."
        );
    }

    #[test]
    fn paths_resolve_within_the_root_only() {
        assert_eq!(
            resolve(Path::new("docs/sop"), "../../a/b.rs"),
            Some(PathBuf::from("a/b.rs"))
        );
        assert_eq!(resolve(Path::new("docs"), "../../escape.md"), None);
        assert_eq!(
            resolve(Path::new(""), "./README.md"),
            Some(PathBuf::from("README.md"))
        );
    }
}
