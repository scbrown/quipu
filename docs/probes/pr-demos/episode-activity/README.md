# Episode activity label floor

This isolated schema demonstration uses the installed native validator, with no
server or live-store writes. The same five inputs run against base
`f6739976ba3433ee8bb0530f55aa3fa209351e88` and the candidate provenance shapes
in this commit. A label-only legacy activity and an activity with two historical
body comments remain valid. Missing, IRI and integer labels change from accepted
to refused, each with one violation.

The [output recording](output.log) doubles as a readable transcript. Download it
with [timing.log](timing.log) and replay using util-linux:

```sh
scriptreplay --log-out output.log --log-timing timing.log
```

Replay was checked with `scriptreplay`. These files are retained with the source;
no inline video playback or installed-server acceptance is claimed.

For the corresponding native library controls on this revision:

```sh
just test --features shacl --test episode_activity_shapes
```

The tests additionally prove that loading the shipped shape publishes
`prov:Activity` in the receiver vocabulary. Runtime shape delivery is a separate
gate: this source change does not load shapes into an installed server.
