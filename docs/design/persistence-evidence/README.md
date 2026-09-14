# Persistence review evidence

These are measurements and design inputs, not backend implementation changes.

- `graph-repeat.json.gz` and `from-repeat.json.gz`: complete request results,
  sampled process memory, phases and server version for the independent repeat.
- `prior-measurements.json`: phase extraction from the six-arm earlier experiment,
  with source gzip SHA-256 and per-response result digests. The suspect startup
  sample is explicitly excluded. Original source: `aegis-f7mxxu`.
- `watdiv-completed.json`: the two completed 10M ingest records, with the local
  binary pathname removed. These are ingestion records, not query/RSS results.

Repeat database: 1,008,000 facts, 1,008,043 terms, 48,000 vectors.
SQLite backup SHA-256:
`dbaaf060a2b3404bf8632085ff17d119c8b84bc24ee578b55205228068cbf48a`.
Copied server binary SHA-256:
`fd3061f75ec408ee2c38a6ac5d946f32b850dc1504bd6a3fe03d6c6fe39f42d2`.
Server reports version 0.5.1, clean SHA
`692d700a3c67ea7a9b5a30bbe89333bbc7f6aeae`, full features, no LanceDB.
The compiler profile was not recovered from the installed artifact.
Runtime in the repeat: ONNX 1.23; no rebuild or production restart.

## Method

The existing `aegis-f7mxxu` stdlib harness was copied without behavioral changes.
For each arm it creates a SQLite backup, starts the copied binary on a private
loopback listener and sets four readers with embedding-on-write disabled. Model
and tokenizer are the existing MiniLM assets; no new model was downloaded.
A user scope caps CPU at two cores and memory at 6 GiB. No production traffic,
mutations or global page-cache flushes are used. The original database and any
WAL are read through SQLite backup, never copied as a bare main file.

Each phase executes three rounds: one request per serial round, four simultaneous
requests per concurrent round. Query serial, query concurrent, search serial,
search concurrent gives 30 requests. Two seconds elapse before each phase sample;
a five-second quiet tail precedes process teardown. The sampler runs every 0.5s.
The startup sample can precede readiness; it is not a precise cold-memory probe.
Use a synchronous post-readiness sample in subsequent harness revisions.

```sparql
SELECT ?s ?p ?o WHERE {
  GRAPH <http://ex.org/g> { ?s ?p ?o }
} LIMIT 200
```

The paired arm replaces only the query with:

```sparql
SELECT ?s ?p ?o FROM <http://ex.org/g>
WHERE { ?s ?p ?o } LIMIT 200
```

Search payload in both arms:
`{"query":"varied entity knowledge","limit":5}`.
The fixture has 960,000 facts in that named graph and 48,000 in ROOT. Its
relationship to the production-sized workload is cardinality only; it is not
an anonymized production corpus and not a valid WatDiv checkpoint.

The results establish all 60 HTTP responses are 200 and all 30 paired result
objects are equal. Query counts are 200 and search counts are five. A result
comparison here does not authorize rewriting arbitrary GRAPH queries to FROM.
No claim is made that either process reached a day-long memory plateau.

The database is not included. The hash identifies the retained artifact for
internal reruns; a stranger can audit the receipts but cannot reproduce this
exact run without obtaining that fixture. Publishing a deterministic synthetic
fixture generator and pinning the build/profile/model hashes is an explicit
follow-up before treating these numbers as a public performance benchmark.
