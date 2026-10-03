#!/bin/sh
# Deterministic protocol fixture; never installed or used for real evaluation.
request=$(cat)
verdict=satisfied
violations='[]'
errors='[]'
code=0
case "$request" in
  *'"source":"timeout"'*) exec sleep 30 ;;
  *'"source":"invalid-json"'*) printf 'invalid'; exit 0 ;;
  *'"source":"unknown"'*) verdict=unknown; errors='["parse error"]'; code=2 ;;
  *'"source":"not_applicable"'*) verdict=not_applicable ;;
  *'"source":"unsatisfied"'*) verdict=unsatisfied; violations='["missing ticket"]'; code=1 ;;
  *'"source":"wrong-status"'*) verdict=unsatisfied; violations='["missing ticket"]' ;;
  *'"source":"inconsistent"'*) violations='["missing ticket"]' ;;
esac
printf '{"schema_version":1,"rule":"test-policy","path":"src/x.rs","verdict":"%s","violations":%s,"errors":%s}' "$verdict" "$violations" "$errors"
exit "$code"
