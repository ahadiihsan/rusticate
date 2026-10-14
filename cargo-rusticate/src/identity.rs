//! The template's identity, and how it becomes the new project's.
//!
//! The template calls itself `rusticate` (and `Rusticate` in titles), lives at
//! `github.com/ahadiihsan/rusticate` and is owned by `@ahadiihsan`. Every
//! occurrence is rewritten, except:
//! - permalinks into the template's history, which must keep pointing there;
//! - the generator's own name (`cargo-rusticate`, `cargo rusticate`);
//! - blocks between template-only markers, which are removed entirely.
//!
//! Any other spelling (`RUSTICATE`, a bare `ahadiihsan`) is an error: the
//! template gained an occurrence these rules do not cover, and leaving it
//! would ship the template's identity inside someone else's project.

use std::borrow::Cow;
use std::fmt;
use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result, ensure};

use crate::name::{Owner, ProjectName};
use crate::tree;

const NAME: &str = "rusticate";
const TITLE: &str = "Rusticate";
const REPOSITORY: &str = "github.com/ahadiihsan/rusticate";
const OWNER: &str = "@ahadiihsan";
const OWNER_LOGIN: &str = "ahadiihsan";

/// Kept verbatim: they name the template's history and the generator itself.
const PROTECTED: [&str; 3] = [
    "https://github.com/ahadiihsan/rusticate/blob/",
    "cargo-rusticate",
    "cargo rusticate",
];

/// Lines (on their own) that delimit content only the template repository has,
/// such as how to install this generator.
const TEMPLATE_ONLY_START: &str = "<!-- rusticate:template-only:start -->";
const TEMPLATE_ONLY_END: &str = "<!-- rusticate:template-only:end -->";

/// Written where the owner is unknown, so it is easy to find and replace.
pub(crate) const OWNER_PLACEHOLDER: &str = "OWNER";

#[derive(Debug)]
pub(crate) struct Identity {
    name: String,
    title: String,
    repository: String,
    owner: String,
}

/// Something in a file the rules cannot handle, with its 1-based line.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Problem {
    line: usize,
    message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl Identity {
    pub(crate) fn new(name: &ProjectName, owner: Option<&Owner>) -> Self {
        let owner = owner.map_or(OWNER_PLACEHOLDER, Owner::as_str);
        Self {
            name: name.as_str().to_owned(),
            title: name.title(),
            repository: format!("github.com/{owner}/{}", name.as_str()),
            owner: format!("@{owner}"),
        }
    }

    /// The file's content for the new project.
    pub(crate) fn render(&self, text: &str) -> Result<String, Problem> {
        let text = strip_template_only(text)?;
        check_covered(&text)?;
        Ok(self.rewrite(&text))
    }

    /// One left-to-right pass: each match is replaced (or kept, if protected)
    /// and never rescanned, so a project name that itself contains `rusticate`
    /// is not rewritten twice.
    fn rewrite(&self, text: &str) -> String {
        let rules = [
            (PROTECTED[0], PROTECTED[0]),
            (PROTECTED[1], PROTECTED[1]),
            (PROTECTED[2], PROTECTED[2]),
            (REPOSITORY, self.repository.as_str()),
            (OWNER, self.owner.as_str()),
            (TITLE, self.title.as_str()),
            (NAME, self.name.as_str()),
        ];
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some((at, token, replacement)) = earliest_match(rest, &rules) {
            let (before, after) = rest.split_at_checked(at).unwrap_or((rest, ""));
            out.push_str(before);
            out.push_str(replacement);
            rest = after.get(token.len()..).unwrap_or_default();
        }
        out.push_str(rest);
        out
    }

    /// Renders every text file under `root` in place and returns how many
    /// changed. Nothing is written unless every file renders.
    pub(crate) fn apply_to_tree(&self, root: &Path) -> Result<usize> {
        let mut problems = Vec::new();
        let mut changes = Vec::new();
        for entry in tree::entries(root)? {
            let shown = entry.path.display();
            if contains_identity(&entry.path.to_string_lossy()) {
                problems.push(format!("{shown}: the path names the template"));
            }
            if entry.is_symlink {
                continue;
            }
            let path = root.join(&entry.path);
            let bytes = fs::read(&path).with_context(|| format!("reading {shown}"))?;
            let Ok(text) = String::from_utf8(bytes) else {
                continue; // binary: nothing to rename
            };
            match self.render(&text) {
                Ok(rendered) if rendered != text => changes.push((path, rendered)),
                Ok(_) => {}
                Err(problem) => problems.push(format!("{shown}: {problem}")),
            }
        }
        ensure!(
            problems.is_empty(),
            "the template has identity the generator does not cover (fix the template, \
             see cargo-rusticate/src/identity.rs):\n  {}",
            problems.join("\n  ")
        );
        for (path, text) in &changes {
            fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
        }
        Ok(changes.len())
    }
}

/// The first rule that matches; at the same position, the longest token wins.
fn earliest_match<'a>(
    text: &str,
    rules: &[(&'a str, &'a str)],
) -> Option<(usize, &'a str, &'a str)> {
    rules
        .iter()
        .filter_map(|&(token, replacement)| text.find(token).map(|at| (at, token, replacement)))
        .min_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.len().cmp(&a.1.len())))
}

fn contains_identity(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains(NAME) || lower.contains(OWNER_LOGIN)
}

/// Fails on any spelling of the identity the rules do not rewrite or keep.
fn check_covered(text: &str) -> Result<(), Problem> {
    // Blank out what the rules handle; tokens hold no newline, so line
    // numbers survive.
    let residue = PROTECTED
        .iter()
        .chain(&[REPOSITORY, OWNER, TITLE, NAME])
        .fold(text.to_owned(), |acc, token| acc.replace(token, " "));
    let lower = residue.to_ascii_lowercase();
    let first = [NAME, OWNER_LOGIN]
        .iter()
        .filter_map(|needle| lower.find(needle).map(|at| (at, needle.len())))
        .min();
    match first {
        None => Ok(()),
        Some((at, len)) => Err(Problem {
            line: line_of(&residue, at),
            message: format!(
                "`{}` is not a spelling the generator rewrites",
                residue.get(at..at + len).unwrap_or_default()
            ),
        }),
    }
}

/// Removes template-only blocks. Markers must sit on their own lines and pair up.
fn strip_template_only(text: &str) -> Result<Cow<'_, str>, Problem> {
    if !text.contains(TEMPLATE_ONLY_START) && !text.contains(TEMPLATE_ONLY_END) {
        return Ok(Cow::Borrowed(text));
    }
    let mut kept = String::with_capacity(text.len());
    let mut open_at = None;
    for (index, line) in text.split_inclusive('\n').enumerate() {
        let number = index + 1;
        let marker = line.trim();
        let problem = |message: &str| Problem {
            line: number,
            message: message.to_owned(),
        };
        match (marker, open_at) {
            (TEMPLATE_ONLY_START, None) => open_at = Some(number),
            (TEMPLATE_ONLY_START, Some(_)) => return Err(problem("nested template-only start")),
            (TEMPLATE_ONLY_END, Some(_)) => open_at = None,
            (TEMPLATE_ONLY_END, None) => return Err(problem("template-only end without a start")),
            _ if line.contains(TEMPLATE_ONLY_START) || line.contains(TEMPLATE_ONLY_END) => {
                return Err(problem("a template-only marker must be on its own line"));
            }
            (_, None) => kept.push_str(line),
            (_, Some(_)) => {}
        }
    }
    match open_at {
        Some(line) => Err(Problem {
            line,
            message: "template-only start is never closed".to_owned(),
        }),
        None => Ok(Cow::Owned(kept)),
    }
}

fn line_of(text: &str, at: usize) -> usize {
    text.get(..at)
        .map_or(0, |before| before.matches('\n').count())
        + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(owner: Option<&str>) -> Identity {
        let name = ProjectName::parse("acme-ledger").unwrap();
        let owner = owner.map(|o| Owner::parse(o).unwrap());
        Identity::new(&name, owner.as_ref())
    }

    #[test]
    fn rewrites_every_form_of_the_identity() {
        let text = "# rusticate\nRusticate Product API\nimage: rusticate-api\n\
                    https://github.com/ahadiihsan/rusticate/security\n* @ahadiihsan\n";
        assert_eq!(
            identity(Some("acme")).render(text).unwrap(),
            "# acme-ledger\nAcme Ledger Product API\nimage: acme-ledger-api\n\
             https://github.com/acme/acme-ledger/security\n* @acme\n"
        );
    }

    #[test]
    fn keeps_history_links_and_the_generator_name() {
        let text = "[review](https://github.com/ahadiihsan/rusticate/blob/af3c/a.md) \
                    `cargo install cargo-rusticate` then `cargo rusticate new`";
        assert_eq!(identity(Some("acme")).render(text).unwrap(), text);
    }

    #[test]
    fn a_name_containing_the_template_name_is_rewritten_once() {
        let name = ProjectName::parse("my-rusticate").unwrap();
        let owner = Owner::parse("acme").unwrap();
        let text = "rusticate Rusticate github.com/ahadiihsan/rusticate";
        assert_eq!(
            Identity::new(&name, Some(&owner)).render(text).unwrap(),
            "my-rusticate My Rusticate github.com/acme/my-rusticate"
        );
    }

    #[test]
    fn unknown_owner_becomes_a_findable_placeholder() {
        assert_eq!(identity(None).render("* @ahadiihsan").unwrap(), "* @OWNER");
    }

    #[test]
    fn leaves_unrelated_text_untouched() {
        let text = "rustic rust crate";
        assert_eq!(identity(None).render(text).unwrap(), text);
    }

    #[test]
    fn uncovered_spellings_are_problems_with_their_line() {
        let err = identity(None).render("ok\nRUSTICATE = 1\n").unwrap_err();
        assert_eq!(err.line, 2);
        assert!(err.message.contains("`RUSTICATE`"), "{err}");
        let err = identity(None)
            .render("see github.com/ahadiihsan/other")
            .unwrap_err();
        assert!(err.message.contains("`ahadiihsan`"), "{err}");
    }

    #[test]
    fn removes_template_only_blocks() {
        let text = "intro\n\n<!-- rusticate:template-only:start -->\n## Install\n\
                    cargo install cargo-rusticate\n\n<!-- rusticate:template-only:end -->\n## Next\n";
        assert_eq!(identity(None).render(text).unwrap(), "intro\n\n## Next\n");
    }

    #[test]
    fn rejects_malformed_template_only_markers() {
        let start = TEMPLATE_ONLY_START;
        let end = TEMPLATE_ONLY_END;
        for (text, line) in [
            (format!("a\n{start}\nb\n"), 2),
            (format!("{end}\n"), 1),
            (format!("{start}\n{start}\n{end}\n"), 2),
            (format!("text {start}\n{end}\n"), 1),
        ] {
            assert_eq!(
                identity(None).render(&text).unwrap_err().line,
                line,
                "{text}"
            );
        }
    }
}
