//! Optional parts of the template (ADR 0002): which exist, which the user
//! wants, and what that means for each file.
//!
//! The template declares its modules in `template.toml`, in dependency order.
//! A module owns whole paths (dropped with it) and marked blocks in shared
//! files (see `blocks`).

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::{BufRead, IsTerminal as _};
use std::path::{Component, Path};

use anyhow::{Context as _, Result, bail, ensure};
use serde::Deserialize;

use crate::blocks::Block;

/// The manifest, at the template's root. Never copied into projects.
pub(crate) const MANIFEST: &str = "template.toml";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    /// Paths that exist only in the template repository.
    #[serde(default)]
    template_only: Vec<String>,
    #[serde(default, rename = "module")]
    modules: Vec<Module>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Module {
    pub(crate) name: String,
    pub(crate) summary: String,
    #[serde(default)]
    pub(crate) requires: Vec<String>,
    #[serde(default)]
    pub(crate) paths: Vec<String>,
    /// Stand-ins used only without this module: removed when it is included.
    #[serde(default)]
    pub(crate) absent_paths: Vec<String>,
}

/// The template's modules, validated, and its template-only paths.
#[derive(Debug, Default)]
pub(crate) struct Modules {
    list: Vec<Module>,
    template_only: Vec<String>,
}

impl Modules {
    /// Reads `template.toml` from the template root. A template without one
    /// (releases before modules) has no modules.
    pub(crate) fn load(template_root: &Path) -> Result<Self> {
        let path = template_root.join(MANIFEST);
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path).with_context(|| format!("reading {MANIFEST}"))?;
        let file: ManifestFile =
            toml::from_str(&text).with_context(|| format!("parsing {MANIFEST}"))?;
        let mut modules = Self::new(file.modules)?;
        for path in &file.template_only {
            ensure!(
                is_plain(path),
                "{MANIFEST}: template-only path `{path}` must be a plain relative path"
            );
        }
        modules.template_only = file.template_only;
        Ok(modules)
    }

    fn new(modules: Vec<Module>) -> Result<Self> {
        let mut seen = BTreeSet::new();
        for module in &modules {
            let name = module.name.as_str();
            ensure!(
                !name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b == b'-'),
                "{MANIFEST}: invalid module name `{name}`"
            );
            for required in &module.requires {
                ensure!(
                    seen.contains(required.as_str()),
                    "{MANIFEST}: `{name}` requires `{required}`, which must be declared before it"
                );
            }
            for path in module.paths.iter().chain(&module.absent_paths) {
                ensure!(
                    is_plain(path),
                    "{MANIFEST}: `{name}` path `{path}` must be a plain relative path"
                );
            }
            ensure!(
                seen.insert(name),
                "{MANIFEST}: module `{name}` is declared twice"
            );
        }
        Ok(Self {
            list: modules,
            template_only: Vec::new(),
        })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    fn get(&self, name: &str) -> Option<&Module> {
        self.list.iter().find(|module| module.name == name)
    }

    fn check_known(&self, names: &[String]) -> Result<()> {
        for name in names {
            if self.get(name).is_none() {
                let known: Vec<&str> = self.list.iter().map(|m| m.name.as_str()).collect();
                if known.is_empty() {
                    bail!("this template version has no optional modules (`{name}` was requested)");
                }
                bail!(
                    "unknown module `{name}` (this template has: {})",
                    known.join(", ")
                );
            }
        }
        Ok(())
    }

    /// Modules that require `name`, directly or not.
    fn dependents(&self, name: &str) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        for module in &self.list {
            let needs = |r: &String| r == name || found.contains(r);
            if module.requires.iter().any(needs) {
                found.push(module.name.clone());
            }
        }
        found
    }
}

/// How the user picks modules.
#[derive(Debug, Default, Clone)]
pub(crate) struct Request {
    pub(crate) with: Vec<String>,
    pub(crate) without: Vec<String>,
    pub(crate) minimal: bool,
}

impl Request {
    fn is_empty(&self) -> bool {
        self.with.is_empty() && self.without.is_empty() && !self.minimal
    }
}

/// The modules a project includes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Selection(BTreeSet<String>);

impl Selection {
    pub(crate) fn names(&self) -> Vec<&str> {
        self.0.iter().map(String::as_str).collect()
    }

    pub(crate) fn includes(&self, name: &str) -> bool {
        self.0.contains(name)
    }

    /// Whether a block's lines belong in the project.
    pub(crate) fn keeps(&self, block: &Block) -> bool {
        match block {
            Block::TemplateOnly => false,
            Block::Module(name) => self.includes(name),
        }
    }

    pub(crate) fn describe(&self, modules: &Modules) -> String {
        if modules.is_empty() {
            return String::new();
        }
        let chosen: Vec<&str> = modules
            .list
            .iter()
            .filter(|m| self.includes(&m.name))
            .map(|m| m.name.as_str())
            .collect();
        if chosen.is_empty() {
            "core only".to_owned()
        } else {
            format!("core + {}", chosen.join(" + "))
        }
    }
}

/// Decides the selection: from flags when given, else interactively in a
/// terminal, else everything (what earlier releases generated).
pub(crate) fn choose(modules: &Modules, request: &Request, interactive: bool) -> Result<Selection> {
    if request.is_empty() && interactive && !modules.is_empty() {
        let stdin = std::io::stdin();
        return ask(modules, &mut stdin.lock(), &mut std::io::stderr());
    }
    resolve(modules, request)
}

/// Whether questions can be asked: both ends are a terminal.
pub(crate) fn can_ask() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

fn resolve(modules: &Modules, request: &Request) -> Result<Selection> {
    modules.check_known(&request.with)?;
    modules.check_known(&request.without)?;
    let mut chosen: BTreeSet<String> = if request.minimal {
        BTreeSet::new()
    } else {
        modules.list.iter().map(|m| m.name.clone()).collect()
    };
    chosen.extend(request.with.iter().cloned());
    for name in &request.without {
        ensure!(
            !request.with.contains(name),
            "`{name}` is both requested and excluded"
        );
        chosen.remove(name);
        for dependent in modules.dependents(name) {
            ensure!(
                !request.with.contains(&dependent),
                "`{dependent}` needs `{name}`: drop `--without {name}`, or drop `--with {dependent}`"
            );
            chosen.remove(&dependent);
        }
    }
    for name in &chosen {
        if let Some(module) = modules.get(name) {
            for required in &module.requires {
                ensure!(
                    chosen.contains(required),
                    "`{name}` needs `{required}`: add `--with {required}`"
                );
            }
        }
    }
    Ok(Selection(chosen))
}

/// One yes/no question per module, in dependency order. A module whose
/// requirements were declined is skipped.
fn ask(
    modules: &Modules,
    input: &mut dyn BufRead,
    out: &mut dyn std::io::Write,
) -> Result<Selection> {
    writeln!(
        out,
        "Which optional parts should the project include? (core: HTTP API, auth, telemetry, tooling)"
    )?;
    let mut chosen = BTreeSet::new();
    for module in &modules.list {
        let missing: Vec<&str> = module
            .requires
            .iter()
            .filter(|r| !chosen.contains(*r))
            .map(String::as_str)
            .collect();
        if !missing.is_empty() {
            writeln!(
                out,
                "  - {}: skipped (needs {})",
                module.name,
                missing.join(", ")
            )?;
            continue;
        }
        if confirm(
            input,
            out,
            &format!("  {}: {}?", module.name, module.summary),
        )? {
            chosen.insert(module.name.clone());
        }
    }
    Ok(Selection(chosen))
}

/// `[Y/n]`, defaulting to yes; asks again on anything else.
fn confirm(input: &mut dyn BufRead, out: &mut dyn std::io::Write, question: &str) -> Result<bool> {
    loop {
        write!(out, "{question} [Y/n] ")?;
        out.flush()?;
        let mut answer = String::new();
        let read = input.read_line(&mut answer).context("reading the answer")?;
        ensure!(
            read > 0,
            "no answer (input closed): pass --with/--without instead"
        );
        match answer.trim().to_ascii_lowercase().as_str() {
            "" | "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => writeln!(out, "  please answer y or n")?,
        }
    }
}

/// Paths the selection drops: template-only paths, the paths of every
/// module not chosen, and the stand-ins (`absent_paths`) of every module
/// chosen.
pub(crate) fn dropped_paths<'a>(modules: &'a Modules, selection: &Selection) -> Vec<&'a str> {
    let by_module = modules.list.iter().flat_map(|m| {
        if selection.includes(&m.name) {
            &m.absent_paths
        } else {
            &m.paths
        }
    });
    modules
        .template_only
        .iter()
        .chain(by_module)
        .map(String::as_str)
        .collect()
}

fn is_plain(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

/// Every module name a block refers to must exist, so a typo in a marker
/// cannot silently drop (or keep) code.
pub(crate) fn check_block(modules: &Modules, block: &Block) -> Option<String> {
    match block {
        Block::Module(name) if modules.get(name).is_none() => {
            let mut message = format!("unknown module `{name}` in a marker");
            if !modules.is_empty() {
                let known: Vec<&str> = modules.list.iter().map(|m| m.name.as_str()).collect();
                let _ = write!(message, " (known: {})", known.join(", "));
            }
            Some(message)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modules() -> Modules {
        let module = |name: &str, requires: &[&str]| Module {
            name: name.to_owned(),
            summary: format!("the {name} module"),
            requires: requires.iter().map(|r| (*r).to_owned()).collect(),
            paths: vec![format!("crates/{name}")],
            absent_paths: vec![format!("stand-in/{name}")],
        };
        Modules::new(vec![
            module("postgres", &[]),
            module("messaging", &["postgres"]),
            module("example", &["postgres", "messaging"]),
        ])
        .unwrap()
    }

    fn request(with: &[&str], without: &[&str], minimal: bool) -> Request {
        Request {
            with: with.iter().map(|s| (*s).to_owned()).collect(),
            without: without.iter().map(|s| (*s).to_owned()).collect(),
            minimal,
        }
    }

    fn names(selection: &Selection) -> Vec<&str> {
        selection.0.iter().map(String::as_str).collect()
    }

    #[test]
    fn without_flags_everything_is_included() {
        let all = resolve(&modules(), &Request::default()).unwrap();
        assert_eq!(names(&all), ["example", "messaging", "postgres"]);
    }

    #[test]
    fn dropping_a_module_drops_what_needs_it() {
        let selection = resolve(&modules(), &request(&[], &["messaging"], false)).unwrap();
        assert_eq!(names(&selection), ["postgres"]);
        let bare = resolve(&modules(), &request(&[], &["example"], false)).unwrap();
        assert_eq!(names(&bare), ["messaging", "postgres"]);
    }

    #[test]
    fn minimal_starts_from_core_and_adds_with_requirements_checked() {
        let core = resolve(&modules(), &request(&[], &[], true)).unwrap();
        assert_eq!(names(&core), [] as [&str; 0]);
        let pg = resolve(&modules(), &request(&["postgres"], &[], true)).unwrap();
        assert_eq!(names(&pg), ["postgres"]);
        let err = resolve(&modules(), &request(&["messaging"], &[], true)).unwrap_err();
        assert!(
            err.to_string().contains("`messaging` needs `postgres`"),
            "{err}"
        );
    }

    #[test]
    fn contradictions_and_unknown_names_are_errors() {
        let err = resolve(&modules(), &request(&["example"], &["postgres"], false)).unwrap_err();
        assert!(
            err.to_string().contains("`example` needs `postgres`"),
            "{err}"
        );
        let err = resolve(&modules(), &request(&["redis"], &[], false)).unwrap_err();
        assert!(err.to_string().contains("unknown module `redis`"), "{err}");
        let err = resolve(&Modules::default(), &request(&[], &["example"], false)).unwrap_err();
        assert!(err.to_string().contains("no optional modules"), "{err}");
    }

    #[test]
    fn questions_skip_modules_whose_requirements_were_declined() {
        let mut out = Vec::new();
        let selection = ask(&modules(), &mut "y\nn\n".as_bytes(), &mut out).unwrap();
        assert_eq!(names(&selection), ["postgres"]);
        let shown = String::from_utf8(out).unwrap();
        assert!(
            shown.contains("example: skipped (needs messaging)"),
            "{shown}"
        );
    }

    #[test]
    fn questions_default_to_yes_and_repeat_on_nonsense() {
        let mut out = Vec::new();
        let selection = ask(&modules(), &mut "\nmaybe\nyes\n\n".as_bytes(), &mut out).unwrap();
        assert_eq!(names(&selection), ["example", "messaging", "postgres"]);
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("please answer y or n")
        );
        let err = ask(&modules(), &mut "".as_bytes(), &mut Vec::new()).unwrap_err();
        assert!(err.to_string().contains("input closed"), "{err}");
    }

    #[test]
    fn manifests_are_validated() {
        let bad = |toml: &str| {
            let file: ManifestFile = toml::from_str(toml).unwrap();
            Modules::new(file.modules).unwrap_err().to_string()
        };
        assert!(
            bad("[[module]]\nname = \"b\"\nsummary = \"\"\nrequires = [\"a\"]\n")
                .contains("declared before")
        );
        assert!(
            bad("[[module]]\nname = \"a\"\nsummary = \"\"\npaths = [\"../x\"]\n")
                .contains("plain relative")
        );
        assert!(bad("[[module]]\nname = \"A\"\nsummary = \"\"\n").contains("invalid module name"));
    }

    #[test]
    fn selection_keeps_blocks_and_lists_dropped_paths() {
        let selection = resolve(&modules(), &request(&[], &["messaging"], false)).unwrap();
        assert!(selection.keeps(&Block::Module("postgres".into())));
        assert!(!selection.keeps(&Block::Module("example".into())));
        assert!(!selection.keeps(&Block::TemplateOnly));
        assert_eq!(
            dropped_paths(&modules(), &selection),
            ["stand-in/postgres", "crates/messaging", "crates/example"]
        );
        assert_eq!(selection.describe(&modules()), "core + postgres");
        assert!(check_block(&modules(), &Block::Module("redis".into())).is_some());
    }
}
