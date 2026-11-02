-- Минимальная PL/pgSQL-функция для проверки pldebugger.
CREATE OR REPLACE FUNCTION public.debug_smoke(input_value integer)
RETURNS integer
LANGUAGE plpgsql
AS $$
DECLARE
    running_total integer := input_value;
BEGIN
    running_total := running_total + 1;
    running_total := running_total * 2;
    RETURN running_total;
END;
$$;

-- Отдельно проверяем объект PROCEDURE: поддержка FUNCTION не доказывает его поддержку.
CREATE OR REPLACE PROCEDURE public.debug_smoke_proc(input_value integer)
LANGUAGE plpgsql
AS $$
DECLARE
    running_total integer := input_value;
BEGIN
    running_total := running_total + 1;
    running_total := running_total * 2;
    RAISE NOTICE 'result=%', running_total;
END;
$$;
