# Bounded private query capture demo

A synthetic 3,995-byte query is selected by its exact digest and byte length.
With capture disabled there is no worker or artifact. Enabling the same private
fixture produces one 4,411-byte artifact, excludes an unknown input field, and
refuses a restart using the same nonce. The byte ceiling stays 16 KiB.

This demonstrates a configuration toggle within the candidate, not two installed
server revisions. Base `b7068fdf6772d88e0f45dcdd63f156ef86abd1e4` has no capture
configuration. The demonstrated product/test sources match commit
`e6f806f7` (the binary was built before that commit and embeds the base revision
with `git_dirty=true`). No network request or production store is used.

Download [the terminal output](query-capture.output) and
[its timing file](query-capture.timing), then replay with util-linux:

```sh
scriptreplay --log-out query-capture.output --log-timing query-capture.timing
```

Replay was checked. CRLF was normalized to LF; the original timing interval
was retained and its byte count adjusted for removed carriage returns. This is a terminal recording with a readable transcript,
not an inline video. The small recording is retained beside this caption in the
repository. Reproduce the private demonstration from the reviewed source:

```sh
just test --bin quipu-server --features full query_capture_private_demo -- --ignored --nocapture
```

Seven separate controls cover disabled behavior, match refusals, absolute and
monotonic expiry, one-shot/restart refusal, serialization quota, private-file
checks and unchanged query responses. The demonstration and controls passed the
HOME/resource guard. These are private-fixture results; source deployment does
not authorize arming a production capture.
