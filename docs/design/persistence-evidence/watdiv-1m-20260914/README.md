# WatDiv 1M diagnostic checkpoint — 2026-09-14

**CONTROL-INVALID.** Root disk was 94% at initial admission and rose to 96%,
exceeding the peer protocol's 80% ceiling. Host swap was occupied/full, other
workloads were active, and the ten-minute thermal/frequency admission was not
performed. Host swap-in/out activity was observed during measurement; the disk
attempt and concurrency-four arm each sampled a temperature sensor at 86°C.
The completed checkpoint uses **tmpfs**, after an interrupted disk attempt. These are raw observations, unsuitable for ranking, a persistence
throughput claim, or an admission-green claim.

This measurement-only evidence accompanies the [persistence design review](https://github.com/scbrown/quipu/pull/248).

## Artifact and count reconciliation

The complete, pinned `watdiv.1M.generated.nt` is 152,195,750 bytes with SHA-256
`c158998c66e11b33bc56cf7fa3cbc9e69c1c36bf9bdd1bab447d8a64e2d8da75`.
Its 1,091,718 parsed triples contain 13,033 identical duplicate lines:
1,078,685 distinct source triples plus three ingest metadata facts produce
**1,078,688 live/history facts**. All are in the explicit benchmark graph.
`count-validation.json` records the independent line count and SQLite metadata
values, including the completion assertion and declared hash/count.

The generator was unseeded. The exact dataset and template archive were copied
out of disposable cache and rehashed before use; regenerating SF10 does not
reproduce this artifact. The 1M label is a scale name, not a 1,000,000 denominator.
This is the entire generated dataset, not a prefix cut from a larger archive.

The tmpfs ingest completed in **26.789 seconds**, 22 transactions, and produced a
**338,128,896-byte database**. This time describes a tmpfs diagnostic only. The
separate ext4 attempt was terminated after 150.386 seconds following observed
journal-commit waits and increasing disk pressure. It had committed 50,000 facts;
its partial state and exit `-15` remain in `disk-load.json`. The original 900-second
maximum was not reached. No query receipt uses that incomplete database.

## Process and query protocol

The checksum-verified official `quipu-ai-v0.6.0` Linux release was built with
`cargo build --release --locked --features full`. `provenance.json` links the
release workflow and records archive/binary hashes. Each server's `/version`
response independently reports commit `62e57bf05045825ffbee60f39933e6041bdf33ff`,
clean source and the full feature bundle. ONNX was compiled but no embedding
provider/model was configured; auto-embedding was disabled. The adjacent old
cached binary with unknown build features was not used.

Each workload ran in its own user scope with `CPUQuota=200%`, `MemoryMax=6G`, and
`MemorySwapMax=0`; raw cgroup controls verify these values. CPU affinity was not
fixed. A Python monitor shares that scope, so cgroup memory includes the monitor
and charged file cache. RSS/PSS values describe the workload PID only. No host
power, governor, swap, cache-flush, or service settings were changed. A local
quality gate overlapped part of the diagnostic, another source of contention.

Concurrency one and four use separate fresh server processes and SQLite backups
of the completed base store, all in tmpfs. Both use a read pool of four and a
private ephemeral loopback listener. The readiness baseline is read synchronously
from `/proc/PID` **after** receiving the complete successful `/version` response.
Baseline, warmup, measured requests, and immediate tail sample share one PID
within each arm. Startup samples in the continuous stream are not baselines.
There is no OS-cache-cold or long idle-tail claim.

All 20 top-level WatDiv v0.6 templates are included. The pinned template archive,
seed-zero instantiation manifest, concrete bindings, and exact executed queries
are recorded. Bindings are selected from observed IRIs in this complete dataset;
this diagnostic does not implement the full peer protocol's distribution or
thousands of warm repetitions. Each template gets one serial warmup, followed by
one homogeneous measured wave of one or four requests. The server timeout is
30 seconds and the client timeout 40 seconds. The row ceiling is 1,000,000;
truncation and failures remain explicit. Query order is lexical, as in the files.

Per-request receipts include wall time, result count, scalar-JSON row multiset
hash, and before/after RSS/PSS/HWM. Concurrent requests overlap: their memory
samples are aggregate process observations, not allocations attributable to one
request. Hashes compare the returned scalar JSON rows, not typed RDF terms.
No independent correctness oracle was run; zero-row responses and matching
hashes do not establish semantic correctness. There are no percentile or
performance-comparison claims.

## Observed receipts

| Concurrency | Requests (warm + measured) | HTTP 200 | HTTP 408 | Ready RSS / PSS (KiB) | Sampled peak RSS / PSS (KiB) |
| --- | --- | --- | --- | --- | --- |
| 1 | 40 | 34 | 6 | 15,808 / 11,303 | 506,848 / 502,343 |
| 4 | 100 | 81 | 19 | 15,992 / 11,446 | 1,611,788 / 821,736 |

C2, C3 and F3 timed out during warmup and measurement in both arms. S7's four
measured concurrent requests also timed out. Successful counts and scalar-row
hashes were consistent across all 17 templates with any successful result.
`verified-requests.json` contains all 140 per-request observations. These are
sampled process measurements under invalid controls; RSS includes file mappings
and must not be read as private heap size. The one-second ingest sample peaks
are lower bounds, not an exact process-exit resource-usage measurement.

Receipt checks passed for coverage, process identity, synchronous readiness
ordering, sequential arms, configured resource limits, no cgroup OOM kill,
no returned truncation, and successful-response consistency. All workload PIDs
were absent before cleanup. `cleanup.json` records **1,066,594,032 bytes** of
transient database files removed; no benchmark database is retained.

## Files and reproduction

- `provenance.json`, `count-validation.json`: pinned inputs, build and counts.
- `load.json`, `disk-load.json`: completed and interrupted ingest receipts.
- `c1.json`, `c4.json`, `summary.json`: query and process memory receipts.
- `*-samples.json.gz`: complete one-second process/cgroup/host sample streams.
- `host-preflight.json`: raw host resource, power-limit and throttle observations.
- `queries/`, `instantiation-manifest.json`: the exact query corpus.
- `measure.py`: portable harness; path settings are supplied through environment.

Set `CHECKPOINT_ROOT` to an owned tmpfs scratch directory, `QUIPU_BENCH_BIN` to
the extracted verified release directory, and `WATDIV_DATASET` to the preserved
pinned dataset. Copy `queries/` into the scratch directory and create
`load/.bobbin/config.toml` containing:

```toml
[quipu.embedding]
auto_embed = false
[quipu.search]
max_sparql_rows = 1000000
query_timeout_ms = 30000
[quipu.server]
read_pool_size = 4
```

Run each command sequentially in a distinct user scope with the same limits:

```sh
systemd-run --user --scope -p CPUQuota=200% -p MemoryMax=6G \
  -p MemorySwapMax=0 python3 measure.py ingest
systemd-run --user --scope -p CPUQuota=200% -p MemoryMax=6G \
  -p MemorySwapMax=0 python3 measure.py 1
systemd-run --user --scope -p CPUQuota=200% -p MemoryMax=6G \
  -p MemorySwapMax=0 python3 measure.py 4
```

The published harness replaces the executed copy's local paths with environment
inputs, formats the code, binds the thread-pool closure explicitly and narrows
exception handling; measurement operations are unchanged. Preserve raw receipts
before removing scratch databases. Local paths in published data use placeholders.
The large source artifact is retained outside the repository; transient database
copies are deleted after measurement under the host's capacity restriction.

The **10M, 100M, and 1B scales are unmeasured in this checkpoint**. An admitted,
disk-backed persistence measurement and the full peer protocol remain outstanding.
