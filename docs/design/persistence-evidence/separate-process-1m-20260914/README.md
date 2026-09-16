# Separate-process persistent-engine memory measurement

This experiment compares Quipu's SQLite-backed engine and RocksDB-backed
Oxigraph in independent processes. It is measurement only: no backend switch,
production configuration change, or performance ranking is proposed.

The pinned, complete WatDiv artifact contains **1,091,718 input triples** in
152,195,750 bytes, including 13,033 duplicate lines. Its SHA-256 is
`c158998c66e11b33bc56cf7fa3cbc9e69c1c36bf9bdd1bab447d8a64e2d8da75`.
Both engines must persist **1,078,685 unique data triples** in the same named
graph. Quipu additionally writes three declared-load completion facts. The
fixture was reused without regeneration. The 20 concrete queries come from the
[earlier full-fixture measurement](../watdiv-1m-20260914/README.md), rather than
from an archive of templates. Query hashes are recorded in `protocol.json`.

## Instrument and controls

`engine.rs` is a measurement worker compiled against revision and dependencies
recorded in `provenance.json`, using `--release --locked --features full,oxicompare`.
The `oxicompare` feature explicitly enables RocksDB. Each process constructs only
its selected engine; the Oxigraph arm uses `Store::open`, never `Store::new`.
The same executable in separate address spaces holds compiler, dependency
versions, build profile, and allocator selection constant. Rust uses its default
system allocator; RocksDB uses the linked C++ runtime and libc allocator.

Each arm keeps one PID from synchronous empty readiness through ingest,
synchronous loaded readiness, warm-up, three rounds at concurrency one, three
rounds at concurrency four, and a five-second idle tail. Four persistent query
workers exist after ingest. Quipu has four read-only SQLite connections plus its
original writer; Oxigraph clones handles sharing its persistent Store. Library
configuration defaults remain in effect. No embedding provider or model is
loaded, and a separate SQLite count must confirm zero vectors.

Both processes receive a two-core CPU quota, a 6 GiB cgroup memory limit, and no
process swap. The orchestrator reads back and asserts these settings. RSS, PSS,
anonymous/file splits, and process-lifetime high-water RSS come from `/proc`;
ready samples are synchronous after the worker's readiness response. A 100 ms
sampler captures phase activity. Its maxima are sampled peaks, not continuous
peak-PSS guarantees. `VmHWM` is cumulative across the process lifetime and cannot
be interpreted as a fresh high-water counter for each phase. Cgroup figures
include the launcher and charged filesystem cache and are recorded separately.

The worker fully consumes every SELECT result, materializes sorted canonical
binding strings, and hashes the result multiset, preserving duplicates. This
small common adapter is part of both measurements. Every completed repeat and concurrent
request must agree, and completed common queries must agree across engines.
Errors and deadline expirations remain explicit did-not-finish records, never
zero-row successes; queries are not removed from later phases. Both engines have
a 30-second query budget: Quipu uses its native deadline and Oxigraph receives
a cancellation token. An identical temporary watchdog thread is present in
both adapters. Quipu retains its default million-row intermediate-join cap;
Oxigraph has no matched intermediate-row cap, which limits comparison of failures.
It is stronger than a count-only check, but it is not a complete SPARQL
conformance suite or a production HTTP workload.

## Interpretation boundaries

Index structures, cache policy, transaction history, native allocations, and
connection architecture are engine-specific. They are not equalized. Quipu
loads in transactions of 50,000 input triples and retains its writer; Oxigraph
uses its transactional `load_from_reader` and flushes before readiness. No store
reopen or allocator trim erases ingest allocations. Ready-after-load therefore
measures the same process after its own ingest, not a fresh server opening an
existing store. The four-reader state persists across both concurrency phases,
so concurrency four runs after concurrency one has warmed reader zero.

The stores use separate directories on the same local ext4 filesystem. The
Quipu arm runs first, followed by Oxigraph. OS caches are not flushed, the input
is hashed before loading, and host co-tenants remain active. File-backed PSS can
be influenced by other processes mapping the same libraries. Raw host and
cgroup receipts expose these conditions.

Memory is not thermally gated. Timing fields are **CONTROL-INVALID for latency
ranking**: no thermal admission was obtained, arms run sequentially, and the
shared host has occupied swap. Disk admission is evaluated from observed
filesystem usage, not the earlier host-capacity warning. No 10M, 100M, or 1B
memory results are measured here. A short idle tail establishes neither a leak
nor long-term stability.

## Reproduction

Run `just build` in this directory. It temporarily copies the worker into Cargo's
examples directory and removes that copy after building. Use the shared Cargo
target directory on a constrained host; the worker is emitted as
`release/examples/memory_engine_probe` beneath that target.

Run `measure.py --help`, then supply the worker, pinned dataset, an unused output
directory, a scratch directory on the intended filesystem, the existing query
directory, and the dataset hash. The host needs Linux cgroup v2 and a working
user systemd manager. The script refuses a different input hash, verifies
counts and limits, and removes each temporary store after recording its receipt,
including on a failed measurement. Preserve receipts before removing scratch.
