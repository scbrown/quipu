# Committed feed replay demo

Isolated in-memory store and direct HTTP handler fixture, not an installed service.
Base: 14a24ebbdaf39cce121d1979e8d59bb677c883ea. Demonstrated source:
2ad4a8cee48b208d3386adf034dd3476a6835979; the following demo-only commit leaves runtime unchanged. A later lint correction
borrows headers and uses equivalent guards/let-else; the replay contract is unchanged.

A named-graph write leaves the legacy ROOT event offset at zero. Two replay
requests return HTTP 200 and the same transaction 2, containing only the target
graph. Delivery leaves the consumer ACK at zero. This proves replay and graph
scope; it does not measure production latency, RAM, or activation.

Repeat with `cargo test --bin quipu-server --no-default-features --features server,shacl,onnx named_graph_changes_replay_without_event_offset_ack -- --nocapture`.

Download [compressed output](committed-feed.txt.gz) and [timing](committed-feed.time), decompress with `gzip -dk committed-feed.txt.gz`, then run
`scriptreplay --log-timing committed-feed.time --log-out committed-feed.txt`.
Playback was verified locally. These small sanitized artifacts are retained in Git.
