-- =============================================================================
-- Platform role contract (SOP DB-002). LOCAL DEVELOPMENT ONLY.
-- In real environments these roles are provisioned by infrastructure code with
-- secrets from the secret manager. The names are a contract: migrations GRANT
-- to them, and the services verify their own role attributes at startup.
--
--   app_owner   owns schemas and tables, runs migrations. NOT superuser, so
--               FORCE ROW LEVEL SECURITY applies to it as well.
--   app_rw      the API and event consumers. Subject to RLS. Never BYPASSRLS.
--   app_system  outbox relay and maintenance jobs. BYPASSRLS, but GRANTs limit
--               it to platform.* tables only.
-- =============================================================================

CREATE ROLE app_owner  LOGIN PASSWORD 'app_owner'  NOSUPERUSER NOBYPASSRLS CREATEDB;
CREATE ROLE app_rw     LOGIN PASSWORD 'app_rw'     NOSUPERUSER NOBYPASSRLS NOCREATEDB NOCREATEROLE;
CREATE ROLE app_system LOGIN PASSWORD 'app_system' NOSUPERUSER BYPASSRLS   NOCREATEDB NOCREATEROLE;

-- The owner may SET ROLE to the runtime roles: tests use this to execute as
-- exactly the role production uses (test_kit::TestDb).
GRANT app_rw     TO app_owner WITH INHERIT FALSE, SET TRUE;
GRANT app_system TO app_owner WITH INHERIT FALSE, SET TRUE;

-- Server-side safety nets (OPS-005). Applied per role so a runaway query or a
-- forgotten open transaction can never pin a connection or the xmin horizon.
ALTER ROLE app_rw     SET statement_timeout = '5s';
ALTER ROLE app_rw     SET lock_timeout = '3s';
ALTER ROLE app_rw     SET idle_in_transaction_session_timeout = '15s';
ALTER ROLE app_system SET statement_timeout = '30s';
ALTER ROLE app_system SET lock_timeout = '5s';
ALTER ROLE app_system SET idle_in_transaction_session_timeout = '60s';

CREATE DATABASE rusticate OWNER app_owner;
