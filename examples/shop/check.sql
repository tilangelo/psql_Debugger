-- Выполнить до и после сеанса GUI для сравнения транзакционных данных.
SELECT id, order_number, status FROM sales.orders ORDER BY id;
SELECT order_id, amount_cents, reference FROM billing.payments ORDER BY id;
SELECT order_id, description FROM audit.events ORDER BY id;
SELECT routine_schema, routine_name, identity_arguments, routine_type, description
FROM orm.functions_view ORDER BY routine_schema, routine_name, identity_arguments;
