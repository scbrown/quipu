# Endpoint pointer SERVICE acceptance

Before: Quipu 0.14.1 at `0cb06945` cannot use the endpoint declaration from
federation config; the CLI reports no configured remotes and sends no request.
After: the candidate verifies the local pendant and asks the real
Tambo/Caboodle endpoint decision CLI before every use. Exact consent returns one
actual SPARQL `SERVICE` row, with no credentials and no imported ROOT facts.
Foreign, pack-only and revoked consent cause no network request; URL neighbours,
redirects, forged declarations and malformed policy refuse. HTTP failures remain
errors. All requests and stores use temporary fixtures, with no live endpoints
or trust configuration changed. This is source acceptance, not installed rollout.

Build the candidate and fixture:

```bash
cargo build --features remote --bin quipu --example endpoint_pointer_fixture
python3 docs/design/endpoint-pointer-demo/demo.py /path/to/quipu /path/to/endpoint_pointer_fixture /path/to/caboodle --baseline /path/to/previous-quipu
```

The trust CLI must include Tambo PR168's `check-endpoint` command. The demo copies
executables into its temporary directory; it makes no symlinks to live tools.
It starts a loopback endpoint and closes it in `finally`.

`transcript.txt` is readable output; `output.gz` is the terminal
recording (command-header paths redacted; terminal payload preserved). Decompress it, then replay with
`scriptreplay --log-out output --log-timing timing.txt`.

## Shared query deadline correction

The same 20 ms query budget and 250 ms trust delay previously sent one HTTP
request after expiry and reported 263 ms of query execution. The correction
reports 20 ms and sends zero requests. Immediate consent still returns one row
and sends one request in both cases. `elapsed_ms` includes CLI startup; the
`ran ...ms` error field measures query execution.

The [recording](deadline-output.gz), [timing](deadline-timing.txt) and
[readable transcript](deadline-transcript.txt) retain the actual paired output.
Download/decompress and replay with `scriptreplay --log-out deadline-output
--log-timing deadline-timing.txt`. Playback requires a local terminal; there is
no claimed inline player. These small artifacts stay with this demo in Git.

Reproduce with `python3 deadline.py <binary-directory>` on each candidate.
The directory contains copies named `quipu-service-candidate` and
`endpoint_pointer_fixture`; build them with the command above. The probe uses
a temporary database, a loopback HTTP endpoint and a stub consent CLI.
The retained before candidate has the same resolver/service blobs as source
`0749d3b45fafc7720074ea50569327043dc7acda`. The after candidate was built from
that source plus the deadline correction in this tree, with dirty diff SHA256
`981da6f89fbeee57219291ad046cf838c5650e3809af08f2c062ca49704514b1`
and executable SHA256
`c909c2ff2e98016a5140da336ab51ce137cd12caf50c200b5b8f9b41b189dbe9`.
This demonstrates candidate behavior, not an installed service or real consent
policy. Separate unit controls check child kill/reap and the HTTP budget left
after earlier work.
