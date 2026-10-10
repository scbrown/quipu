# Receiver review lifecycle

The same private unknown-type share imports into quarantine on both binaries.
Before: clean installed ee174b23476be30015e23a1f35c1c5c681dc3aff cannot list
pending reviews (`import review pending`, exit 1). After: native candidate
c0a614cdbe267be7c64b39650c813bea48d61b97 returns one bounded pending review
(exit 0); named rejection blocks re-import (exit 1), and explicit reopening
allows re-import (exit 0). Neither arm adopts bundled shapes.

This uses two temporary receiver databases and actual native CLI calls.
The candidate binary reports its clean source SHA. Later caption-only commits
preserve those source bytes. It proves source behavior, with no production
import, timer, outbound delivery or feature activation.

Recording: [output.log.gz](output.log.gz) and [playback.timing](playback.timing).
Download both and replay with util-linux:

```sh
gzip -dc output.log.gz > /tmp/import-review-output.log
scriptreplay --log-out /tmp/import-review-output.log --log-timing playback.timing
```

Reproduce with clean before and candidate binaries:

```sh
python3 reproduce.py /path/to/before/quipu /path/to/candidate/quipu
```

[reproduce.py](reproduce.py) creates the one-triple fixture, verifies declared
count/hash, exports the share, and checks both actual receiver CLI results.
Age timestamps and generated share IDs vary with the recording. The small
sanitized recording and reproduction source are retained in this repository.
