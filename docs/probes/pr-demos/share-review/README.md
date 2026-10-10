# Share review demonstration

Replayable terminal output and timing pair, not an inline video. On the same local datatype-violation fixtures, installed clean dfc5 rejects the unsupported report flag (exit 1); candidate source 8659f951 renders facts and one introduced SHACL violation, then refuses the gate (exit 3). This demonstrates candidate CLI behavior; it does not demonstrate deployment.

Input: tests/fixtures/share-review/base and introduced. Reproduce on each binary with `quipu share diff tests/fixtures/share-review/base tests/fixtures/share-review/introduced --report --fail-on-introduced`. Replay: `gzip -dc output.log.gz > /tmp/share-review-output.log` then `scriptreplay --log-out /tmp/share-review-output.log --log-timing timing.log`. Recorder: util-linux script; asciinema is unavailable. Small sanitized logs are retained alongside this caption in Git. Candidate implementation is unchanged by this recording-only commit.

Raw output is gzip-compressed to preserve every terminal byte under whitespace hooks; decompression was verified byte for byte.
