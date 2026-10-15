//! Layout rules: lint wiring, engine guardrails, the feature-slice shape,
//! router mounting, test coverage of workflows, migrations and schema
//! ownership.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Result;
use regex::Regex;
use syn::Visibility;

use super::Violation;
use crate::workspace::{Member, Workspace, owned_schema, root};

pub fn check(workspace: &Workspace) -> Result<Vec<Violation>> {
    let mut violations = Vec::new();
    let known_schemas: BTreeSet<String> = workspace
        .members
        .iter()
        .filter_map(|m| owned_schema(m).ok().flatten())
        .collect();
    for member in workspace.members.iter().filter(|m| !m.exempt) {
        lints_wired(member, &mut violations)?;
        if member.tier == "engine" {
            engine_guardrails(member, &mut violations)?;
        }
        if member.tier == "domain" {
            feature_slices(member, &mut violations)?;
        }
        if member.has_migrations() {
            migrations(member, &known_schemas, &mut violations)?;
        }
    }
    migrators_registered(workspace, &mut violations)?;
    Ok(violations)
}

/// LINT-001: every product crate inherits the workspace lints.
fn lints_wired(member: &Member, violations: &mut Vec<Violation>) -> Result<()> {
    let manifest: toml::Value = toml::from_str(&std::fs::read_to_string(member.manifest())?)?;
    let inherits = manifest
        .get("lints")
        .and_then(|l| l.get("workspace"))
        .and_then(toml::Value::as_bool)
        == Some(true);
    if !inherits {
        violations.push(Violation::new(
            "LINT-001",
            format!("{}/Cargo.toml", member.dir),
            "add `[lints]\\nworkspace = true` (the compiler guardrails are not optional)",
        ));
    }
    Ok(())
}

/// ENG-002 / ENG-004: engines carry the stricter clippy config and lint attributes.
fn engine_guardrails(member: &Member, violations: &mut Vec<Violation>) -> Result<()> {
    let clippy_path = member.abs_dir.join("clippy.toml");
    let location = format!("{}/clippy.toml", member.dir);
    if clippy_path.exists() {
        let root_cfg: toml::Table =
            toml::from_str(&std::fs::read_to_string(root().join("clippy.toml"))?)?;
        let engine_cfg: toml::Table = toml::from_str(&std::fs::read_to_string(&clippy_path)?)?;
        for (key, value) in &root_cfg {
            let mirrored = match (value, engine_cfg.get(key)) {
                (toml::Value::Array(items), Some(toml::Value::Array(engine_items))) => {
                    items.iter().all(|item| engine_items.contains(item))
                }
                (value, Some(engine_value)) => value == engine_value,
                (_, None) => false,
            };
            if !mirrored {
                violations.push(Violation::new(
                    "ENG-002",
                    location.clone(),
                    format!(
                        "must contain every root clippy.toml entry; `{key}` is missing or different"
                    ),
                ));
            }
        }
    } else {
        violations.push(Violation::new("ENG-002", location, "engine crates need their own clippy.toml (copy the root one, add the determinism bans)"));
    }

    let lib = std::fs::read_to_string(member.abs_dir.join("src/lib.rs"))?;
    let file = syn::parse_file(&lib)?;
    let denies = file
        .attrs
        .iter()
        .filter(|a| a.path().is_ident("deny"))
        .filter_map(|a| a.meta.require_list().ok().map(|l| l.tokens.to_string()))
        .collect::<Vec<_>>()
        .join(",");
    for lint in ["arithmetic_side_effects", "as_conversions"] {
        if !denies.contains(lint) {
            violations.push(Violation::new(
                "ENG-004",
                format!("{}/src/lib.rs", member.dir),
                format!("engines must `#![deny(clippy::{lint})]`"),
            ));
        }
    }
    Ok(())
}

/// ARCH-004..ARCH-006, ARCH-009, TEST-002, TEST-005: the shape of every
/// feature slice in a context.
fn feature_slices(member: &Member, violations: &mut Vec<Violation>) -> Result<()> {
    let src = member.abs_dir.join("src");
    let context = Context {
        member,
        lib: std::fs::read_to_string(src.join("lib.rs"))?,
        tests: read_all(&member.abs_dir.join("tests"))?,
    };
    let mut has_features = false;

    for entry in std::fs::read_dir(&src)? {
        let path = entry?.path();
        let (Some(stem), Some("rs")) = (
            path.file_stem().and_then(|s| s.to_str()),
            path.extension().and_then(|e| e.to_str()),
        ) else {
            continue;
        };
        let feature_dir = src.join(stem);
        let is_feature = ["db.rs", "http.rs", "handler.rs"]
            .iter()
            .any(|file| feature_dir.join(file).exists());
        if is_feature {
            has_features = true;
            check_feature(&context, &path, stem, violations)?;
        }
    }

    if has_features && !member.abs_dir.join("tests/integration/main.rs").exists() {
        violations.push(Violation::new(
            "TEST-002",
            member.dir.clone(),
            "contexts keep their integration tests in one binary: tests/integration/main.rs",
        ));
    }
    Ok(())
}

/// What every feature check of one context needs.
struct Context<'a> {
    member: &'a Member,
    lib: String,
    /// Concatenated text of every file under `tests/`.
    tests: String,
}

fn check_feature(
    context: &Context<'_>,
    path: &Path,
    stem: &str,
    violations: &mut Vec<Violation>,
) -> Result<()> {
    let dir = &context.member.dir;
    let feature_dir = context.member.abs_dir.join("src").join(stem);
    let location = format!("{dir}/src/{stem}.rs");
    let file = syn::parse_file(&std::fs::read_to_string(path)?)?;

    private_sql_module(
        &file,
        &feature_dir,
        &location,
        &format!("{dir}/src/{stem}/db.rs"),
        violations,
    )?;

    // ARCH-006 / OBS-001 / SEC-003 / TEST-005: entry workflows.
    let workflows: Vec<&syn::ItemFn> = file
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Fn(f)
                if matches!(f.vis, Visibility::Public(_)) && f.sig.asyncness.is_some() =>
            {
                Some(f)
            }
            _ => None,
        })
        .collect();
    if workflows.is_empty() {
        violations.push(Violation::new(
            "ARCH-006",
            location.clone(),
            "feature has no `pub async fn` workflow entry point",
        ));
    }
    for workflow in workflows {
        check_workflow(context, workflow, &location, violations);
    }

    // ARCH-009: an HTTP adapter must be mounted.
    if feature_dir.join("http.rs").exists()
        && !context.lib.contains(&format!("{stem}::http::handle"))
    {
        violations.push(Violation::new(
            "ARCH-009",
            format!("{dir}/src/lib.rs"),
            format!("`{stem}::http::handle` is not registered in router()"),
        ));
    }
    Ok(())
}

/// ARCH-004 / ARCH-005: `mod db;` exists, is private, and exports nothing `pub`.
fn private_sql_module(
    file: &syn::File,
    feature_dir: &Path,
    location: &str,
    db_location: &str,
    violations: &mut Vec<Violation>,
) -> Result<()> {
    let db_module = file.items.iter().find_map(|item| match item {
        syn::Item::Mod(m) if m.ident == "db" => Some(m),
        _ => None,
    });
    match db_module {
        Some(m) if matches!(m.vis, Visibility::Inherited) => {}
        Some(_) => violations.push(Violation::new(
            "ARCH-005",
            location,
            "`mod db;` must be private",
        )),
        None => violations.push(Violation::new(
            "ARCH-004",
            location,
            "feature declares no `mod db;`",
        )),
    }
    let db_path = feature_dir.join("db.rs");
    if !db_path.exists() {
        violations.push(Violation::new(
            "ARCH-004",
            db_location,
            "feature has no db.rs: its SQL belongs in a private `db` module",
        ));
        return Ok(());
    }
    let db_file = syn::parse_file(&std::fs::read_to_string(db_path)?)?;
    for item in &db_file.items {
        if let Some((ident, Visibility::Public(_))) = item_visibility(item) {
            violations.push(Violation::new(
                "ARCH-005",
                db_location,
                format!("`{ident}` is `pub`: queries are `pub(super)` to their feature"),
            ));
        }
    }
    Ok(())
}

fn check_workflow(
    context: &Context<'_>,
    workflow: &syn::ItemFn,
    location: &str,
    violations: &mut Vec<Violation>,
) {
    let name = workflow.sig.ident.to_string();
    let instrumented = workflow.attrs.iter().any(|a| {
        a.path()
            .segments
            .last()
            .is_some_and(|s| s.ident == "instrument")
            && a.meta
                .require_list()
                .is_ok_and(|l| l.tokens.to_string().contains("skip_all"))
    });
    if !instrumented {
        violations.push(Violation::new(
            "OBS-001",
            location,
            format!(
                "workflow `{name}` needs #[tracing::instrument(skip_all, fields(tenant_id, ...))]"
            ),
        ));
    }
    if first_param_is_opctx(&workflow.sig) && !requires_permission_first(workflow) {
        violations.push(Violation::new(
            "SEC-003",
            location,
            format!("workflow `{name}` must start with `ctx.require(permissions::...)?;`"),
        ));
    }
    if !first_param_is_opctx(&workflow.sig) {
        violations.push(Violation::new(
            "SEC-003",
            location,
            format!("workflow `{name}` must take `ctx: &OpCtx` as its first parameter"),
        ));
    }
    let called =
        Regex::new(&format!(r"\b{name}\s*\(")).is_ok_and(|call| call.is_match(&context.tests));
    if !called {
        violations.push(Violation::new(
            "TEST-005",
            location,
            format!("workflow `{name}` is not exercised by any integration test in tests/"),
        ));
    }
}

fn item_visibility(item: &syn::Item) -> Option<(String, &Visibility)> {
    match item {
        syn::Item::Fn(f) => Some((f.sig.ident.to_string(), &f.vis)),
        syn::Item::Struct(s) => Some((s.ident.to_string(), &s.vis)),
        syn::Item::Enum(e) => Some((e.ident.to_string(), &e.vis)),
        syn::Item::Const(c) => Some((c.ident.to_string(), &c.vis)),
        _ => None,
    }
}

/// The first statement is `<ctx>.require(..)?` on the first parameter.
fn requires_permission_first(workflow: &syn::ItemFn) -> bool {
    let Some(syn::FnArg::Typed(first)) = workflow.sig.inputs.first() else {
        return false;
    };
    let syn::Pat::Ident(param) = first.pat.as_ref() else {
        return false;
    };
    let Some(syn::Stmt::Expr(syn::Expr::Try(attempt), _)) = workflow.block.stmts.first() else {
        return false;
    };
    let syn::Expr::MethodCall(call) = attempt.expr.as_ref() else {
        return false;
    };
    call.method == "require"
        && matches!(call.receiver.as_ref(), syn::Expr::Path(receiver) if receiver.path.is_ident(&param.ident))
}

fn first_param_is_opctx(signature: &syn::Signature) -> bool {
    let Some(syn::FnArg::Typed(first)) = signature.inputs.first() else {
        return false;
    };
    let syn::Type::Reference(reference) = first.ty.as_ref() else {
        return false;
    };
    matches!(reference.elem.as_ref(), syn::Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "OpCtx"))
}

/// DB-011: schema ownership and migration hygiene.
///
/// SQL is lexed first (comments removed, string literals blanked) so that
/// neither a comment nor `'app.tenant_id'` can hide or fake a reference.
fn migrations(
    member: &Member,
    known_schemas: &BTreeSet<String>,
    violations: &mut Vec<Violation>,
) -> Result<()> {
    let location = format!("{}/sqlx.toml", member.dir);
    let Some(schema) = owned_schema(member)? else {
        violations.push(Violation::new(
            "DB-011",
            location,
            "schema-owning crates declare `[migrate] create-schemas = [\"<schema>\"]` and `table-name = \"<schema>._sqlx_migrations\"`",
        ));
        return Ok(());
    };
    let config: toml::Value =
        toml::from_str(&std::fs::read_to_string(member.abs_dir.join("sqlx.toml"))?)?;
    let table = config
        .get("migrate")
        .and_then(|m| m.get("table-name"))
        .and_then(toml::Value::as_str);
    if table != Some(format!("{schema}._sqlx_migrations").as_str()) {
        violations.push(Violation::new(
            "DB-011",
            location,
            format!("table-name must be `{schema}._sqlx_migrations`"),
        ));
    }

    let rules = MigrationRules::new()?;
    for entry in std::fs::read_dir(member.abs_dir.join("migrations"))? {
        let path = entry?.path();
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned();
        let location = format!("{}/migrations/{file_name}", member.dir);
        if !rules.name.is_match(&file_name) {
            violations.push(Violation::new(
                "DB-011",
                location.clone(),
                "name migrations `<YYYYMMDDHHMMSS>_<description>.sql` (cargo rusticate new migration)",
            ));
        }
        let sql = lex_sql(&std::fs::read_to_string(&path)?);
        for message in rules.check(&sql, &schema, known_schemas) {
            violations.push(Violation::new("DB-011", location.clone(), message));
        }
    }
    Ok(())
}

/// Platform objects every context may reference from its migrations.
const SHARED_PLATFORM_OBJECTS: &[&str] = &["current_tenant_id", "current_actor_id"];

struct MigrationRules {
    name: Regex,
    /// `CREATE [OR REPLACE] <kind> <name>` — the created object.
    created: Regex,
    /// `CREATE INDEX|POLICY|TRIGGER ... ON <name>`, `GRANT|REVOKE ... ON [TABLE] <name>`.
    targets: Regex,
    /// `ALTER TABLE [IF EXISTS] [ONLY] <name>`.
    altered: Regex,
    /// Any `schema.object` reference.
    reference: Regex,
}

impl MigrationRules {
    fn new() -> Result<Self> {
        let ident = r#"("?[a-z_][a-z0-9_]*"?)"#;
        Ok(Self {
            name: Regex::new(r"^\d{14}_[a-z0-9_]+\.sql$")?,
            created: Regex::new(&format!(
                r"(?i)\bCREATE\s+(?:OR\s+REPLACE\s+)?(?:UNLOGGED\s+)?(?:TABLE|VIEW|MATERIALIZED\s+VIEW|SEQUENCE|TYPE|FUNCTION|PROCEDURE|DOMAIN)(?:\s+IF\s+NOT\s+EXISTS)?\s+{ident}(\s*\.)?"
            ))?,
            targets: Regex::new(&format!(
                r"(?i)\b(?:CREATE\s+(?:UNIQUE\s+)?INDEX|CREATE\s+POLICY|CREATE\s+(?:OR\s+REPLACE\s+)?TRIGGER|GRANT|REVOKE)\b[^;]*?\bON\s+(?:ONLY\s+|TABLE\s+)?{ident}(\s*\.)?"
            ))?,
            altered: Regex::new(&format!(
                r"(?i)\bALTER\s+TABLE\s+(?:IF\s+EXISTS\s+)?(?:ONLY\s+)?{ident}(\s*\.)?"
            ))?,
            reference: Regex::new(r#"(?i)"?\b([a-z_][a-z0-9_]*)"?\s*\.\s*"?([a-z_][a-z0-9_]*)"#)?,
        })
    }

    fn check(&self, sql: &str, schema: &str, known_schemas: &BTreeSet<String>) -> Vec<String> {
        // Keywords that may follow `ON` in GRANT/REVOKE without naming a relation.
        const NOT_RELATIONS: &[&str] = &[
            "schema",
            "function",
            "functions",
            "procedure",
            "routine",
            "all",
            "sequence",
            "database",
            "type",
            "domain",
            "language",
            "large",
            "foreign",
            "tablespace",
        ];
        let mut messages = Vec::new();
        for regex in [&self.created, &self.targets, &self.altered] {
            for capture in regex.captures_iter(sql) {
                let (Some(name), qualified) = (capture.get(1), capture.get(2).is_some()) else {
                    continue;
                };
                let name = name.as_str().trim_matches('"').to_ascii_lowercase();
                if !qualified && !NOT_RELATIONS.contains(&name.as_str()) {
                    messages.push(format!(
                        "`{name}` is not schema-qualified: write `{schema}.{name}`"
                    ));
                } else if qualified && name != schema {
                    // Creating, altering or granting on another schema's
                    // objects is never allowed, shared platform functions
                    // included: they may be called, not redefined.
                    messages.push(format!(
                        "creates, alters or grants on objects in schema `{name}`; this crate owns only `{schema}`"
                    ));
                }
            }
        }
        for capture in self.reference.captures_iter(sql) {
            let (Some(target), Some(object)) = (capture.get(1), capture.get(2)) else {
                continue;
            };
            let target = target.as_str().to_ascii_lowercase();
            let object = object.as_str().to_ascii_lowercase();
            let shared = target == "platform" && SHARED_PLATFORM_OBJECTS.contains(&object.as_str());
            if known_schemas.contains(&target) && target != schema && !shared {
                messages.push(format!(
                    "references `{target}.{object}`; this crate owns only `{schema}` (other schemas are reached through their owner's API or events)"
                ));
            }
        }
        messages
    }
}

/// Removes `--` and `/* */` comments and blanks the contents of string
/// literals, keeping line breaks.
fn lex_sql(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '-' if chars.peek() == Some(&'-') => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut previous = ' ';
                for next in chars.by_ref() {
                    if next == '\n' {
                        out.push('\n');
                    }
                    if previous == '*' && next == '/' {
                        break;
                    }
                    previous = next;
                }
                out.push(' ');
            }
            '\'' => {
                out.push('\'');
                while let Some(next) = chars.next() {
                    if next == '\'' {
                        if chars.peek() == Some(&'\'') {
                            chars.next();
                            continue;
                        }
                        break;
                    }
                    if next == '\n' {
                        out.push('\n');
                    }
                }
                out.push('\'');
            }
            other => out.push(other),
        }
    }
    out
}

/// DB-012: an app migrates every schema-owning crate it depends on.
fn migrators_registered(workspace: &Workspace, violations: &mut Vec<Violation>) -> Result<()> {
    for app in workspace.members.iter().filter(|m| m.tier == "app") {
        let migrate_rs = app.abs_dir.join("src/migrate.rs");
        if !migrate_rs.exists() {
            continue;
        }
        let text = std::fs::read_to_string(&migrate_rs)?;
        let Some(package) = workspace.package(&app.name) else {
            continue;
        };
        for dependency in &package.dependencies {
            let Some(owner) = workspace.member(&dependency.name) else {
                continue;
            };
            if owner.has_migrations() {
                let ident = owner.name.replace('-', "_");
                if !text.contains(&format!("{ident}::MIGRATOR")) {
                    violations.push(Violation::new(
                        "DB-012",
                        format!("{}/src/migrate.rs", app.dir),
                        format!("`{ident}::MIGRATOR` must be listed in MIGRATORS"),
                    ));
                }
            }
        }
        if !text.contains("datastore::MIGRATOR") {
            violations.push(Violation::new(
                "DB-012",
                format!("{}/src/migrate.rs", app.dir),
                "the platform migrator (datastore::MIGRATOR) must run first",
            ));
        }
    }
    Ok(())
}

fn read_all(dir: &Path) -> Result<String> {
    let mut text = String::new();
    if !dir.exists() {
        return Ok(text);
    }
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.path().extension().is_some_and(|e| e == "rs") {
            text.push_str(&std::fs::read_to_string(entry.path())?);
        }
    }
    Ok(text)
}
