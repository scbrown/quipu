# Complete native sorts with bounded retained keys

The same library fixture sorts 70,000 reversed integer rows. The control
refuses at the existing 65,536-key-entry guard; the candidate returns all
70,000 rows and asserts every sorted position. The recording also runs
LIMIT/OFFSET stable-tie, mixed/computed-key fallback, nested-sort, deadline,
encoded-record, lossless-codec, and spill-quota controls.

This is a synthetic library demonstration. It does not measure a production
export or bound the upstream materialized relation's memory. The key-memory
limits are unchanged; native large sorts use private bounded spill runs.

To replay, build the library test executable in two isolated checkouts with
`just --command cargo test --lib sparql::order --no-run`. Add the same
70,000-row regression test to the unchanged control implementation first.
Keep copies of the resulting control and candidate executables, then run:

```bash
BEFORE_TEST_BIN=/path/to/control-test AFTER_TEST_BIN=/path/to/candidate-test \
  script -q -e -c 'bash docs/probes/order-spill-demo/demo.sh' recording.log
```

The control's expected test failure is part of the demonstration. Any other
exit status or candidate failure stops it. Neither command contacts a server.
