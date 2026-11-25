-- Только для отдельной тестовой базы: создаёт 1001 функцию, включая тело на 10000 строк.
CREATE SCHEMA perf_checks;
DO $fixture$
DECLARE i integer; arguments text; body text;
BEGIN
    FOR i IN 1..1000 LOOP
        EXECUTE format('CREATE FUNCTION perf_checks.f%s() RETURNS integer LANGUAGE plpgsql AS %L', i, 'BEGIN RETURN 1; END;');
    END LOOP;
    SELECT string_agg(format('p%s integer', n), ', ' ORDER BY n) INTO arguments FROM generate_series(0,99) AS g(n);
    SELECT E'\nBEGIN\n' || string_agg(format(' -- large function measurement %s', n), E'\n' ORDER BY n) || E'\n RETURN p0;\nEND;\n' INTO body FROM generate_series(0,9999) AS g(n);
    EXECUTE format('CREATE FUNCTION perf_checks.large_body(%s) RETURNS integer LANGUAGE plpgsql AS %L', arguments, body);
END;
$fixture$;
