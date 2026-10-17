# Accepted snapshot cancellation

This replay records the same 77-byte, in-memory HTTP route fixture on base
`be9f52b40b5801450a1c323cb8eecb92b3bf96ca` and candidate source
`93c7e7c0aec2961251e3e5681437eff4c2f0b4ac`. The next commit adds this
recording and caption only. It exercises source binaries, with no network or
installed service involved. The test holds the writer while accepting promotion,
requests cancellation, releases the writer, then reads the terminal result.

Watch `cancel_http`, `terminal` and `active_facts`: the base returns 404 and later
commits one fact; the candidate returns 200 and reaches terminal cancellation
with zero facts committed. The base's failed assertion is expected, and the
recording checks that exit status separately from the candidate's success.

Repeat the fixture on either revision (copy the test-only probe onto the base):

```sh
just test --features full --bin quipu-server accepted_promotion_stop_contract -- --nocapture
```

The shared fixture is
[`snapshot_cancel_contract_probe.rs`](../../../../src/server/snapshot_cancel_contract_probe.rs).
The recording is a replayable terminal output/timing pair, not a video:

```sh
gzip -dc output.log.gz > /tmp/snapshot-cancellation-output.log
scriptreplay --log-out /tmp/snapshot-cancellation-output.log --log-timing timing.log
```

This queued-writer demonstration proves the stop contract. Separate tests cover
rollback after actual SQL writes, expiry without heartbeat, commit/cancel races,
authenticated ownership and explicit resumption after terminal cancellation.
Neither the recording nor a passing test establishes a universal memory bound
for arbitrary production shapes, validation or deferred embedding.
