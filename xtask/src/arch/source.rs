//! Source rules, evaluated on the syntax tree of every `src/**/*.rs` file
//! (test modules excluded): restricted capabilities, engine purity, ids,
//! SQL hygiene, DTO strictness, unfinished scaffolds.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Result;
use regex::Regex;
use syn::visit::{self, Visit};
use walkdir::WalkDir;

use super::Violation;
use crate::workspace::{Member, Workspace, owned_schema};

/// Matches the paths a capability covers.
type PathPredicate = dyn Fn(&str) -> bool;

const RAW_SQL_FNS: &[&str] = &[
    "sqlx::query",
    "sqlx::query_as",
    "sqlx::query_scalar",
    "sqlx::query_with",
    "sqlx::query_as_with",
    "sqlx::query_scalar_with",
    "sqlx::raw_sql",
];
const SQL_MACROS: &[&str] = &[
    "query",
    "query_as",
    "query_scalar",
    "query_unchecked",
    "query_as_unchecked",
    "query_scalar_unchecked",
];

pub fn check(workspace: &Workspace) -> Result<Vec<Violation>> {
    let schemas: BTreeSet<String> = workspace
        .members
        .iter()
        .filter_map(|m| owned_schema(m).ok().flatten())
        .collect();
    let schema_ref = Regex::new(r#"(?i)"?\b([a-z_][a-z0-9_]*)"?\s*\.\s*"?[a-z_][a-z0-9_]*"#)?;
    let session = Regex::new(
        r"(?i)\bset_config\s*\(|\bSET\s+(?:LOCAL\s+|SESSION\s+)?(?:ROLE|SESSION\s+AUTHORIZATION)\b|\bSET\s+(?:LOCAL\s+|SESSION\s+)?app\.",
    )?;

    let mut violations = Vec::new();
    for member in workspace.members.iter().filter(|m| !m.exempt) {
        let own_schema = owned_schema(member)?;
        for path in rust_files(&member.abs_dir.join("src")) {
            let text = std::fs::read_to_string(&path)?;
            let location = |line: usize| format!("{}:{line}", relative(&path));

            text_rules(
                workspace,
                member,
                &text,
                &session,
                &location,
                &mut violations,
            );

            let file = match syn::parse_file(&text) {
                Ok(file) => file,
                Err(err) => {
                    violations.push(Violation::new(
                        "SRC-000",
                        location(1),
                        format!("cannot parse: {err}"),
                    ));
                    continue;
                }
            };
            let mut facts = Facts::default();
            facts.visit_file(&file);

            capability_rules(workspace, member, &facts, &location, &mut violations);
            if member.tier == "engine" {
                engine_rules(&facts, &location, &mut violations);
            }
            for (line, name) in &facts.request_structs_without_deny {
                violations.push(Violation::new(
                    "API-003",
                    location(*line),
                    format!("request DTO `{name}` must use #[serde(deny_unknown_fields)]"),
                ));
            }
            for line in &facts.unbounded_fetch_all {
                violations.push(Violation::new(
                    "DB-007",
                    location(*line),
                    "fetch_all on a query without LIMIT or `= ANY(...)` is unbounded: page it (keyset) or bound it",
                ));
            }
            for (line, path) in &facts.paths {
                if path.ends_with("new_v4") {
                    violations.push(Violation::new(
                        "ID-001",
                        location(*line),
                        "identifiers are UUIDv7: use Uuid::now_v7()",
                    ));
                }
            }
            if member.tier == "domain" {
                for (line, sql) in &facts.macro_sql {
                    for referenced in schema_ref.captures_iter(sql).filter_map(|c| c.get(1)) {
                        let schema = referenced.as_str().to_ascii_lowercase();
                        let schema = schema.as_str();
                        if schemas.contains(schema) && own_schema.as_deref() != Some(schema) {
                            violations.push(Violation::new(
                                "DB-013",
                                location(*line),
                                format!(
                                    "SQL references schema `{schema}`; a context may only query its own schema \
                                     (use the owning crate's API or events)"
                                ),
                            ));
                        }
                    }
                }
            }
        }
        // Unfinished scaffolds anywhere in the crate: code, tests, migrations.
        for path in files_with(&member.abs_dir, &["rs", "sql"]) {
            let text = std::fs::read_to_string(&path)?;
            for (index, line) in text.lines().enumerate() {
                if line.contains("SCAFFOLD:") {
                    violations.push(Violation::new(
                        "SCAF-001",
                        format!("{}:{}", relative(&path), index + 1),
                        "unfinished scaffold: replace the placeholder and remove the SCAFFOLD marker",
                    ));
                }
            }
        }
    }
    Ok(violations)
}

fn text_rules(
    workspace: &Workspace,
    member: &Member,
    text: &str,
    session: &Regex,
    location: &dyn Fn(usize) -> String,
    violations: &mut Vec<Violation>,
) {
    let Some(capability) = workspace.architecture.capabilities.get("session_settings") else {
        return;
    };
    if capability.allowed.contains(&member.name) {
        return;
    }
    for (index, line) in text.lines().enumerate() {
        let code = line.trim_start();
        if !code.starts_with("//") && session.is_match(code) {
            violations.push(Violation::new(
                capability.rule.clone(),
                location(index + 1),
                format!(
                    "{} is reserved to: {}",
                    capability.description,
                    capability.allowed.join(", ")
                ),
            ));
        }
    }
}

fn capability_rules(
    workspace: &Workspace,
    member: &Member,
    facts: &Facts,
    location: &dyn Fn(usize) -> String,
    violations: &mut Vec<Violation>,
) {
    let checks: [(&str, &PathPredicate); 4] = [
        ("mint", &|p| p.starts_with("op_context::mint")),
        ("system_store", &|p| {
            p.split("::").any(|s| s == "SystemStore" || s == "SystemTx")
        }),
        ("raw_sql", &|p| RAW_SQL_FNS.contains(&p)),
        ("pool", &|p| {
            p.split("::").any(|s| s == "PgPool" || s == "PgPoolOptions") || p == "sqlx::Pool"
        }),
    ];
    for (name, matches) in checks {
        let Some(capability) = workspace.architecture.capabilities.get(name) else {
            continue;
        };
        if capability.allowed.contains(&member.name) {
            continue;
        }
        for (line, path) in facts.paths.iter().filter(|(_, p)| matches(p)) {
            violations.push(Violation::new(
                capability.rule.clone(),
                location(*line),
                format!(
                    "`{path}`: {} is reserved to: {}",
                    capability.description,
                    capability.allowed.join(", ")
                ),
            ));
        }
    }
}

fn engine_rules(
    facts: &Facts,
    location: &dyn Fn(usize) -> String,
    violations: &mut Vec<Violation>,
) {
    for (line, name) in &facts.async_fns {
        violations.push(Violation::new(
            "ENG-001",
            location(*line),
            format!("engines are synchronous: `async fn {name}`"),
        ));
    }
    for line in &facts.awaits {
        violations.push(Violation::new(
            "ENG-001",
            location(*line),
            "engines are synchronous: `.await`",
        ));
    }
    for line in &facts.floats {
        violations.push(Violation::new(
            "ENG-003",
            location(*line),
            "no floating point in engines: use integer minor units / basis points",
        ));
    }
}

fn rust_files(dir: &Path) -> Vec<std::path::PathBuf> {
    files_with(dir, &["rs"])
}

fn files_with(dir: &Path, extensions: &[&str]) -> Vec<std::path::PathBuf> {
    WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| extensions.contains(&ext))
        })
        .filter(|e| !e.path().components().any(|c| c.as_os_str() == "target"))
        .map(walkdir::DirEntry::into_path)
        .collect()
}

fn relative(path: &Path) -> String {
    let root = crate::workspace::root();
    path.strip_prefix(&root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

/// Facts collected from one file's syntax tree (test modules skipped).
#[derive(Default)]
struct Facts {
    /// Every path in code and every `use` leaf, as `a::b::c`, with its line.
    paths: Vec<(usize, String)>,
    async_fns: Vec<(usize, String)>,
    awaits: Vec<usize>,
    floats: Vec<usize>,
    unbounded_fetch_all: Vec<usize>,
    request_structs_without_deny: Vec<(usize, String)>,
    /// Token text of every `sqlx::query*!` macro.
    macro_sql: Vec<(usize, String)>,
}

fn line_of(spanned: &impl syn::spanned::Spanned) -> usize {
    spanned.span().start().line
}

fn join(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg") && {
            let tokens = attr
                .meta
                .require_list()
                .map(|l| l.tokens.to_string())
                .unwrap_or_default();
            tokens
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .any(|t| t == "test")
        }
    })
}

fn use_leaves(tree: &syn::UseTree, prefix: &mut Vec<String>, out: &mut Vec<(usize, String)>) {
    match tree {
        syn::UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            use_leaves(&p.tree, prefix, out);
            prefix.pop();
        }
        syn::UseTree::Name(n) => {
            let mut full = prefix.clone();
            full.push(n.ident.to_string());
            out.push((line_of(&n.ident), full.join("::")));
        }
        syn::UseTree::Rename(r) => {
            let mut full = prefix.clone();
            full.push(r.ident.to_string());
            out.push((line_of(&r.ident), full.join("::")));
        }
        syn::UseTree::Glob(g) => out.push((line_of(g), format!("{}::*", prefix.join("::")))),
        syn::UseTree::Group(g) => {
            for item in &g.items {
                use_leaves(item, prefix, out);
            }
        }
    }
}

/// A real `LIMIT <n|$n>` or `= ANY(`; a column named `rate_limit` is not a bound.
fn sql_is_bounded(sql: &str) -> bool {
    static BOUND: std::sync::LazyLock<Option<Regex>> =
        std::sync::LazyLock::new(|| Regex::new(r"(?i)\bLIMIT\s+(?:\$\d+|\d+)\b|=\s*ANY\s*\(").ok());
    BOUND.as_ref().is_some_and(|bound| bound.is_match(sql))
}

/// Token-level scan of a macro body. Macros are opaque to the syntax tree,
/// so `select!`, `join!`, `vec!`, `format!`, ... would otherwise hide paths,
/// `.await` and float literals from every rule.
fn scan_tokens(tokens: proc_macro2::TokenStream, facts: &mut Facts) {
    use proc_macro2::{Spacing, TokenTree};
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    let mut index = 0;
    while let Some(tree) = trees.get(index) {
        match tree {
            TokenTree::Group(group) => scan_tokens(group.stream(), facts),
            TokenTree::Ident(ident) => {
                // Collect `a::b::c` sequences.
                let line = ident.span().start().line;
                let mut segments = vec![ident.to_string()];
                let mut next = index;
                while let (
                    Some(TokenTree::Punct(first)),
                    Some(TokenTree::Punct(second)),
                    Some(TokenTree::Ident(segment)),
                ) = (
                    trees.get(next + 1),
                    trees.get(next + 2),
                    trees.get(next + 3),
                ) {
                    if first.as_char() == ':'
                        && first.spacing() == Spacing::Joint
                        && second.as_char() == ':'
                    {
                        segments.push(segment.to_string());
                        next += 3;
                    } else {
                        break;
                    }
                }
                if ident == "await"
                    && index > 0
                    && matches!(trees.get(index - 1), Some(TokenTree::Punct(dot)) if dot.as_char() == '.')
                {
                    facts.awaits.push(line);
                } else {
                    facts.paths.push((line, segments.join("::")));
                }
                index = next;
            }
            TokenTree::Literal(literal) => {
                if syn::parse_str::<syn::LitFloat>(&literal.to_string()).is_ok() {
                    facts.floats.push(literal.span().start().line);
                }
            }
            TokenTree::Punct(_) => {}
        }
        index += 1;
    }
}

impl<'ast> Visit<'ast> for Facts {
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if !is_cfg_test(&node.attrs) {
            visit::visit_item_mod(self, node);
        }
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        if is_cfg_test(&node.attrs) {
            return;
        }
        if node.sig.asyncness.is_some() {
            self.async_fns
                .push((line_of(&node.sig.ident), node.sig.ident.to_string()));
        }
        visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        if node.sig.asyncness.is_some() {
            self.async_fns
                .push((line_of(&node.sig.ident), node.sig.ident.to_string()));
        }
        visit::visit_impl_item_fn(self, node);
    }

    fn visit_item_use(&mut self, node: &'ast syn::ItemUse) {
        if !is_cfg_test(&node.attrs) {
            use_leaves(&node.tree, &mut Vec::new(), &mut self.paths);
        }
    }

    fn visit_path(&mut self, node: &'ast syn::Path) {
        if let Some(first) = node.segments.first() {
            self.paths.push((line_of(&first.ident), join(node)));
        }
        visit::visit_path(self, node);
    }

    fn visit_expr_await(&mut self, node: &'ast syn::ExprAwait) {
        self.awaits.push(line_of(&node.await_token));
        visit::visit_expr_await(self, node);
    }

    fn visit_type_path(&mut self, node: &'ast syn::TypePath) {
        if node.qself.is_none() && (node.path.is_ident("f32") || node.path.is_ident("f64")) {
            self.floats.push(line_of(&node.path));
        }
        visit::visit_type_path(self, node);
    }

    fn visit_lit_float(&mut self, node: &'ast syn::LitFloat) {
        self.floats.push(line_of(node));
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        // The macro *path* is deliberately not recorded: `sqlx::query!` is the
        // compile-time-checked form and must not count as `sqlx::query`. The
        // macro *body* is scanned token by token.
        let is_sql = node
            .path
            .segments
            .last()
            .is_some_and(|s| SQL_MACROS.contains(&s.ident.to_string().as_str()));
        if is_sql {
            self.macro_sql
                .push((line_of(&node.path), node.tokens.to_string()));
        }
        scan_tokens(node.tokens.clone(), self);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if node.method == "fetch_all" {
            let mut receiver = node.receiver.as_ref();
            while let syn::Expr::MethodCall(call) = receiver {
                receiver = call.receiver.as_ref();
            }
            if let syn::Expr::Macro(mac) = receiver
                && !sql_is_bounded(&mac.mac.tokens.to_string())
            {
                self.unbounded_fetch_all.push(line_of(&node.method));
            }
        }
        visit::visit_expr_method_call(self, node);
    }

    fn visit_item_struct(&mut self, node: &'ast syn::ItemStruct) {
        let name = node.ident.to_string();
        let tokens_of = |attr: &syn::Attribute| {
            attr.meta
                .require_list()
                .map(|l| l.tokens.to_string())
                .unwrap_or_default()
        };
        let deserializes = node
            .attrs
            .iter()
            .any(|a| a.path().is_ident("derive") && tokens_of(a).contains("Deserialize"));
        let denies = node
            .attrs
            .iter()
            .any(|a| a.path().is_ident("serde") && tokens_of(a).contains("deny_unknown_fields"));
        if name.ends_with("Request") && deserializes && !denies {
            self.request_structs_without_deny
                .push((line_of(&node.ident), name));
        }
        visit::visit_item_struct(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(source: &str) -> Facts {
        let mut facts = Facts::default();
        facts.visit_file(&syn::parse_file(source).unwrap());
        facts
    }

    #[test]
    fn collects_use_leaves_and_paths() {
        let facts =
            facts("use op_context::{mint, OpCtx}; fn f() { datastore::SystemStore::connect(); }");
        let paths: Vec<_> = facts.paths.iter().map(|(_, p)| p.as_str()).collect();
        assert!(paths.contains(&"op_context::mint"));
        assert!(paths.contains(&"datastore::SystemStore::connect"));
    }

    #[test]
    fn sql_macros_are_not_raw_sql_calls() {
        let facts = facts(r#"fn f() { sqlx::query!("SELECT 1"); sqlx::query("SELECT 2"); }"#);
        let raw: Vec<_> = facts
            .paths
            .iter()
            .filter(|(_, p)| p == "sqlx::query")
            .collect();
        assert_eq!(raw.len(), 1);
        assert_eq!(facts.macro_sql.len(), 1);
    }

    #[test]
    fn detects_unbounded_fetch_all() {
        let unbounded =
            facts(r#"async fn f() { sqlx::query!("SELECT * FROM t").fetch_all(c).await; }"#);
        let bounded = facts(
            r#"async fn f() { sqlx::query!("SELECT * FROM t LIMIT $1").fetch_all(c).await; }"#,
        );
        assert_eq!(unbounded.unbounded_fetch_all.len(), 1);
        assert_eq!(bounded.unbounded_fetch_all.len(), 0);
    }

    #[test]
    fn skips_test_modules_and_flags_lenient_requests() {
        let facts = facts(
            "#[derive(Deserialize)] struct CreateRequest { a: u8 }
             #[cfg(test)] mod tests { fn f() { let x: f64 = 1.0; } }",
        );
        assert_eq!(facts.request_structs_without_deny.len(), 1);
        assert_eq!(facts.floats.len(), 0);
    }
}
