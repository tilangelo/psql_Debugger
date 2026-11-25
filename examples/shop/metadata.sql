-- Представления для списка схем и объектов тестовой базы.
BEGIN;
CREATE SCHEMA orm;
CREATE VIEW orm.schemas_view AS
SELECT nspname::information_schema.sql_identifier AS schema_name
FROM pg_catalog.pg_namespace
WHERE nspname IN ('sales', 'billing', 'audit')
  AND pg_catalog.has_schema_privilege(oid, 'USAGE');

CREATE VIEW orm.functions_view AS
SELECT
    current_database()::information_schema.sql_identifier AS routine_catalog,
    n.nspname::information_schema.sql_identifier AS routine_schema,
    p.proname::information_schema.sql_identifier AS routine_name,
    p.oid,
    pg_catalog.obj_description(p.oid, 'pg_proc') AS description,
    CASE p.prokind WHEN 'p' THEN 'PROCEDURE' ELSE 'FUNCTION' END
        ::information_schema.character_data AS routine_type,
    l.lanname,
    p.prosrc,
    pg_catalog.pg_get_functiondef(p.oid) AS func_def,
    p.proretset,
    pg_catalog.pg_get_function_identity_arguments(p.oid) AS identity_arguments
FROM pg_catalog.pg_proc p
JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
JOIN pg_catalog.pg_language l ON l.oid = p.prolang
JOIN orm.schemas_view sv ON n.nspname = sv.schema_name::name
WHERE p.prokind IN ('f', 'p')
  AND (pg_catalog.pg_has_role(p.proowner, 'USAGE')
       OR pg_catalog.has_function_privilege(p.oid, 'EXECUTE'));
COMMIT;
