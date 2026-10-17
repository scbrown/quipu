#!/usr/bin/env python3
"""Record actual CLI behavior on the same isolated fixture before and after."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


def demonstrate(executable, phase):
    profile = Path(__file__).resolve().parents[4] / "shapes/rule-answers.ttl"
    with tempfile.TemporaryDirectory(prefix="rule-answer-demo-") as directory:
        work = Path(directory)
        binary = work / "quipu"
        shutil.copyfile(executable, binary)
        binary.chmod(0o700)
        home = work / "home"
        home.mkdir()
        environment = {"HOME": str(home), "PATH": os.defpath}

        def run(*arguments):
            return subprocess.run(
                [str(binary), *arguments], cwd=work, env=environment,
                capture_output=True, text=True, check=False,
            )

        print(f"{phase}: actual copied CLI, isolated synthetic store")
        version = run("--version")
        assert version.returncode == 0
        print(version.stdout.strip())
        facts = work / "facts.ttl"
        facts.write_text(
            "<urn:supported> <urn:positive> <urn:target> .\n"
            "<urn:refuted> <urn:negative> <urn:target> .\n"
            "<urn:both> <urn:positive> <urn:target> ; <urn:negative> <urn:target> .\n"
        )
        assert run("knot", "facts.ttl", "--db", "fixture.db").returncode == 0
        assert run("shapes", "load", "rule-answers", str(profile), "--db", "fixture.db").returncode == 0
        for subject, positive, negative in [
            ("supported", True, False), ("refuted", False, True),
            ("both", True, True), ("neither", False, False),
        ]:
            for predicate, expected in [("positive", positive), ("negative", negative)]:
                ask = run("read", f"ASK {{ <urn:{subject}> <urn:{predicate}> <urn:target> }}", "--db", "fixture.db")
                assert ask.returncode == 0 and ask.stdout.strip() == str(expected).lower()
            request = {
                "subject": f"urn:{subject}", "positive_predicate": "urn:positive",
                "negative_predicate": "urn:negative", "object": {"iri": "urn:target"},
                "judgment_policy": "urn:policy",
            }
            (work / "request.json").write_text(json.dumps(request))
            print(f"$ quipu rule-query request.json --db fixture.db # {subject}")
            result = run("rule-query", "request.json", "--db", "fixture.db")
            if phase == "BEFORE":
                assert result.returncode == 1 and "unknown command: rule-query" in result.stderr
                print(f"exit={result.returncode}; {result.stderr.splitlines()[0]}")
            else:
                assert result.returncode == 0, result.stderr
                answer = json.loads(result.stdout)
                assert answer["status"] == "complete"
                assert answer["evidence"]["truth"] == subject
                assert answer["evidence"]["positive_supported"] == positive
                assert answer["evidence"]["negative_supported"] == negative
                assert answer["judgment"]["disposition"] == "none_recorded"
                assert answer["action_authority"] == "not_granted"
                print(json.dumps({
                    "status": answer["status"], "truth": answer["evidence"]["truth"],
                    "positive": positive, "negative": negative,
                    "judgment": answer["judgment"]["disposition"],
                    "action_authority": answer["action_authority"],
                }, sort_keys=True))
        print("Eight unchanged ASK controls: PASS; no installation or live graph mutation")


if __name__ == "__main__":
    assert len(sys.argv) == 3, "usage: demo.py BASE_CLI CANDIDATE_CLI"
    demonstrate(Path(sys.argv[1]).resolve(), "BEFORE")
    demonstrate(Path(sys.argv[2]).resolve(), "AFTER")
