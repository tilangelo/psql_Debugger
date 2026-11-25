\set ON_ERROR_STOP on
\pset pager off
\if :{?target}
\else
\set target public.debug_smoke(integer)
\endif
SELECT pldbg_create_listener() AS listener \gset
\echo LISTENER :listener
SELECT pldbg_set_global_breakpoint(:listener, :'target'::regprocedure, NULL, NULL) AS breakpoint_set;
SELECT pldbg_wait_for_target(:listener) AS target_pid \gset
\echo TARGET_PID :target_pid
SELECT * FROM pldbg_wait_for_breakpoint(:listener);
SELECT * FROM pldbg_get_stack(:listener);
SELECT name, lineNumber, value FROM pldbg_get_variables(:listener);
SELECT * FROM pldbg_step_over(:listener);
SELECT name, lineNumber, value FROM pldbg_get_variables(:listener);
\set ON_ERROR_STOP off
SELECT * FROM pldbg_continue(:listener);
