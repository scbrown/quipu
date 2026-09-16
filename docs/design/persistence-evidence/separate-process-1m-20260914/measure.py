# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Separate persistent-engine processes; Linux cgroup and phase-aligned receipts."""
import argparse
import gzip
import hashlib
import json
import os
import selectors
import shutil
import sqlite3
import subprocess
import tempfile
import threading
import time
from pathlib import Path


def save(path, data):
    path.write_text(json.dumps(data, indent=2) + "\n")


def memory(pid):
    result = {"pid": pid, "time_ns": time.time_ns()}
    for file, keys in (
        ("status", {"VmRSS", "VmHWM", "RssAnon", "RssFile", "VmSwap", "Threads"}),
        ("smaps_rollup", {"Pss", "Pss_Anon", "Pss_File", "Private_Dirty"}),
    ):
        for line in Path(f"/proc/{pid}/{file}").read_text().splitlines():
            key, _, value = line.partition(":")
            if key in keys:
                result[key + ("" if key == "Threads" else "_kib")] = int(value.split()[0])
    return result


def cgroup(pid):
    relative = Path(f"/proc/{pid}/cgroup").read_text().strip().split("::")[-1]
    base = Path("/sys/fs/cgroup") / relative.lstrip("/")
    return {name: (base / name).read_text().strip() for name in (
        "cpu.max", "memory.max", "memory.swap.max", "memory.current", "memory.peak",
        "memory.events", "memory.swap.current", "cpu.stat",
    )}


def host():
    disk = shutil.disk_usage("/")
    return {
        "time_ns": time.time_ns(),
        "disk_bytes": {"total": disk.total, "used": disk.used, "free": disk.free},
        "disk_used_fraction": disk.used / disk.total,
        "loadavg": list(os.getloadavg()),
        "meminfo": Path("/proc/meminfo").read_text(),
        "memory_pressure": Path("/proc/pressure/memory").read_text(),
        "thermal_millidegrees": {
            str(p.relative_to('/sys/class/hwmon')): int(p.read_text())
            for p in Path("/sys/class/hwmon").glob("hwmon*/temp*_input")
        },
        "vmstat": {k: int(v) for k, v in (
            line.split() for line in Path("/proc/vmstat").read_text().splitlines()
        ) if k in {"pswpin", "pswpout", "pgmajfault"}},
    }


def receive(proc, timeout):
    with selectors.DefaultSelector() as selector:
        selector.register(proc.stdout, selectors.EVENT_READ)
        if not selector.select(timeout):
            raise TimeoutError(f"worker reply timeout after {timeout}s")
    line = proc.stdout.readline()
    if not line:
        raise RuntimeError(f"worker exited: {proc.poll()}")
    return json.loads(line)


def send(proc, message, timeout=120):
    proc.stdin.write(json.dumps(message) + "\n")
    proc.stdin.flush()
    return receive(proc, timeout)


def store_bytes(folder):
    files = [p for p in folder.rglob("*") if p.is_file()]
    return {"logical": sum(p.stat().st_size for p in files),
            "allocated": sum(p.stat().st_blocks * 512 for p in files), "files": len(files)}


def sqlite_counts(path):
    with sqlite3.connect(f"file:{path}?mode=ro", uri=True) as conn:
        return {"live_facts": conn.execute(
            "SELECT COUNT(*) FROM facts WHERE op=1 AND valid_to IS NULL"
        ).fetchone()[0], "vectors": conn.execute("SELECT COUNT(*) FROM vectors").fetchone()[0]}


def arm(args, name, queries):
    folder = args.output / name
    folder.mkdir()
    temp = Path(tempfile.mkdtemp(prefix=f"memory-{name}-", dir=args.scratch))
    path = temp / ("store.db" if name == "quipu" else "rocksdb")
    samples, stop, phase = [], threading.Event(), ["startup"]
    record = {"engine": name, "preflight": host(), "queries": [], "phases": {}}
    env = {k: v for k, v in os.environ.items() if not k.startswith(("QUIPU_", "BOBBIN_"))}
    with (folder / "stderr.log").open("w") as log:
        proc = subprocess.Popen([
            "systemd-run", "--user", "--scope", "--quiet",
            "-p", "CPUQuota=200%", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
            str(args.binary), name, str(path), str(args.dataset), args.graph, args.sha,
        ], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, text=True, env=env, cwd=temp)
        thread = None
        pid = None
        try:
            ready = receive(proc, 120)
            assert ready["event"] == "ready_empty"
            pid = ready["pid"]
            record["pid"] = pid
            record["phases"]["ready_empty"] = memory(pid)
            limits = cgroup(pid)
            assert limits["memory.max"] == str(6 * 1024**3)
            assert limits["memory.swap.max"] == "0"
            quota, period = map(int, limits["cpu.max"].split())
            assert quota == 2 * period
            record["cgroup_start"] = limits

            def sampler():
                while not stop.is_set():
                    try:
                        samples.append({"phase": phase[0], **memory(pid)})
                    except (OSError, ProcessLookupError):
                        break
                    stop.wait(0.1)

            thread = threading.Thread(target=sampler)
            thread.start()
            phase[0] = "ingest"
            loaded = send(proc, {"op": "load", "declared_count": args.count}, timeout=900)
            assert loaded["event"] == "ready_loaded" and loaded["pid"] == pid
            record["load"] = loaded
            phase[0] = "ready_loaded"
            record["phases"]["ready_loaded"] = memory(pid)
            record["bytes_after_load"] = store_bytes(temp)
            if name == "quipu":
                record["counts"] = sqlite_counts(path)
                assert record["counts"] == {"live_facts": args.unique + 3, "vectors": 0}
            else:
                assert loaded["oxigraph_quads"] == args.unique
                record["counts"] = {"quads": loaded["oxigraph_quads"], "vectors": "not supported"}
            save(folder / "receipt.json", record)
            expected = {}
            for stage, concurrency, rounds in (("warmup", 1, 1), ("c1", 1, args.rounds), ("c4", 4, args.rounds)):
                phase[0] = stage
                record["phases"][stage + "_before"] = memory(pid)
                for repeat in range(rounds):
                    for query_name, query_text in queries:
                        before = memory(pid)
                        result = send(proc, {"op": "query", "query": query_text, "concurrency": concurrency})
                        assert result["pid"] == pid and len(result["results"]) == concurrency
                        for value in result["results"]:
                            if "error" in value:
                                continue
                            signature = (value["rows"], value["multiset_sha256"])
                            if query_name in expected:
                                assert signature == expected[query_name], (name, query_name, signature, expected[query_name])
                            expected[query_name] = signature
                        record["queries"].append({"phase": stage, "repeat": repeat, "name": query_name,
                            "concurrency": concurrency, "before": before, "after": memory(pid), **result})
                        save(folder / "receipt.json", record)
                record["phases"][stage + "_after"] = memory(pid)
                print(name, stage, record["phases"][stage + "_after"], flush=True)
            phase[0] = "idle_tail"
            time.sleep(5)
            record["phases"]["idle_tail"] = memory(pid)
            record["bytes_after_queries"] = store_bytes(temp)
            record["cgroup_end"] = cgroup(pid)
            assert "oom_kill 0" in record["cgroup_end"]["memory.events"]
            record["postflight"] = host()
            record["complete"] = True
            proc.stdin.write('{"op":"stop"}\n')
            proc.stdin.flush()
            proc.wait(timeout=30)
            assert proc.returncode == 0
        except BaseException as error:
            record["error"] = repr(error)
            if pid:
                os.kill(pid, 15)
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                if pid:
                    os.kill(pid, 9)
                proc.wait(timeout=10)
            raise
        finally:
            stop.set()
            if thread:
                thread.join()
            save(folder / "receipt.json", record)
            with gzip.open(folder / "samples.json.gz", "wt") as stream:
                json.dump(samples, stream)
            shutil.rmtree(temp)
            save(folder / "cleanup.json", {"temporary_store_removed": not temp.exists()})
    return record


def main():
    parser = argparse.ArgumentParser()
    for flag in ("binary", "dataset", "output", "scratch", "queries"):
        parser.add_argument("--" + flag, required=True, type=Path)
    parser.add_argument("--sha", required=True)
    parser.add_argument("--count", type=int, default=1091718)
    parser.add_argument("--unique", type=int, default=1078685)
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--graph", default="http://quipu.invalid/watdiv/1M")
    args = parser.parse_args()
    args.binary = args.binary.resolve()
    args.output.mkdir(parents=True)
    assert hashlib.sha256(args.dataset.read_bytes()).hexdigest() == args.sha
    queries = [(p.stem, p.read_text()) for p in sorted(args.queries.glob("*.rq"))]
    assert len(queries) == 20
    save(args.output / "protocol.json", {"dataset_sha256": args.sha, "input_count": args.count,
        "unique_data_triples": args.unique, "rounds": args.rounds, "query_sha256": {
            name: hashlib.sha256(text.encode()).hexdigest() for name, text in queries},
        "memory_scope": "worker PID only; cgroup also includes launcher and charged page cache",
        "timing_validity": "CONTROL-INVALID: no thermal admission, sequential arms, occupied host swap; no latency ranking",
        "memory_validity": "phase-aligned separate processes; thermal gate does not apply; inspect disk and cgroup receipts",
        "cache_control": "no OS cache flush; input sha read before arms; Quipu first then Oxigraph",
        "result_processing": "both arms materialize sorted canonical binding strings and hash every result multiset",
        "worker_lifetime": "same PID for empty readiness, ingest, four persistent readers, warm-up, c1, c4, idle tail",
        "query_concurrency": [1, 4], "vectors": "absent in both arms",
        "query_errors": "recorded as did-not-finish, never zero rows; comparison only for completed common queries",
        "deadline": "30 seconds: native Quipu deadline and Oxigraph cancellation token; both get identical temporary watchdog thread overhead"})
    results = [arm(args, name, queries) for name in ("quipu", "oxigraph")]
    signatures = [{q["name"]: (q["results"][0]["rows"], q["results"][0]["multiset_sha256"])
                   for q in result["queries"] if "error" not in q["results"][0]} for result in results]
    common = sorted(signatures[0].keys() & signatures[1].keys())
    equal = all(signatures[0][name] == signatures[1][name] for name in common)
    save(args.output / "comparison.json", {"completed_common_queries": common, "common_query_multisets_equal": equal,
        "quipu_no_success": sorted(set(name for name, _ in queries) - signatures[0].keys()),
        "oxigraph_no_success": sorted(set(name for name, _ in queries) - signatures[1].keys()),
        "quipu": signatures[0], "oxigraph": signatures[1]})
    assert common and equal, "completed engine answers differ; inspect comparison.json"


if __name__ == "__main__":
    main()
