"""Проверка функций, процедуры оплаты и отката в отдельной базе заказов."""
import json
import os
import re
import subprocess
import time
import urllib.request

base = os.environ["PGDEBUG_WEB_TEST_URL"]
password = os.environ["PGDEBUG_GUI_TEST_PASSWORD"]
port = os.environ["PGDEBUG_GUI_TEST_PORT"]
database = "gui_shop_test"
page = urllib.request.urlopen(base, timeout=10).read().decode()
token = re.search(r'name="pg-debugger-token" content="([a-f0-9]+)"', page).group(1)


def post(path, body):
    request = urllib.request.Request(
        base + path,
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json", "x-pg-debugger-token": token},
    )
    with urllib.request.urlopen(request, timeout=15) as response:
        return json.load(response)


def sql(query):
    return subprocess.check_output(
        [os.environ["PGDEBUG_TEST_PSQL"], "-h", "127.0.0.1", "-p", port,
         "-U", "debugger_test", "-d", database, "-XAt", "-v", "ON_ERROR_STOP=1", "-c", query],
        env={**os.environ, "PGPASSWORD": password}, text=True, timeout=10,
    ).strip()


connection = post("/api/connect", {
    "host": "127.0.0.1", "port": port, "database": database,
    "user": "debugger_test", "password": password,
})
identity = {"connection_id": connection["connection_id"], "database": database}
assert {"sales", "billing", "audit", "orm"}.issubset(connection["schemas"])
assert sql("SELECT count(*) FROM orm.functions_view WHERE routine_schema IN ('sales','billing')") == "3"


def browse(schema, name, argument_type=None):
    selection = {**identity, "schema": schema}
    routines = post("/api/catalog", selection)["routines"]
    candidates = [routine for routine in routines if routine["name"] == name]
    if argument_type:
        assert len(candidates) == 2, candidates
        candidates = [routine for routine in candidates if argument_type in routine["signature"]]
    assert len(candidates) == 1, candidates
    return post("/api/catalog", {**selection, "routine_oid": candidates[0]["oid"]})["selected"]


def wait(state, phase):
    deadline = time.monotonic() + 15
    while state["phase"] != phase:
        assert time.monotonic() < deadline and state["phase"] != "finished", state
        time.sleep(0.1)
        state = post("/api/debug/poll", {"session_id": state["session_id"]})
    return state


def start(schema, routine, values):
    return wait(post("/api/debug/start", {
        **identity, "schema": schema, "routine_oid": routine["oid"],
        "expected_definition": routine["definition"], "values": values, "breakpoints": [],
    }), "paused")


def command(state, action, phase):
    return wait(post("/api/debug/command", {
        "session_id": state["session_id"], "revision": state["revision"],
        "action": {"command": action},
    }), phase)


def finish(state, outcome, text):
    result = command(state, "continue", "finished")["outcome"]
    assert result["call"] == outcome and result["rollback"] == "confirmed", result
    assert text in result["text"], result


def snapshot_order():
    return sql("""
        SELECT status,
               (SELECT count(*) FROM billing.payments WHERE order_id = orders.id),
               (SELECT count(*) FROM audit.events WHERE order_id = orders.id)
        FROM sales.orders WHERE id = 1001
    """)


total = browse("sales", "order_total", "integer")
finish(start("sales", total, ["1001"]), "succeeded", "5990")
by_number = browse("sales", "order_total", "text")
finish(start("sales", by_number, ["ORDER-1001"]), "succeeded", "5990")
payment = browse("billing", "pay_order")
assert snapshot_order() == "new|0|0"
state = start("billing", payment, ["1001", "5990", "WEB-TEST", None])
for _ in range(5):
    line = payment["source"].splitlines()[state["position"]["line"] - 1]
    if "expected_cents := sales.order_total" in line:
        break
    state = command(state, "next", "paused")
else:
    raise AssertionError(("Nested call not reached", state["position"]))
state = command(state, "step_into", "paused")
assert state["position"]["oid"] == total["oid"], state["position"]
state = command(state, "step_out", "paused")
assert state["position"]["oid"] == payment["oid"]
finish(state, "succeeded", "PAY-1001")
assert snapshot_order() == "new|0|0"
finish(start("billing", payment, ["1001", "1", "WEB-TEST-INVALID", None]), "failed", "22023")
assert snapshot_order() == "new|0|0"
post("/api/disconnect", {})
print("HTTP shop: overloads return 5990, nested call enters order_total, payment returns PAY-1001, invalid amount returns 22023, status/payment/audit rollback confirmed.")

