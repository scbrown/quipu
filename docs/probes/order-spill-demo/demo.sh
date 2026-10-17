#!/usr/bin/env bash
# Library-level before/after proof; no service or database connection.
set -euo pipefail
: "${BEFORE_TEST_BIN:?provide the control lib-test executable}"
: "${AFTER_TEST_BIN:?provide the candidate lib-test executable}"
case_name=sparql::order::tests::complete_order_exceeds_key_slots_without_raising_retained_budget
echo 'BEFORE: complete ORDER BY over 70,000 reversed integer rows'
set +e
"$BEFORE_TEST_BIN" "$case_name" --exact --nocapture
control_rc=$?
set -e
test "$control_rc" = 101
echo 'AFTER: same 70,000 rows; asserts every position and complete count'
"$AFTER_TEST_BIN" "$case_name" --exact --nocapture
echo 'AFTER: LIMIT/OFFSET, mixed keys, nested sort, quota and codec controls'
"$AFTER_TEST_BIN" sparql::order:: --nocapture --test-threads=2
