-- Самостоятельная учебная предметная модель для отладчика.
-- Запускать один раз в новой выделенной базе.
BEGIN;
CREATE EXTENSION IF NOT EXISTS pldbgapi;
CREATE SCHEMA sales;
CREATE SCHEMA billing;
CREATE SCHEMA audit;

CREATE TABLE sales.customers (
    id integer PRIMARY KEY,
    name text NOT NULL
);
CREATE TABLE sales.orders (
    id integer PRIMARY KEY,
    order_number text NOT NULL UNIQUE,
    customer_id integer NOT NULL REFERENCES sales.customers(id),
    status text NOT NULL DEFAULT 'new' CHECK (status IN ('new', 'paid'))
);
CREATE TABLE sales.order_items (
    order_id integer NOT NULL REFERENCES sales.orders(id),
    product text NOT NULL,
    quantity integer NOT NULL CHECK (quantity > 0),
    unit_cents integer NOT NULL CHECK (unit_cents > 0),
    PRIMARY KEY (order_id, product)
);
CREATE TABLE billing.payments (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    order_id integer NOT NULL REFERENCES sales.orders(id),
    amount_cents integer NOT NULL CHECK (amount_cents > 0),
    reference text NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE audit.events (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    order_id integer NOT NULL REFERENCES sales.orders(id),
    description text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

INSERT INTO sales.customers VALUES (1, 'Анна'), (2, 'Иван');
INSERT INTO sales.orders VALUES
    (1001, 'ORDER-1001', 1, 'new'),
    (1002, 'ORDER-1002', 2, 'new'),
    (1003, 'ORDER-1003', 1, 'paid');
INSERT INTO sales.order_items VALUES
    (1001, 'Кабель', 2, 1990), (1001, 'Адаптер', 1, 2010),
    (1002, 'Кабель', 1, 1990), (1003, 'Адаптер', 1, 2010);
INSERT INTO billing.payments (order_id, amount_cents, reference)
VALUES (1003, 2010, 'EXISTING-PAYMENT');
INSERT INTO audit.events (order_id, description) VALUES (1003, 'Заказ оплачен');

CREATE FUNCTION sales.order_total(input_order_id integer)
RETURNS integer LANGUAGE plpgsql AS $$
DECLARE
    total_cents integer;
BEGIN
    SELECT sum(quantity * unit_cents)::integer
    INTO total_cents
    FROM sales.order_items
    WHERE order_id = input_order_id;
    IF total_cents IS NULL THEN
        RAISE EXCEPTION 'Заказ % не найден или не содержит позиций', input_order_id
            USING ERRCODE = 'P0002';
    END IF;
    RETURN total_cents;
END;
$$;
COMMENT ON FUNCTION sales.order_total(integer) IS 'Сумма заказа в копейках по его идентификатору';

CREATE FUNCTION sales.order_total(input_order_number text)
RETURNS integer LANGUAGE plpgsql AS $$
DECLARE
    selected_id integer;
BEGIN
    SELECT id INTO STRICT selected_id
    FROM sales.orders WHERE order_number = input_order_number;
    RETURN sales.order_total(selected_id);
END;
$$;
COMMENT ON FUNCTION sales.order_total(text) IS 'Перегрузка расчёта суммы по номеру заказа';

CREATE PROCEDURE billing.pay_order(
    IN input_order_id integer,
    IN input_amount_cents integer,
    IN input_reference text,
    OUT receipt text
)
LANGUAGE plpgsql AS $$
DECLARE
    current_status text;
    expected_cents integer;
BEGIN
    SELECT status INTO STRICT current_status
    FROM sales.orders WHERE id = input_order_id FOR UPDATE;
    IF current_status <> 'new' THEN
        RAISE EXCEPTION 'Заказ % уже оплачен', input_order_id USING ERRCODE = '22023';
    END IF;
    expected_cents := sales.order_total(input_order_id);
    IF input_amount_cents IS DISTINCT FROM expected_cents THEN
        RAISE EXCEPTION 'Ожидалось %, получено %', expected_cents, input_amount_cents
            USING ERRCODE = '22023';
    END IF;
    UPDATE sales.orders SET status = 'paid' WHERE id = input_order_id;
    INSERT INTO billing.payments (order_id, amount_cents, reference)
    VALUES (input_order_id, input_amount_cents, input_reference);
    INSERT INTO audit.events (order_id, description)
    VALUES (input_order_id, 'Заказ оплачен');
    receipt := 'PAY-' || input_order_id::text;
END;
$$;
COMMENT ON PROCEDURE billing.pay_order(integer, integer, text)
IS 'Оплата заказа с проверкой суммы, записью платежа и аудитом';
COMMIT;
