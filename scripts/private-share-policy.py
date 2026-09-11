#!/usr/bin/env python3
"""Project live scrub policy on the trusted producer; never publish this output."""

import json
import os
from pathlib import Path
import sys
import urllib.request


PATTERN = """
?rule a aegis:InternalIdentifierPattern ; aegis:enforcementTier "block" .
OPTIONAL { ?rule rdfs:label ?label }
OPTIONAL { ?rule aegis:regex ?regex }
"""
QUERY = """
PREFIX aegis: <http://aegis.gastown.local/ontology/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
SELECT DISTINCT (STR(?rule) AS ?iri) ?label ?regex WHERE {
  { %s } UNION { GRAPH ?catalogue { %s } }
} ORDER BY ?iri ?label ?regex
""" % (PATTERN, PATTERN)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ValueError("policy authority redirected")


def project(server, token):
    if not server or not server.startswith(("http://", "https://")):
        raise ValueError("policy authority is required")
    headers = {
        "Content-Type": "application/json",
        "Accept": "application/json",
        "X-Quipu-Client": "agent-adhoc",
    }
    if token:
        headers["Authorization"] = "Bearer " + token
    request = urllib.request.Request(
        server.rstrip("/") + "/query",
        data=json.dumps({"query": QUERY}).encode(),
        headers=headers,
    )
    opener = urllib.request.build_opener(NoRedirect())
    with opener.open(request, timeout=30) as response:
        if response.status != 200:
            raise ValueError("policy authority did not succeed")
        document = json.load(response)
    rows = document.get("rows")
    if (
        document.get("truncated") is not False
        or not isinstance(rows, list)
        or not rows
        or type(document.get("count")) is not int
        or document["count"] != len(rows)
    ):
        raise ValueError("policy catalogue is empty or incomplete")
    rules = {}
    for row in rows:
        if not isinstance(row, dict):
            raise ValueError("invalid policy row")
        iri, label, regex = (row.get(key) for key in ("iri", "label", "regex"))
        if not all(isinstance(value, str) and value for value in (iri, label, regex)):
            raise ValueError("incomplete policy rule")
        if ":" not in iri or any(c.isspace() or c in '<>"{}|^`\\' for c in iri):
            raise ValueError("invalid policy identity")
        value = (label, regex)
        if iri in rules and rules[iri] != value:
            raise ValueError("conflicting policy definitions")
        rules[iri] = value
    # JSON string escapes are also valid Turtle string escapes. Regex syntax
    # remains the native scrub engine's responsibility, not Python's dialect.
    lines = [
        "@prefix aegis: <http://aegis.gastown.local/ontology/> .",
        "@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .",
    ]
    for iri, (label, regex) in sorted(rules.items()):
        lines.append(
            f"<{iri}> a aegis:InternalIdentifierPattern ; "
            f"rdfs:label {json.dumps(label, ensure_ascii=False)} ; "
            f"aegis:regex {json.dumps(regex, ensure_ascii=False)} ; "
            'aegis:enforcementTier "block" .'
        )
    return "\n".join(lines) + "\n"


def main():
    if len(sys.argv) != 2:
        print("usage: private-share-policy.py <private-new-file>", file=sys.stderr)
        return 2
    try:
        token_file = os.environ.get("QUIPU_POLICY_TOKEN_FILE")
        token = Path(token_file).read_text().strip() if token_file else ""
        turtle = project(os.environ.get("QUIPU_POLICY_SERVER"), token)
        # Exclusive creation prevents accidental overwrite of an existing
        # catalogue; every build must obtain a fresh projection.
        fd = os.open(sys.argv[1], os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w") as output:
            output.write(turtle)
    except Exception:
        # Transport exceptions can contain the private URL; rule errors can
        # contain identifiers. Keep diagnostics inside this trust boundary.
        print("cannot verify live policy; no share may be emitted", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
