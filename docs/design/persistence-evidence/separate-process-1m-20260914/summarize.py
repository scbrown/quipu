# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Verify completed separate-process receipts and derive memory summaries."""
import collections
import gzip
import json
import sys
from pathlib import Path


def summarize(root):
    protocol = json.loads((root / "protocol.json").read_text())
    rounds = protocol["rounds"]
    expected = {"warmup": 20, "c1": 20 * rounds, "c4": 20 * rounds}
    summaries, signatures = {}, {}
    for arm in ("quipu", "oxigraph"):
        folder = root / arm
        receipt = json.loads((folder / "receipt.json").read_text())
        cleanup = json.loads((folder / "cleanup.json").read_text())
        with gzip.open(folder / "samples.json.gz", "rt") as stream:
            samples = json.load(stream)
        assert receipt.get("complete") is True, f"incomplete {arm} arm"
        assert cleanup["temporary_store_removed"] is True
        pid = receipt["pid"]
        assert samples and all(sample["pid"] == pid for sample in samples)
        assert collections.Counter(q["phase"] for q in receipt["queries"]) == expected
        observed = collections.defaultdict(set)
        errors = collections.Counter()
        success = collections.Counter()
        for wave in receipt["queries"]:
            assert wave["pid"] == wave["before"]["pid"] == wave["after"]["pid"] == pid
            n = 4 if wave["phase"] == "c4" else 1
            assert wave["concurrency"] == n and len(wave["results"]) == n
            for result in wave["results"]:
                if "error" in result:
                    errors[wave["name"]] += 1
                    assert "rows" not in result
                else:
                    success[wave["name"]] += 1
                    observed[wave["name"]].add((result["rows"], result["multiset_sha256"]))
        assert all(len(values) == 1 for values in observed.values()), f"unstable {arm} answers"
        signatures[arm] = {name: list(values)[0] for name, values in observed.items()}
        by_phase = {}
        for phase in ("ingest", "ready_loaded", "warmup", "c1", "c4", "idle_tail"):
            phase_samples = [sample for sample in samples if sample["phase"] == phase]
            by_phase[phase] = {
                "sample_count": len(phase_samples),
                "sampled_max_rss_kib": max((s["VmRSS_kib"] for s in phase_samples), default=None),
                "sampled_max_pss_kib": max((s["Pss_kib"] for s in phase_samples), default=None),
            }
        assert all(point["pid"] == pid for point in receipt["phases"].values())
        for key in ("cgroup_start", "cgroup_end"):
            cgroup = receipt[key]
            quota, period = map(int, cgroup["cpu.max"].split())
            assert quota == 2 * period
            assert cgroup["memory.max"] == str(6 * 1024**3)
            assert cgroup["memory.swap.max"] == "0"
            events = dict(line.split() for line in cgroup["memory.events"].splitlines())
            assert events["oom_kill"] == "0"
        summaries[arm] = {
            "pid": pid, "counts": receipt["counts"], "query_waves": len(receipt["queries"]),
            "successful_requests": sum(success.values()), "did_not_finish_requests": sum(errors.values()),
            "success_by_query": dict(sorted(success.items())), "dnf_by_query": dict(sorted(errors.items())),
            "synchronous_phases": receipt["phases"], "sampled_phases": by_phase,
            "ingest_wall_seconds_control_invalid": receipt["load"]["ingest_wall_seconds"],
            "bytes_after_load": receipt["bytes_after_load"], "bytes_after_queries": receipt["bytes_after_queries"],
            "root_disk_admission": all(receipt[k]["disk_used_fraction"] <= 0.8 for k in ("preflight", "postflight")),
            "store_removed": cleanup["temporary_store_removed"],
        }
    assert summaries["quipu"]["pid"] != summaries["oxigraph"]["pid"], "same-process comparison refused"
    common = sorted(signatures["quipu"].keys() & signatures["oxigraph"].keys())
    mismatches = [name for name in common if signatures["quipu"][name] != signatures["oxigraph"][name]]
    output = {"arms": summaries, "completed_common_queries": common,
              "common_result_mismatches": mismatches, "signatures": signatures,
              "scope": "separate-process memory observations; not a latency or overall engine ranking"}
    (root / "summary.json").write_text(json.dumps(output, indent=2) + "\n")
    assert common and not mismatches, "completed engine results differ; inspect summary"
    print(json.dumps(output, indent=2))


if __name__ == "__main__":
    summarize(Path(sys.argv[1]))
