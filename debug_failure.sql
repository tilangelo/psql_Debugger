-- Намеренная ошибка для проверки жизненного цикла сеанса отладки.
CREATE OR REPLACE FUNCTION public.debug_failure(input_value integer)
RETURNS integer
LANGUAGE plpgsql
AS $$
DECLARE
    running_total integer := input_value;
BEGIN
    running_total := running_total + 1;
    RAISE EXCEPTION 'intentional debug failure: running_total=%', running_total;
END;
$$;
