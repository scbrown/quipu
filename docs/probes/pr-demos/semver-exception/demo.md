# Scoped semver exception and reversal

Hosted before/after timing demonstration is unavailable until the next normal
code push after landing. The baseline is the actual Release-plz job in
[run38893215505](https://github.com/scbrown/quipu/actions/runs/38893215505),
1,321 seconds; two more code pushes measured1,325 and1,355 seconds. The target
is under about300 seconds. This remains an open acceptance check.

The small [recording](output.log) and [timing](timing.log) show the **actual
policy entrypoint** against private TOML fixtures: private0.x accepted;
publishing or1.x refused; re-enabling the check permits that transition. This
is a local control demo, not a claim about hosted semver-check duration.

Base release policy is commit459b598602d9ff3bbd69fb9c41382ef35faf7d40; it has no
such exception. The candidate adds the scoped override and the policy guard.
To repeat from the checkout:

```bash
python3 docs/probes/pr-demos/semver-exception/run.py
scriptreplay --log-timing docs/probes/pr-demos/semver-exception/timing.log --log-out docs/probes/pr-demos/semver-exception/output.log
```

Util-linux scriptreplay playback was checked. This is an output-only terminal
recording, also readable as a transcript, retained here for review. It makes no
network calls, publishes nothing, and writes only temporary fixture files.
