CREATE SCHEMA gui_checks;

CREATE FUNCTION gui_checks.calculate(input_value integer) RETURNS integer
LANGUAGE plpgsql AS $$ BEGIN RETURN input_value + 1; END; $$;

CREATE FUNCTION gui_checks.calculate(input_value text) RETURNS text
LANGUAGE plpgsql AS $$ BEGIN RETURN upper(input_value); END; $$;

CREATE FUNCTION gui_checks.no_args() RETURNS integer
LANGUAGE plpgsql AS $$ BEGIN RETURN 1; END; $$;

CREATE FUNCTION gui_checks.sql_function() RETURNS integer
LANGUAGE sql AS $$ SELECT 1 $$;

CREATE PROCEDURE gui_checks.process(IN input_value integer, INOUT counter integer, OUT result text)
LANGUAGE plpgsql AS $$
BEGIN
    counter := counter + input_value;
    result := counter::text;
END;
$$;
