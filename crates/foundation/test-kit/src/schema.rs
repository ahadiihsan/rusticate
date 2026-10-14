//! Catalog-level schema guard (SOP DB-004, DB-005, TEST-004).
//!
//! Instead of trusting every migration author to remember every rule, this
//! inspects the *migrated* catalog and fails on any table, policy, key, view,
//! function or grant that could weaken tenant isolation. Each product runs it
//! once against all of its migrators (`apps/api/tests/schema_guard.rs`).

use sqlx::PgPool;

/// The only accepted tenant predicate, exactly as PostgreSQL deparses
/// `tenant_id = (SELECT platform.current_tenant_id())`. Exact matching means a
/// widened policy (`... OR true`) cannot pass.
pub const CANONICAL_TENANT_PREDICATE: &str =
    "(tenant_id = ( SELECT platform.current_tenant_id() AS current_tenant_id))";

/// Relations in application schemas (everything but system schemas and `public`).
const APP_RELATIONS: &str = "FROM pg_class c \
     JOIN pg_namespace n ON n.oid = c.relnamespace \
     WHERE n.nspname NOT IN ('pg_catalog', 'information_schema', 'public') \
       AND n.nspname NOT LIKE 'pg\\_%'";

/// `attnum` of the relation's `tenant_id` column, NULL if it has none.
const TENANT_ATTNUM: &str = "(SELECT a.attnum FROM pg_attribute a \
     WHERE a.attrelid = c.oid AND a.attname = 'tenant_id' AND NOT a.attisdropped)";

struct Rule {
    id: &'static str,
    description: &'static str,
    /// Returns one text row per offending object.
    sql: String,
}

#[expect(
    clippy::too_many_lines,
    reason = "a flat catalogue of independent rules reads best in one place"
)]
fn rules() -> Vec<Rule> {
    let tenant_table = format!("c.relkind IN ('r', 'p') AND {TENANT_ATTNUM} IS NOT NULL");
    let canonical = CANONICAL_TENANT_PREDICATE;
    vec![
        Rule {
            id: "DB-004a",
            description: "tenant table must ENABLE and FORCE row level security",
            sql: format!(
                "SELECT format('%I.%I', n.nspname, c.relname) {APP_RELATIONS} \
                 AND {tenant_table} AND NOT (c.relrowsecurity AND c.relforcerowsecurity)"
            ),
        },
        Rule {
            id: "DB-004b",
            description: "tenant table must have a PERMISSIVE FOR ALL policy using the canonical tenant predicate in USING and WITH CHECK",
            sql: format!(
                "SELECT format('%I.%I', n.nspname, c.relname) {APP_RELATIONS} \
                 AND {tenant_table} AND NOT EXISTS ( \
                     SELECT 1 FROM pg_policy p \
                     WHERE p.polrelid = c.oid AND p.polpermissive AND p.polcmd = '*' \
                       AND pg_get_expr(p.polqual, p.polrelid) = '{canonical}' \
                       AND pg_get_expr(p.polwithcheck, p.polrelid) = '{canonical}')"
            ),
        },
        Rule {
            id: "DB-004c",
            description: "every PERMISSIVE policy on a tenant table must use exactly the canonical tenant predicate (permissive policies are OR-ed: one `USING (true)` opens the table)",
            sql: format!(
                "SELECT format('%I.%I policy %I', n.nspname, c.relname, p.polname) \
                 FROM pg_policy p \
                 JOIN pg_class c ON c.oid = p.polrelid \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE p.polpermissive AND {TENANT_ATTNUM} IS NOT NULL \
                   AND (coalesce(pg_get_expr(p.polqual, p.polrelid), '{canonical}') <> '{canonical}' \
                     OR coalesce(pg_get_expr(p.polwithcheck, p.polrelid), '{canonical}') <> '{canonical}')"
            ),
        },
        Rule {
            id: "DB-004d",
            description: "tenant_id must be NOT NULL",
            sql: format!(
                "SELECT format('%I.%I', n.nspname, c.relname) {APP_RELATIONS} \
                 AND c.relkind IN ('r', 'p') AND EXISTS ( \
                     SELECT 1 FROM pg_attribute a \
                     WHERE a.attrelid = c.oid AND a.attname = 'tenant_id' \
                       AND NOT a.attisdropped AND NOT a.attnotnull)"
            ),
        },
        Rule {
            id: "DB-004e",
            description: "a table without tenant_id must be declared global: COMMENT ON TABLE ... IS 'tenancy:global <reason>'",
            sql: format!(
                "SELECT format('%I.%I', n.nspname, c.relname) {APP_RELATIONS} \
                 AND c.relkind IN ('r', 'p') AND {TENANT_ATTNUM} IS NULL \
                 AND c.relname <> '_sqlx_migrations' \
                 AND coalesce(obj_description(c.oid, 'pg_class'), '') NOT LIKE 'tenancy:global%'"
            ),
        },
        Rule {
            id: "DB-004f",
            description: "views must set security_invoker = true (otherwise they run with the owner's privileges)",
            sql: format!(
                "SELECT format('%I.%I', n.nspname, c.relname) {APP_RELATIONS} \
                 AND c.relkind IN ('v', 'm') \
                 AND NOT coalesce(c.reloptions @> ARRAY['security_invoker=true'], false)"
            ),
        },
        Rule {
            id: "DB-004g",
            description: "SECURITY DEFINER functions bypass RLS; each needs COMMENT ON FUNCTION ... IS 'security-definer:reviewed <reason>'",
            sql: "SELECT p.oid::regprocedure::text FROM pg_proc p \
                  JOIN pg_namespace n ON n.oid = p.pronamespace \
                  WHERE p.prosecdef \
                    AND n.nspname NOT IN ('pg_catalog', 'information_schema') \
                    AND n.nspname NOT LIKE 'pg\\_%' \
                    AND coalesce(obj_description(p.oid, 'pg_proc'), '') NOT LIKE 'security-definer:reviewed%'"
                .to_owned(),
        },
        Rule {
            id: "DB-005a",
            description: "a foreign key between tenant tables must include tenant_id on both sides at the same position (FK checks bypass RLS)",
            sql: "SELECT format('%s (%s -> %s)', con.conname, con.conrelid::regclass, con.confrelid::regclass) \
                  FROM pg_constraint con \
                  JOIN pg_attribute sa ON sa.attrelid = con.conrelid AND sa.attname = 'tenant_id' AND NOT sa.attisdropped \
                  JOIN pg_attribute da ON da.attrelid = con.confrelid AND da.attname = 'tenant_id' AND NOT da.attisdropped \
                  WHERE con.contype = 'f' \
                    AND coalesce(array_position(con.conkey, sa.attnum) <> array_position(con.confkey, da.attnum), true)"
                .to_owned(),
        },
        Rule {
            id: "DB-005b",
            description: "a unique index on a domain tenant table must include tenant_id (otherwise uniqueness leaks across tenants)",
            sql: format!(
                "SELECT i.indexrelid::regclass::text \
                 FROM pg_index i \
                 JOIN pg_class c ON c.oid = i.indrelid \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE i.indisunique \
                   AND n.nspname NOT IN ('pg_catalog', 'information_schema', 'public', 'platform') \
                   AND n.nspname NOT LIKE 'pg\\_%' \
                   AND {TENANT_ATTNUM} IS NOT NULL \
                   AND NOT ({TENANT_ATTNUM} = ANY (i.indkey::int2[]))"
            ),
        },
        Rule {
            id: "DB-005c",
            description: "every table must have a primary key",
            sql: format!(
                "SELECT format('%I.%I', n.nspname, c.relname) {APP_RELATIONS} \
                 AND c.relkind IN ('r', 'p') \
                 AND NOT EXISTS (SELECT 1 FROM pg_constraint k WHERE k.conrelid = c.oid AND k.contype = 'p')"
            ),
        },
        Rule {
            id: "DB-002a",
            description: "app_rw must not hold TRUNCATE (TRUNCATE ignores row-level security)",
            sql: format!(
                "SELECT format('%I.%I', n.nspname, c.relname) {APP_RELATIONS} \
                 AND c.relkind IN ('r', 'p') AND has_table_privilege('app_rw', c.oid, 'TRUNCATE')"
            ),
        },
        Rule {
            id: "DB-002b",
            description: "app_rw must not be superuser, BYPASSRLS, or own schema objects",
            sql: "SELECT r.rolname::text FROM pg_roles r \
                  WHERE r.rolname = 'app_rw' \
                    AND (r.rolsuper OR r.rolbypassrls OR EXISTS ( \
                        SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                        WHERE c.relowner = r.oid AND n.nspname NOT IN ('pg_catalog', 'information_schema')))"
                .to_owned(),
        },
        Rule {
            id: "DB-002c",
            description: "app_system (BYPASSRLS) may hold privileges only on platform.* relations",
            sql: "SELECT format('%I.%I', n.nspname, c.relname) \
                  FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                  WHERE c.relkind IN ('r', 'p', 'v', 'm', 'S') \
                    AND n.nspname NOT IN ('pg_catalog', 'information_schema', 'platform') \
                    AND n.nspname NOT LIKE 'pg\\_%' \
                    AND has_table_privilege('app_system', c.oid, 'SELECT, INSERT, UPDATE, DELETE, TRUNCATE')"
                .to_owned(),
        },
        Rule {
            id: "DB-011b",
            description: "no application objects in schema public (every context owns its schema)",
            sql: "SELECT format('public.%I', c.relname) \
                  FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                  WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p', 'v', 'm', 'S', 'f')"
                .to_owned(),
        },
        Rule {
            id: "DB-003a",
            description: "platform.current_tenant_id() must keep its canonical definition (STABLE, not SECURITY DEFINER, NULLIF over app.tenant_id)",
            sql: format!(
                "SELECT 'platform.current_tenant_id()' WHERE NOT EXISTS ( \
                     SELECT 1 FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
                     WHERE n.nspname = 'platform' AND p.proname = 'current_tenant_id' \
                       AND NOT p.prosecdef AND p.provolatile = 's' \
                       AND btrim(regexp_replace(p.prosrc, '\\s+', ' ', 'g')) = '{CANONICAL_TENANT_FUNCTION_BODY}') \
                 OR (SELECT count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
                     WHERE n.nspname = 'platform' AND p.proname = 'current_tenant_id') <> 1"
            ),
        },
    ]
}

/// Body of `platform.current_tenant_id()`, SQL-quoted. Every tenant policy
/// depends on it, so a migration that redefines it is rejected (DB-003a).
const CANONICAL_TENANT_FUNCTION_BODY: &str =
    "SELECT NULLIF(current_setting(''app.tenant_id'', true), '''')::uuid";

/// Every violated rule as `"<rule id> <description>: <object>"`.
pub async fn schema_violations(owner: &PgPool) -> Vec<String> {
    let mut violations = Vec::new();
    for rule in rules() {
        let offenders: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(rule.sql))
            .fetch_all(owner)
            .await
            .expect("schema guard query");
        violations.extend(
            offenders
                .into_iter()
                .map(|object| format!("{} {}: {object}", rule.id, rule.description)),
        );
    }
    violations
}

/// Fails the test with every violation listed.
pub async fn assert_schema_invariants(owner: &PgPool) {
    let violations = schema_violations(owner).await;
    assert!(
        violations.is_empty(),
        "schema invariants violated:\n  {}",
        violations.join("\n  ")
    );
}
