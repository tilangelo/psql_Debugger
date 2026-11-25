CREATE TABLE public.debug_writes(value integer);

CREATE PROCEDURE public.write_for_debug(IN input_value integer, INOUT result integer)
LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO public.debug_writes VALUES (input_value);
    result := input_value + 1;
END;
$$;

CREATE PROCEDURE public.write_then_commit(IN input_value integer)
LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO public.debug_writes VALUES (input_value);
    COMMIT;
END;
$$;

CREATE PROCEDURE public.write_then_wait(IN input_value integer)
LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO public.debug_writes VALUES (input_value);
    PERFORM pg_sleep(30);
END;
$$;
