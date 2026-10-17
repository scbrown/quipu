# Static HTTP profile demonstration

The same synthetic candidate/manifest input is refused as an unknown class with
only the base code-entities vocabulary, then yields one unique candidate pair
when the opt-in profile is explicitly loaded in a private in-memory store. The
printed row carries repository, revision, source digest and both DTO manifest
references/digests. The remaining controls keep collisions, stale identities,
missing evidence and uncertainty from resolving. A ROOT collision is excluded.

Base engine/source: `f6739976ba3433ee8bb0530f55aa3fa209351e88`.
Demonstrated profile/test source: `254e1b0766535cb27fb9b2d02a4654663d6c8095`.
Subsequent recording-only changes preserve those four source files byte-for-byte.
These are fabricated fixture repositories, revisions and source bytes; no real
extractor/Git correspondence, production enrollment or runtime dependency is
claimed. Mapper verification and separate schema approval are still required.

Run the private reproducer with `just test --test static_http_profile -- --nocapture`.
Six tests pass, with zero ignored or filtered tests. The separate library run
passed 2,438 tests with 13 existing ignores. These local results do not replace
hosted CI, independent source review or the normal landing/deployment lane.

The [output recording](demo.output) and [timing file](demo.timing) replay with:

```sh
scriptreplay --log-out demo.output --log-timing demo.timing
```

Playback was verified with util-linux `scriptreplay`; download both files first.
This is a terminal recording, not an inline video. CRLF was normalized to LF and
chunk byte counts adjusted while retaining each original measured delay. The
original raw pair is retained with the task's private evidence. Actual output
and success/refusal outcomes were preserved. The guarded recording run left all
158 watched paths and five UID resources unchanged; this bounds the observation,
rather than certifying general hermeticity.
