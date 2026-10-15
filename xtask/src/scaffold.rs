//! `cargo rusticate new ...`: generators that produce code already shaped to pass
//! every architecture rule. Placeholders carry `SCAFFOLD:` markers, which
//! `cargo rusticate arch` rejects until they are replaced (SCAF-001).
//!
//! Existing files are edited only at explicit `xtask:<name>` marker comments.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail, ensure};
use clap::ValueEnum;
use regex::Regex;

use crate::sh;
use crate::workspace::{Workspace, owned_schema, root};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FeatureKind {
    /// State-changing HTTP endpoint (POST, idempotent).
    Command,
    /// Read-only HTTP endpoint (GET).
    Query,
    /// Event consumer (worker).
    Event,
}

/// Every placeholder a template may use.
struct Names {
    pairs: Vec<(&'static str, String)>,
}

impl Names {
    fn domain(domain: &str) -> Self {
        Self {
            pairs: vec![
                ("domain_ident", domain.replace('-', "_")),
                ("domain_kebab", domain.replace('_', "-")),
                ("Domain", pascal(domain)),
                ("DOMAIN", domain.to_ascii_uppercase()),
                ("schema", domain.to_owned()),
                ("domain", domain.to_owned()),
            ],
        }
    }

    fn feature(domain: &str, feature: &str) -> Self {
        let (verb, noun) = feature.split_once('_').unwrap_or((feature, feature));
        let mut names = Self::domain(domain);
        names.pairs.extend([
            ("feature_kebab", feature.replace('_', "-")),
            ("featureCamel", camel(feature)),
            ("Feature", pascal(feature)),
            ("PERMISSION", format!("{noun}_{verb}").to_ascii_uppercase()),
            ("permission_name", format!("{domain}.{noun}.{verb}")),
            ("feature", feature.to_owned()),
        ]);
        names
    }

    fn render(&self, template: &str) -> String {
        self.pairs
            .iter()
            .fold(template.to_owned(), |text, (key, value)| {
                text.replace(&format!("{{{{{key}}}}}"), value)
            })
    }

    fn get(&self, key: &str) -> &str {
        self.pairs
            .iter()
            .find(|(k, _)| *k == key)
            .map_or("", |(_, v)| v.as_str())
    }
}

fn pascal(snake: &str) -> String {
    snake
        .split(['_', '-'])
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_ascii_uppercase().to_string() + chars.as_str()
            })
        })
        .collect()
}

fn camel(snake: &str) -> String {
    let pascal = pascal(snake);
    let mut chars = pascal.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_ascii_lowercase().to_string() + chars.as_str()
    })
}

fn write_new(path: &Path, contents: &str) -> Result<()> {
    ensure!(!path.exists(), "{} already exists", path.display());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;
    println!(
        "  created {}",
        path.strip_prefix(root()).unwrap_or(path).display()
    );
    Ok(())
}

/// Inserts `line` (indented like the marker) after the line containing
/// `marker`. Idempotent: does nothing if `line` is already present.
fn insert_after_marker(path: &Path, marker: &str, line: &str) -> Result<()> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    if text.lines().any(|l| l.trim() == line.trim()) {
        return Ok(());
    }
    let mut output = String::with_capacity(text.len() + line.len() + 8);
    let mut inserted = false;
    for current in text.lines() {
        output.push_str(current);
        output.push('\n');
        if !inserted && current.contains(marker) {
            let indent: String = current.chars().take_while(|c| c.is_whitespace()).collect();
            output.push_str(&indent);
            output.push_str(line.trim());
            output.push('\n');
            inserted = true;
        }
    }
    ensure!(
        inserted,
        "marker `{marker}` not found in {}; add it where generated lines belong",
        path.display()
    );
    std::fs::write(path, output)?;
    println!(
        "  edited  {}",
        path.strip_prefix(root()).unwrap_or(path).display()
    );
    Ok(())
}

fn timestamp() -> String {
    chrono::Utc::now().format("%Y%m%d%H%M%S").to_string()
}

/// The template modules this project has (ADR 0002), from the crates present.
#[derive(Debug, Clone, Copy)]
struct Stack {
    postgres: bool,
    messaging: bool,
}

impl Stack {
    fn detect(workspace: &Workspace) -> Self {
        Self {
            postgres: workspace.member("datastore").is_some(),
            messaging: workspace.member("messaging").is_some(),
        }
    }

    /// Scaffold templates without a database live under `core/`.
    const fn templates(self) -> &'static str {
        if self.postgres { "" } else { "core/" }
    }
}

/// A scaffold template (under `xtask/templates`), read at run time so a
/// project carries only the templates of its modules. Module marker lines,
/// present only in the template repository (where every module is), are
/// dropped.
fn template(relative: &str) -> Result<String> {
    let path = root().join("xtask/templates").join(relative);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading the scaffold template {}", path.display()))?;
    let marker = Regex::new(r"^\s*(//|#|--)\s*[a-z-]+:module\([a-z-]+\):(start|end)\s*$")?;
    let mut kept = String::with_capacity(text.len());
    for line in text.lines().filter(|line| !marker.is_match(line)) {
        kept.push_str(line);
        kept.push('\n');
    }
    Ok(kept)
}

/// Files of a new context: (`crate` or `engine`, path, template).
fn domain_files(stack: Stack) -> Vec<(&'static str, &'static str, String)> {
    let dir = stack.templates();
    let mut files = vec![
        (
            "crate",
            "Cargo.toml",
            format!("{dir}domain/Cargo.toml.tmpl"),
        ),
        ("crate", "src/lib.rs", format!("{dir}domain/lib.rs.tmpl")),
        (
            "crate",
            "src/error.rs",
            format!("{dir}domain/error.rs.tmpl"),
        ),
        (
            "crate",
            "src/permissions.rs",
            "domain/permissions.rs.tmpl".to_owned(),
        ),
        (
            "crate",
            "tests/integration/main.rs",
            format!("{dir}domain/tests_main.rs.tmpl"),
        ),
        ("engine", "Cargo.toml", "engine/Cargo.toml.tmpl".to_owned()),
        ("engine", "src/lib.rs", "engine/lib.rs.tmpl".to_owned()),
        ("engine", "src/error.rs", "engine/error.rs.tmpl".to_owned()),
    ];
    if stack.postgres {
        files.extend([
            ("crate", "sqlx.toml", "domain/sqlx.toml.tmpl".to_owned()),
            (
                "crate",
                "tests/integration/support.rs",
                "domain/tests_support.rs.tmpl".to_owned(),
            ),
            (
                "crate",
                "tests/integration/schema.rs",
                "domain/tests_schema.rs.tmpl".to_owned(),
            ),
        ]);
    }
    if stack.messaging {
        files.push(("crate", "src/events.rs", "domain/events.rs.tmpl".to_owned()));
    }
    files
}

/// Registers a new context with the workspace and mounts it in the API.
fn wire_domain(name: &str, ident: &str, stack: Stack) -> Result<()> {
    let root = root();
    let edits = [
        (
            "Cargo.toml",
            "# xtask:domains",
            format!("{name} = {{ path = \"crates/domains/{name}\" }}"),
        ),
        (
            "Cargo.toml",
            "# xtask:domains",
            format!("{name}-engine = {{ path = \"crates/domains/{name}-engine\" }}"),
        ),
        (
            "apps/api/Cargo.toml",
            "# xtask:contexts",
            format!("{name}.workspace = true"),
        ),
        (
            "apps/api/src/app.rs",
            "// xtask:contexts",
            format!(".merge({ident}::router())"),
        ),
    ];
    for (file, marker, line) in edits {
        insert_after_marker(&root.join(file), marker, &line)?;
    }
    if stack.postgres {
        insert_after_marker(
            &root.join("apps/api/src/migrate.rs"),
            "// xtask:migrators",
            &format!("(\"{name}\", &{ident}::MIGRATOR),"),
        )?;
    }
    Ok(())
}

/// `cargo rusticate new domain <name>`.
pub fn domain(name: &str) -> Result<()> {
    ensure!(
        Regex::new(r"^[a-z][a-z0-9]*(_[a-z0-9]+)*$")?.is_match(name),
        "context names are lowercase snake_case (they become the schema name)"
    );
    ensure!(
        !["platform", "public", "pg_catalog"].contains(&name),
        "`{name}` is reserved"
    );
    let root = root();
    let crate_dir = root.join("crates/domains").join(name);
    let engine_dir = root.join("crates/domains").join(format!("{name}-engine"));
    ensure!(
        !crate_dir.exists() && !engine_dir.exists(),
        "context `{name}` already exists"
    );
    let names = Names::domain(name);
    let stack = Stack::detect(&Workspace::load()?);

    println!("Scaffolding bounded context `{name}`:");
    for (dir, file, relative) in domain_files(stack) {
        let base = if dir == "engine" {
            &engine_dir
        } else {
            &crate_dir
        };
        write_new(&base.join(file), &names.render(&template(&relative)?))?;
    }
    if stack.postgres {
        write_new(
            &crate_dir.join(format!("migrations/{}_{name}_init.sql", timestamp())),
            &names.render(&template("domain/init.sql.tmpl")?),
        )?;
    }
    write_new(&engine_dir.join("clippy.toml"), &engine_clippy_config()?)?;
    wire_domain(name, names.get("domain_ident"), stack)?;
    sh::run(sh::cargo(["fmt", "--all"]))?;

    let migrate = if stack.postgres {
        format!("  cargo rusticate db migrate                      # creates the `{name}` schema\n")
    } else {
        String::new()
    };
    println!(
        "\nNext:\n{migrate}  cargo rusticate new feature {name} <verb_noun>    # add use cases\n  \
         then replace every SCAFFOLD marker, and: cargo rusticate verify"
    );
    Ok(())
}

/// Root clippy.toml plus the engine determinism and purity bans (ENG-002).
fn engine_clippy_config() -> Result<String> {
    let mut config: toml::Table =
        toml::from_str(&std::fs::read_to_string(root().join("clippy.toml"))?)?;
    let ban = |path: &str, reason: &str| {
        let mut entry = toml::Table::new();
        entry.insert("path".into(), path.into());
        entry.insert("reason".into(), reason.into());
        toml::Value::Table(entry)
    };
    let extend = |config: &mut toml::Table, key: &str, extra: Vec<toml::Value>| {
        if let Some(toml::Value::Array(items)) = config.get_mut(key) {
            items.extend(extra);
        }
    };
    extend(
        &mut config,
        "disallowed-methods",
        vec![
            ban(
                "std::time::SystemTime::now",
                "ENG-002: pass the instant in as an argument",
            ),
            ban(
                "std::time::Instant::now",
                "ENG-002: engines are not timed internally",
            ),
            ban(
                "chrono::Utc::now",
                "ENG-002: pass the instant in as an argument",
            ),
            ban(
                "chrono::Local::now",
                "ENG-002: pass the instant in as an argument",
            ),
            ban(
                "uuid::Uuid::now_v7",
                "ENG-002: identifiers are generated by the orchestrator",
            ),
        ],
    );
    extend(
        &mut config,
        "disallowed-types",
        vec![
            ban("std::fs::File", "ENG-001: engines perform no I/O"),
            ban("std::net::TcpStream", "ENG-001: engines perform no I/O"),
            ban("std::net::UdpSocket", "ENG-001: engines perform no I/O"),
            ban("std::process::Command", "ENG-001: engines perform no I/O"),
            ban(
                "std::collections::HashMap",
                "ENG-002: iteration order is random; use BTreeMap",
            ),
            ban(
                "std::collections::HashSet",
                "ENG-002: iteration order is random; use BTreeSet",
            ),
        ],
    );
    Ok(format!(
        "# Engine clippy configuration (ENG-002), generated by `cargo rusticate new domain`.\n\
         # Clippy reads only the nearest clippy.toml: this repeats the root settings\n\
         # and adds the determinism bans. `cargo rusticate arch` keeps it in sync.\n\n{}",
        toml::to_string_pretty(&config)?
    ))
}

/// `cargo rusticate new feature <domain> <verb_noun> --kind ...`.
pub fn feature(domain: &str, feature: &str, kind: FeatureKind) -> Result<()> {
    ensure!(
        Regex::new(r"^[a-z]+(_[a-z0-9]+)+$")?.is_match(feature),
        "features are named `<verb>_<noun>` in the ubiquitous language, e.g. `issue_invoice`"
    );
    let workspace = Workspace::load()?;
    let member = workspace
        .member(domain)
        .filter(|m| m.tier == "domain")
        .with_context(|| format!("no bounded context named `{domain}`"))?;
    let src = member.abs_dir.join("src");
    ensure!(
        !src.join(format!("{feature}.rs")).exists(),
        "feature `{feature}` already exists"
    );
    let stack = Stack::detect(&workspace);
    ensure!(
        kind != FeatureKind::Event || stack.messaging,
        "event consumers need the messaging module, which this project does not have"
    );
    let names = Names::feature(domain, feature);
    let render = |relative: &str| -> Result<String> {
        Ok(names.render(&template(&format!(
            "{}feature/{relative}",
            stack.templates()
        ))?))
    };
    let tests = member.abs_dir.join("tests/integration");

    println!("Scaffolding {kind:?} feature `{domain}::{feature}`:");
    let (workflow, adapter, adapter_file, test) = match kind {
        FeatureKind::Command => (
            "command.rs.tmpl",
            "command_http.rs.tmpl",
            "http.rs",
            "command_test.rs.tmpl",
        ),
        FeatureKind::Query => (
            "query.rs.tmpl",
            "query_http.rs.tmpl",
            "http.rs",
            "query_test.rs.tmpl",
        ),
        FeatureKind::Event => (
            "event.rs.tmpl",
            "event_handler.rs.tmpl",
            "handler.rs",
            "event_test.rs.tmpl",
        ),
    };
    write_new(&src.join(format!("{feature}.rs")), &render(workflow)?)?;
    write_new(&src.join(feature).join(adapter_file), &render(adapter)?)?;
    if stack.postgres {
        write_new(&src.join(feature).join("db.rs"), &render("db.rs.tmpl")?)?;
    }
    write_new(&tests.join(format!("{feature}.rs")), &render(test)?)?;

    wire_feature(&names, domain, feature, kind, &src, &tests)?;
    sh::run(sh::cargo(["fmt", "--all"]))?;

    let prepare = if stack.postgres {
        "  cargo rusticate db prepare        # caches the new query metadata (needs `db up`)\n"
    } else {
        ""
    };
    println!(
        "\nNext:\n{prepare}  replace every SCAFFOLD marker (cargo rusticate arch lists them)\n  \
         then: cargo rusticate verify"
    );
    Ok(())
}

/// Registers a new feature: its permission, module, test module, and its
/// route (HTTP) or consumer (event).
fn wire_feature(
    names: &Names,
    domain: &str,
    feature: &str,
    kind: FeatureKind,
    src: &Path,
    tests: &Path,
) -> Result<()> {
    insert_after_marker(
        &src.join("permissions.rs"),
        "// xtask:permissions",
        &format!(
            "pub const {}: op_context::Permission = op_context::Permission::new(\"{}\");",
            names.get("PERMISSION"),
            names.get("permission_name")
        ),
    )?;
    insert_after_marker(
        &src.join("lib.rs"),
        "// xtask:features",
        &format!("pub mod {feature};"),
    )?;
    insert_after_marker(
        &tests.join("main.rs"),
        "// xtask:tests",
        &format!("mod {feature};"),
    )?;
    match kind {
        FeatureKind::Command | FeatureKind::Query => insert_after_marker(
            &src.join("lib.rs"),
            "// xtask:routes",
            &format!(".routes(utoipa_axum::routes!({feature}::http::handle))"),
        )?,
        FeatureKind::Event => {
            let worker = root().join("apps/worker");
            insert_after_marker(
                &worker.join("Cargo.toml"),
                "# xtask:contexts",
                &format!("{domain}.workspace = true"),
            )?;
            insert_after_marker(
                &worker.join("src/tasks.rs"),
                "// xtask:consumers",
                &format!(
                    "spawn_consumer(&mut tasks, &deps, shutdown, {}::{feature}::{});",
                    names.get("domain_ident"),
                    names.get("Feature")
                ),
            )?;
        }
    }
    Ok(())
}

/// `cargo rusticate new migration <domain> <description>`. Returns the new path.
pub fn migration(domain: &str, description: &str) -> Result<PathBuf> {
    let workspace = Workspace::load()?;
    let Some(member) = workspace.member(domain).filter(|m| m.has_migrations()) else {
        bail!("`{domain}` owns no schema (no migrations/ directory)");
    };
    let schema =
        owned_schema(member)?.with_context(|| format!("{domain}/sqlx.toml declares no schema"))?;
    let slug: String = description
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    ensure!(
        !slug.is_empty(),
        "describe the migration, e.g. \"create invoices\""
    );
    let path = member
        .abs_dir
        .join(format!("migrations/{}_{slug}.sql", timestamp()));
    let names = Names {
        pairs: vec![("schema", schema), ("description", description.to_owned())],
    };
    write_new(&path, &names.render(&template("migration.sql.tmpl")?))?;
    println!(
        "\nNext: edit it, `cargo rusticate db migrate`, then `cargo rusticate db prepare` once queries use it."
    );
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_names_from_verb_noun() {
        let names = Names::feature("billing", "issue_credit_note");

        assert_eq!(names.get("Feature"), "IssueCreditNote");
        assert_eq!(names.get("featureCamel"), "issueCreditNote");
        assert_eq!(names.get("feature_kebab"), "issue-credit-note");
        assert_eq!(names.get("PERMISSION"), "CREDIT_NOTE_ISSUE");
        assert_eq!(names.get("permission_name"), "billing.credit_note.issue");
    }

    #[test]
    fn renders_placeholders_next_to_braces() {
        let names = Names::feature("billing", "issue_invoice");

        assert_eq!(
            names.render("use super::{{{Feature}}Cmd, {{feature}}};"),
            "use super::{IssueInvoiceCmd, issue_invoice};"
        );
    }
}
