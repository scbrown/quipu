#!/usr/bin/env python3
"""Keep server.json honest for the official MCP Registry (aegis-gys8sx).

    usage: mcp-registry.py check [repo-root]
           mcp-registry.py stamp <version> <in> <out>
           mcp-registry.py --selftest

    exit 0  ok
    exit 1  refused (the reason is on stderr)
    exit 2  called wrong

WHY `stamp` AND NOT A HAND-EDITED VERSION. release-plz bumps Cargo.toml and
nothing else, and it has no hook for arbitrary version files. A server.json
version that is edited by hand is a second copy of the release number, and the
day someone forgets, the registry advertises an image tag that does not exist,
so every Install click fails at `docker pull`. So the committed server.json
carries a placeholder version and the publish job stamps the RELEASED version
(taken from the tag, not Cargo.toml) into both `version` and the OCI tag right
before it publishes. There is one source of truth and nothing to forget.

WHY `check` RUNS ON EVERY PR. The registry proves we own the image by reading a
LABEL inside it that must equal server.json's `name`. The name is therefore
spelled in three places (server.json, the Dockerfile LABEL, the README), and a
rename that misses one is refused only at publish time, after a release has
shipped. `check` makes that mismatch a red PR instead. It also refuses any
homelab host in what we publish: a stranger's Install click must give them their
OWN local database, never a pointer at infrastructure they cannot reach.
"""
import json
import re
import sys
import tempfile
from pathlib import Path

NAME = 'io.github.scbrown/quipu'
IMAGE = 'ghcr.io/scbrown/quipu'
DOCKERFILE = Path('packaging/oci/Dockerfile')
# `.svc` is this project's homelab suffix (quipu.svc, search.svc). None of it
# is reachable from a stranger's machine, so none of it may be a default.
HOMELAB = re.compile(r'\b[\w.-]+\.svc\b|\b[\w.-]+\.lan\b')
SEMVER = re.compile(r'^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$')


class Refused(Exception):
    pass


def oci_packages(doc):
    return [p for p in doc.get('packages', []) if p.get('registryType') == 'oci']


def check(root):
    root = Path(root)
    doc = json.loads((root / 'server.json').read_text())
    if doc.get('name') != NAME:
        raise Refused(f'server.json name is {doc.get("name")!r}, expected {NAME!r}')

    label = re.search(r'io\.modelcontextprotocol\.server\.name="([^"]+)"',
                      (root / DOCKERFILE).read_text())
    if not label or label.group(1) != doc['name']:
        found = label.group(1) if label else 'no label'
        raise Refused(f'{DOCKERFILE} LABEL io.modelcontextprotocol.server.name is '
                      f'{found!r}; the registry refuses the image unless it equals {doc["name"]!r}')

    if f'mcp-name: {doc["name"]}' not in (root / 'README.md').read_text():
        raise Refused(f'README.md lacks the visible line `mcp-name: {doc["name"]}`')

    ocis = oci_packages(doc)
    if len(ocis) != 1:
        raise Refused(f'expected exactly one oci package, found {len(ocis)}')
    oci = ocis[0]
    # The registry REJECTS an oci package with a `version` field, and VS Code
    # would append it as a second tag (`image:1.2.3:1.2.3`) if it got through.
    if 'version' in oci:
        raise Refused('the oci package must not carry `version`; the tag lives in `identifier`')
    repo, _, tag = oci['identifier'].rpartition(':')
    if repo != IMAGE:
        raise Refused(f'oci identifier {oci["identifier"]!r} is not {IMAGE}:<tag>')
    if tag != doc.get('version'):
        raise Refused(f'oci tag {tag!r} disagrees with server version {doc.get("version")!r}')

    for path in ('server.json', str(DOCKERFILE)):
        hit = HOMELAB.search((root / path).read_text())
        if hit:
            raise Refused(f'{path} names homelab host {hit.group(0)!r}; published defaults must be local')


def stamp(version, source, dest):
    if not SEMVER.match(version):
        raise Refused(f'{version!r} is not a release version')
    doc = json.loads(Path(source).read_text())
    ocis = oci_packages(doc)
    if len(ocis) != 1:
        raise Refused(f'expected exactly one oci package, found {len(ocis)}')
    doc['version'] = version
    ocis[0]['identifier'] = f'{IMAGE}:{version}'
    Path(dest).write_text(json.dumps(doc, indent=2) + '\n')


def selftest():
    here = Path(__file__).resolve().parents[2]
    good = json.loads((here / 'server.json').read_text())
    failures = []

    def expect(label, want_refused, fn):
        try:
            fn()
            refused = False
        except Refused:
            refused = True
        if refused != want_refused:
            failures.append(f'{label}: expected {"refusal" if want_refused else "pass"}')

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / DOCKERFILE).parent.mkdir(parents=True)

        def lay(doc=None, dockerfile=None, readme=None):
            (root / 'server.json').write_text(json.dumps(doc or good))
            (root / DOCKERFILE).write_text(dockerfile if dockerfile is not None
                                           else (here / DOCKERFILE).read_text())
            (root / 'README.md').write_text(readme if readme is not None
                                            else (here / 'README.md').read_text())

        lay()
        expect('the committed tree', False, lambda: check(root))

        lay(dockerfile='LABEL io.modelcontextprotocol.server.name="io.github.someone/else"\n')
        expect('a Dockerfile LABEL that disagrees', True, lambda: check(root))

        lay(readme='# quipu\n')
        expect('a README without the visible mcp-name line', True, lambda: check(root))

        bad = json.loads(json.dumps(good)); oci_packages(bad)[0]['version'] = good['version']
        lay(doc=bad)
        expect('an oci package with a version field', True, lambda: check(root))

        bad = json.loads(json.dumps(good)); bad['version'] = '9.9.9'
        lay(doc=bad)
        expect('an oci tag that disagrees with the version', True, lambda: check(root))

        bad = json.loads(json.dumps(good)); bad['websiteUrl'] = 'https://quipu.svc/'
        lay(doc=bad)
        expect('a homelab host in server.json', True, lambda: check(root))

        lay()
        out = root / 'stamped.json'
        expect('stamping a release version', False, lambda: stamp('1.2.3', root / 'server.json', out))
        stamped = json.loads(out.read_text())
        if stamped['version'] != '1.2.3' or oci_packages(stamped)[0]['identifier'] != f'{IMAGE}:1.2.3':
            failures.append('stamp did not move both the version and the oci tag')
        (root / 'server.json').write_text(out.read_text())
        expect('a stamped file still passes check', False, lambda: check(root))
        expect('stamping a tag name instead of a version', True,
               lambda: stamp('quipu-ai-v1.2.3', root / 'server.json', out))

    for failure in failures:
        print(f'SELFTEST FAILED: {failure}', file=sys.stderr)
    if failures:
        return 1
    print('mcp-registry selftest: all refusals observed')
    return 0


def main(argv):
    try:
        if argv[:1] == ['--selftest'] and len(argv) == 1:
            return selftest()
        if argv[:1] == ['check'] and len(argv) <= 2:
            check(argv[1] if len(argv) == 2 else '.')
            print('server.json, Dockerfile LABEL and README agree')
            return 0
        if argv[:1] == ['stamp'] and len(argv) == 4:
            stamp(*argv[1:])
            return 0
    except Refused as refusal:
        print(f'refused: {refusal}', file=sys.stderr)
        return 1
    print(__doc__.split('\n\n')[1], file=sys.stderr)
    return 2


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
