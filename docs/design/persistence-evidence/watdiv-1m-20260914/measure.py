# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Bounded process measurements. All runs are CONTROL-INVALID diagnostics."""

import concurrent.futures
import hashlib
import json
import os
import shutil
import socket
import sqlite3
import subprocess
import sys
import threading
import time
import urllib.request
from pathlib import Path

ROOT = Path(os.environ["CHECKPOINT_ROOT"]).resolve()
BIN = Path(os.environ["QUIPU_BENCH_BIN"]).resolve()
HEADER = {
    "validity": "CONTROL-INVALID",
    "conditions": [
        "disk 94%, exceeds peer <=80% gate",
        "swap occupied",
        "shared host heavy co-tenants",
        "no ten-minute thermal admission",
    ],
    "scope": "raw diagnostic only; no ranking or admission-green claim",
    "cpu_quota": "200%",
    "memory_max": "6G",
    "memory_swap_max": 0,
    "os_cache": "not flushed",
    "query_rounds": "one serial warmup and one measured homogeneous wave per template",
    "server_timeout_ms": 30000,
    "client_timeout_s": 40,
    "read_pool_size": 4,
}


def memory(pid):
    result = {"time": time.time(), "pid": pid}
    for filename, keys in [
        ("status", ["VmRSS", "VmHWM", "RssAnon", "RssFile", "VmSwap", "Threads"]),
        ("smaps_rollup", ["Pss", "Pss_Anon", "Pss_File", "Private_Dirty"]),
    ]:
        try:
            lines = Path(f"/proc/{pid}/{filename}").read_text().splitlines()
            for line in lines:
                key, _, value = line.partition(":")
                if key in keys:
                    result[key] = value.strip()
        except (OSError, ProcessLookupError):
            result["unavailable"] = True
    return result


def controls():
    data = {"time": time.time(), "thermal_millidegrees": {}}
    for p in Path("/sys/class/hwmon").glob("hwmon*/temp*_input"):
        try:
            data["thermal_millidegrees"][str(p)] = int(p.read_text())
        except OSError:
            pass
    data["cgroup"] = {}
    relative = Path("/proc/self/cgroup").read_text().strip().split("::")[-1]
    for name in (
        "memory.current",
        "memory.peak",
        "memory.events",
        "memory.swap.current",
        "cpu.stat",
        "cpu.max",
        "memory.max",
        "memory.swap.max",
    ):
        try:
            data["cgroup"][name] = (
                (Path("/sys/fs/cgroup") / relative.lstrip("/") / name)
                .read_text()
                .strip()
            )
        except OSError:
            data["cgroup"][name] = None
    data["memory_pressure"] = Path("/proc/pressure/memory").read_text()
    data["vmstat"] = {
        k: int(v)
        for k, v in (
            line.split() for line in Path("/proc/vmstat").read_text().splitlines()
        )
        if k in ("pswpin", "pswpout", "pgmajfault")
    }
    return data


def save(path, data):
    temp = path.with_suffix(".partial")
    temp.write_text(json.dumps(data, indent=2))
    temp.replace(path)


def launch(args, folder):
    env = {
        k: v for k, v in os.environ.items() if not k.startswith(("QUIPU_", "BOBBIN_"))
    }
    log = (folder / "process.log").open("w")
    proc = subprocess.Popen(args, cwd=folder, env=env, stdout=log, stderr=log)
    return proc, log


def sampler(proc, stop, samples):
    while not stop.is_set():
        samples.append({"memory": memory(proc.pid), "host": controls()})
        stop.wait(1)


def counts(db):
    with sqlite3.connect(f"file:{db}?mode=ro", uri=True) as conn:
        return {
            "facts_history": conn.execute("SELECT COUNT(*) FROM facts").fetchone()[0],
            "facts_live": conn.execute(
                "SELECT COUNT(*) FROM facts WHERE op=1 AND valid_to IS NULL"
            ).fetchone()[0],
            "by_graph": conn.execute(
                "SELECT g, COUNT(*) FROM facts WHERE op=1 AND valid_to IS NULL GROUP BY g"
            ).fetchall(),
            "terms": conn.execute("SELECT COUNT(*) FROM terms").fetchone()[0],
        }


def ingest():
    folder = ROOT / "load"
    db = folder / "base.db"
    if db.exists():
        raise RuntimeError("Refusing to overwrite existing load")
    args = [
        str(BIN / "quipu"),
        "ingest",
        os.environ["WATDIV_DATASET"],
        "--db",
        str(db),
        "--graph",
        "http://quipu.invalid/watdiv/1M",
        "--timestamp",
        "2026-01-01T00:00:00Z",
        "--declare-count",
        "1091718",
        "--declare-sha256",
        "c158998c66e11b33bc56cf7fa3cbc9e69c1c36bf9bdd1bab447d8a64e2d8da75",
        "--format",
        "nt",
        "--chunk",
        "50000",
    ]
    start = time.monotonic()
    proc, log = launch(args, folder)
    samples, stop = [], threading.Event()
    thread = threading.Thread(target=sampler, args=(proc, stop, samples))
    thread.start()
    try:
        rc = proc.wait(timeout=900)
    except subprocess.TimeoutExpired:
        proc.kill()
        rc = proc.wait()
    finally:
        stop.set()
        thread.join()
        log.close()
    data = {
        "header": HEADER,
        "exit_code": rc,
        "wall_seconds": time.monotonic() - start,
        "pid": proc.pid,
        "samples": samples,
        "files": {p.name: p.stat().st_size for p in folder.glob("base.db*")},
    }
    if rc == 0:
        data["counts"] = counts(db)
    save(ROOT / "load.json", data)
    print(json.dumps({k: v for k, v in data.items() if k != "samples"}), flush=True)


def arm(concurrency):
    folder = ROOT / f"c{concurrency}"
    folder.mkdir()
    shutil.copytree(ROOT / "load/.bobbin", folder / ".bobbin")
    with (
        sqlite3.connect(ROOT / "load/base.db") as src,
        sqlite3.connect(folder / "store.db") as dst,
    ):
        src.backup(dst)
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    proc, log = launch(
        [
            str(BIN / "quipu-server"),
            "--db",
            str(folder / "store.db"),
            "--bind",
            f"127.0.0.1:{port}",
        ],
        folder,
    )
    samples, stop = [], threading.Event()
    thread = threading.Thread(target=sampler, args=(proc, stop, samples))
    thread.start()
    data = {
        "header": HEADER,
        "concurrency": concurrency,
        "pid": proc.pid,
        "queries": [],
        "samples": samples,
    }

    def request(path, query=None):
        start = time.monotonic()
        before = memory(proc.pid)
        req = urllib.request.Request(
            f"http://127.0.0.1:{port}" + path,
            data=json.dumps({"query": query, "verbose": True}).encode()
            if query
            else None,
            headers={
                "Content-Type": "application/json",
                "X-Quipu-Client": "agent-adhoc",
            },
        )
        record = {"before": before}
        try:
            with urllib.request.urlopen(req, timeout=40) as response:
                body = response.read()
                result = json.loads(body)
                rows = result.get("rows", [])
                canonical = "\n".join(
                    sorted(
                        json.dumps(r, sort_keys=True, separators=(",", ":"))
                        for r in rows
                    )
                )
                record.update(
                    status=response.status,
                    count=result.get("count", len(rows)),
                    truncated=result.get("truncated", False),
                    multiset_sha256=hashlib.sha256(canonical.encode()).hexdigest(),
                    response_bytes=len(body),
                )
                if not query:
                    record["result"] = result
        except (OSError, ValueError, RuntimeError) as exc:
            record.update(status="DNF", error=str(exc))
        record.update(wall_seconds=time.monotonic() - start, after=memory(proc.pid))
        return record

    try:
        for _ in range(120):
            if proc.poll() is not None:
                raise RuntimeError("server exited before readiness")
            version = request("/version")
            if version["status"] == 200:
                data["ready"] = version
                data["ready_idle_memory"] = memory(proc.pid)
                break
            time.sleep(0.5)
        else:
            raise RuntimeError("readiness timeout")
        save(ROOT / f"c{concurrency}.json", data)
        for queryfile in sorted((ROOT / "queries").glob("*.rq")):
            query = queryfile.read_text()
            warm = request("/query", query)
            data["queries"].append(
                {"template": queryfile.stem, "phase": "warm", "receipt": warm}
            )
            with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as pool:
                receipts = list(
                    pool.map(
                        lambda _, text=query: request("/query", text),
                        range(concurrency),
                    )
                )
            data["queries"].extend(
                {"template": queryfile.stem, "phase": "measured", "receipt": r}
                for r in receipts
            )
            save(ROOT / f"c{concurrency}.json", data)
            print(
                queryfile.stem,
                concurrency,
                [(r["status"], r.get("count")) for r in receipts],
                flush=True,
            )
        data["tail_idle_memory"] = memory(proc.pid)
    except (OSError, ValueError, RuntimeError) as exc:
        data["error"] = str(exc)
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
        stop.set()
        thread.join()
        log.close()
        data["exit_code"] = proc.returncode
        save(ROOT / f"c{concurrency}.json", data)


if __name__ == "__main__":
    if sys.argv[1] == "ingest":
        ingest()
    else:
        arm(int(sys.argv[1]))
