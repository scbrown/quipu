# Separate-process memory comparison — quipu (SQLite) vs Oxigraph (RocksDB), run 4

WatDiv 1M, one process per engine, measured 2026-09-16. Harness in the parent directory
(authored by gennaro); this directory is the run-4 evidence.

**Scope, from the summary's own `scope` field:** *separate-process memory observations; not a
latency or overall engine ranking.* Timing figures below are recorded for completeness and are
**not admissible as a comparison** — the host was shared during the run.

## Why separate processes

A same-process peak RSS cannot attribute memory to one engine. Each arm runs in its own process
with its own data directory and identical CPU/memory limits, and memory is sampled synchronously at
named phases against a single PID per arm.

## Correctness control — run first

| | |
|---|---|
| common queries completed by both | 17 |
| result mismatches | **0** (by result count *and* result hash) |
| quipu records loaded | 1,078,688 live facts |
| Oxigraph records loaded | 1,078,685 quads |

Both engines return identical result multisets on every shared query. Without this, comparing their
memory would be comparing two different computations.

## Memory by phase (MiB)

| phase | quipu RSS | Oxigraph RSS | quipu Anon | Oxi Anon | quipu File | Oxi File |
|---|---|---|---|---|---|---|
| ready_empty  | 17.0 | 19.7 | 1.8 | 5.5 | 15.2 | 14.3 |
| ready_loaded | **57.6** | **1013.6** | 41.4 | 998.8 | 16.2 | 14.8 |
| c4_after | **1603.1** | **1077.7** | 569.0 | 1061.5 | 1034.1 | 16.2 |
| peak (VmHWM) | 1603.1 | **1724.7** | — | — | — | — |

Three true statements that point in different directions:

1. **At load, quipu is 17.6x leaner** — 57.6 MiB vs 1013.6 MiB. Oxigraph materialises about a
   gigabyte immediately; quipu does not.
2. **Under four concurrent readers, quipu grows 27.8x and Oxigraph grows 1.06x.** What quipu defers
   at load, it pays for under concurrency.
3. **Final RSS favours Oxigraph by 1.49x; peak RSS favours quipu by 1.08x** — Oxigraph peaks during
   load, quipu peaks at the end. Either number alone lets you declare a winner.

## ⚠️ Read RssAnon, not RSS — the ranking reverses

quipu's growth is **file-backed** (RssFile 16.2 -> 1034.1 MiB) and Oxigraph's is **anonymous**
(RssAnon 998.8 -> 1061.5 MiB, RssFile flat at ~15 MiB). File-backed pages here are SQLite mappings
and are reclaimable under memory pressure; anonymous pages are not.

    RssAnon at c4_after:   quipu 569.0 MiB   vs   Oxigraph 1061.5 MiB   ->  quipu 0.54x

**On the non-reclaimable component, quipu is leaner at every phase and ends at roughly half.** On
raw RSS it ends 1.49x higher. Same run, opposite conclusions, differing only in which quantity is
quoted — so this directory publishes the phase table with the Anon/File split rather than a single
RSS ranking.

## Query completion — a real, systematic gap

| | successful | did-not-finish |
|---|---|---|
| quipu | 272 / 320 | **48** — C2, C3, F3, each failing 16 of 16 attempts |
| Oxigraph | 320 / 320 | 0 |

quipu does not fail these three *intermittently under load*; it fails them on **every** attempt.
That is a capability difference at this scale, not contention.

## Recorded but NOT admissible as a comparison

| | quipu | Oxigraph |
|---|---|---|
| ingest wall | 101.3 s | 20.1 s |
| store on disk | 442 MB, 3 files | 1.06 GB, 18 files |

The host was shared during the run, so these are not engine-versus-engine results. They are kept
because the receipts contain them and omitting them would be selective.

## Re-deriving this

`measure.py` runs one arm per process; `summarize.py` produces `summary.json` from a receipts
directory. Each arm's `cleanup.json` records `temporary_store_removed: true`, and the run's store
directory is empty afterwards — verified for both arms.

## Limitations

- One dataset (WatDiv 1M), one machine, one run per engine.
- Concurrency tested at 1 and 4 readers only.
- Timing is not comparable (above).
- Store-device effects are **not** controlled here beyond both arms sharing one device. A separate
  measurement found ingest throughput varying by more than an order of magnitude with the store's
  device under I/O contention, so any future timing arm must pin and state the device.
