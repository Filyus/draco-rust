#!/usr/bin/env python3
"""Point-cloud methods and constants on whatever machine this runs on.

Builds the codec several times from `git archive` copies -- HEAD, HEAD again
as a control, HEAD with one technique switched off, and the probe branches --
each behind its own small harness, generates synthetic clouds, and times the
variants against HEAD in one sitting with the order rotated every round.
Writes Markdown tables to `$GITHUB_STEP_SUMMARY` (or stdout) and everything
it measured to `results.json`.

Environment:
  BENCH_WORK    working directory (default: <repo>/target/bench)
  BENCH_POINTS  points per synthetic cloud (default 500000)
  BENCH_ROUNDS  rounds per comparison (default 3)
  BENCH_ITERS   iterations per run, best kept (default 3)
  BENCH_PAUSE   seconds before each run (default 1)
  BENCH_ONLY    comma-separated experiments to run (buckets,ablation,inner,
                order,rule); all by default
"""
import io
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = subprocess.check_output(["git", "rev-parse", "--show-toplevel"], cwd=HERE, text=True).strip()
WORK = os.environ.get("BENCH_WORK", os.path.join(ROOT, "target", "bench"))
POINTS = int(os.environ.get("BENCH_POINTS", "500000"))
ROUNDS = int(os.environ.get("BENCH_ROUNDS", "3"))
ITERS = int(os.environ.get("BENCH_ITERS", "3"))
PAUSE = float(os.environ.get("BENCH_PAUSE", "1"))
ONLY = set(filter(None, os.environ.get("BENCH_ONLY", "").split(",")))
EXE = ".exe" if os.name == "nt" else ""
TARGET = os.path.join(WORK, "target")
THREADS = os.cpu_count() or 1

REFS = {
    "head": "HEAD",
    "q34": "origin/probe/bucket-rule-three-quarters",
    "pick": "origin/probe/order-choice-by-trial-encode",
}

# (file under crates/draco-core/src, text to find exactly once, replacement)
PATCHES = {
    "head": ("head", []),
    "head2": ("head", []),
    "q34": ("q34", []),
    "pick": ("pick", []),
    "nob": ("head", [("rans_symbol_decoder.rs",
                      "    fn build_buckets(&mut self) {\n",
                      "    fn build_buckets(&mut self) {\n        return;\n")]),
    "allb": ("head", [("rans_symbol_decoder.rs",
                       "        if self.lut.len() * self.lut.slot_bytes() > L2_BYTES || owned * 4 >= BUCKETS * 3 {\n",
                       "        eprintln!(\"OWNED {owned} PRECISION {precision}\");\n        if true {\n")]),
    "nop": ("head", [("symbol_encoding.rs",
                      ") -> Option<(Vec<u32>, Vec<u32>)> {\n",
                      ") -> Option<(Vec<u32>, Vec<u32>)> {\n    if true {\n        return None;\n    }\n")]),
    "nos": ("head", [("rans_symbol_decoder.rs",
                      "    fn build_steps(&mut self) {\n",
                      "    fn build_steps(&mut self) {\n        return;\n")]),
    "noi": ("head", [("point_cloud_encoder.rs",
                      "let inner_threads = (threads / num_attributes.max(1) as usize).max(1);",
                      "let inner_threads = 1;")]),
}

summary = []
results = {}


def log(*args):
    print(*args, flush=True)


def out(text=""):
    summary.append(text)


def run(cmd, **kwargs):
    log("+", " ".join(cmd))
    return subprocess.run(cmd, check=True, **kwargs)


def hardware():
    info = {"platform": platform.platform(), "machine": platform.machine(),
            "processor": platform.processor(), "logical_cpus": THREADS}
    def capture(cmd):
        try:
            return subprocess.run(cmd, capture_output=True, text=True, timeout=60).stdout.strip()
        except Exception as error:  # noqa: BLE001 -- best effort, reported as text
            return f"unavailable: {error}"
    system = platform.system()
    if system == "Linux":
        info["lscpu"] = capture(["lscpu"])
        caches = []
        base = "/sys/devices/system/cpu/cpu0/cache"
        for entry in sorted(os.listdir(base)) if os.path.isdir(base) else []:
            def read(name):
                try:
                    return open(os.path.join(base, entry, name)).read().strip()
                except OSError:
                    return "?"
            caches.append(f"L{read('level')} {read('type')} {read('size')} shared by {read('shared_cpu_list')}")
        info["caches"] = caches
    elif system == "Darwin":
        info["sysctl"] = capture(["sysctl", "machdep.cpu.brand_string", "hw.ncpu", "hw.perflevel0.physicalcpu",
                                  "hw.perflevel0.l1dcachesize", "hw.perflevel0.l2cachesize",
                                  "hw.perflevel0.cpusperl2", "hw.l2cachesize", "hw.l3cachesize"])
    elif system == "Windows":
        info["cim"] = capture(["powershell", "-NoProfile", "-Command",
                               "Get-CimInstance Win32_Processor | Format-List Name,NumberOfCores,"
                               "NumberOfLogicalProcessors,L2CacheSize,L3CacheSize"])
    info["rustc"] = capture(["rustc", "-V"])
    return info


def make_variant(name):
    ref_key, patches = PATCHES[name]
    root = os.path.join(WORK, name)
    if os.path.isdir(root):
        shutil.rmtree(root)
    os.makedirs(root)
    archive = subprocess.check_output(["git", "archive", "--format=tar", REFS[ref_key],
                                       "crates/draco-core", "crates/draco-io"], cwd=ROOT)
    with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
        tar.extractall(root)
    # A dev-dependency by a local path on the machine the branch was written
    # on; nothing here builds what needs it.
    for crate in ("draco-core", "draco-io"):
        manifest = os.path.join(root, "crates", crate, "Cargo.toml")
        lines = open(manifest, encoding="utf-8").read().splitlines(keepends=True)
        open(manifest, "w", encoding="utf-8").writelines(l for l in lines if "packed_spatial_index" not in l)
    for file, old, new in patches:
        path = os.path.join(root, "crates", "draco-core", "src", file)
        text = open(path, encoding="utf-8").read()
        if text.count(old) != 1:
            raise SystemExit(f"{name}: {file} has the patch anchor {text.count(old)} times, not once")
        open(path, "w", encoding="utf-8").write(text.replace(old, new))
    core = os.path.join(root, "crates", "draco-core").replace("\\", "/")
    io_ = os.path.join(root, "crates", "draco-io").replace("\\", "/")
    for tool, deps in (("pcprof", f'draco-core = {{ path = "{core}" }}\ndraco-io = {{ path = "{io_}" }}\n'),
                       ("rbench", f'draco-core = {{ path = "{core}" }}\n')):
        crate = os.path.join(root, f"{tool}-{name}")
        os.makedirs(os.path.join(crate, "src"))
        shutil.copy(os.path.join(HERE, tool, "main.rs"), os.path.join(crate, "src", "main.rs"))
        open(os.path.join(crate, "Cargo.toml"), "w").write(
            f'[package]\nname = "{tool}-{name}"\nversion = "0.0.0"\nedition = "2021"\npublish = false\n\n'
            f"[workspace]\n\n[dependencies]\n{deps}")


def build(name, tool="pcprof"):
    manifest = os.path.join(WORK, name, f"{tool}-{name}", "Cargo.toml")
    run(["cargo", "build", "--release", "--quiet", "--manifest-path", manifest],
        env={**os.environ, "CARGO_TARGET_DIR": TARGET})
    return os.path.join(TARGET, "release", f"{tool}-{name}{EXE}")


PARSE = re.compile(r"best ([0-9.]+) s, ([0-9]+) bytes, hash (\w+)")


def compare(arms, args, rounds=None):
    """Best time of each arm, the order rotated every round, a pause before
    every run. `arms` maps a name to an executable."""
    names = list(arms)
    best = {}
    for r in range(rounds or ROUNDS):
        for k in range(len(names)):
            name = names[(k + r) % len(names)]
            time.sleep(PAUSE)
            proc = subprocess.run([arms[name], *args], capture_output=True, text=True)
            match = PARSE.search(proc.stderr)
            if proc.returncode != 0 or not match:
                raise SystemExit(f"{name} {args}: {proc.returncode}\n{proc.stderr[-2000:]}")
            t, size, digest = float(match[1]), int(match[2]), match[3]
            if name not in best or t < best[name][0]:
                best[name] = (t, size, digest)
    return best


def table(title, header, rows):
    out(f"### {title}\n")
    out("| " + " | ".join(header) + " |")
    out("| " + " | ".join("---" for _ in header) + " |")
    for row in rows:
        out("| " + " | ".join(str(c) for c in row) + " |")
    out()


def pct(a, b):
    return f"{(a - b) / b:+.1%}"


def main():
    os.makedirs(WORK, exist_ok=True)
    run(["git", "fetch", "origin", "probe/bucket-rule-three-quarters", "probe/order-choice-by-trial-encode"], cwd=ROOT)
    hw = hardware()
    results["hardware"] = hw
    out(f"## Point-cloud bench on {hw['platform']}, {THREADS} logical CPUs\n")
    out("```")
    for key in ("lscpu", "caches", "sysctl", "cim", "rustc"):
        if key in hw:
            out("\n".join(hw[key]) if isinstance(hw[key], list) else hw[key])
    out("```\n")
    out(f"{POINTS} points a cloud, best of {ITERS} iterations over {ROUNDS} rounds, order rotated, "
        f"{PAUSE}s pause. `head2` is HEAD built again: its gap to `head` is the floor.\n")

    variants = ["head", "head2", "nob", "nop", "nos", "noi", "q34", "pick", "allb"]
    for name in variants:
        make_variant(name)
    exe = {name: build(name) for name in variants if name != "allb"}
    rb = {name: build(name, "rbench") for name in ("allb", "nob")}

    data = os.path.join(WORK, "data")
    if not os.path.isdir(data):
        run(["cargo", "run", "--release", "--quiet", "--manifest-path",
             os.path.join(WORK, "head", "crates", "draco-core", "Cargo.toml"),
             "--example", "synthetic_clouds", "--", data, str(POINTS), "1"],
            env={**os.environ, "CARGO_TARGET_DIR": TARGET})
    cloud = lambda name: os.path.join(data, f"{name}.ply")
    every = lambda name: not ONLY or name in ONLY

    if every("buckets"):
        rows, sweep = [], []
        for bits in (12, 13, 16, 18):
            for scale in (10, 160, 320, 480, 640, 1280):
                times, owned, precision = {}, None, None
                for r in range(ROUNDS):
                    for name in (("allb", "nob") if r % 2 == 0 else ("nob", "allb")):
                        time.sleep(PAUSE)
                        proc = subprocess.run([rb[name], str(scale), str(bits), "2000000", str(ITERS)],
                                              capture_output=True, text=True, check=True)
                        ns = float(re.search(r"([0-9.]+) ns/symbol", proc.stdout)[1])
                        times[name] = min(times.get(name, ns), ns)
                        found = re.search(r"OWNED (\d+) PRECISION (\d+)", proc.stderr)
                        if found:
                            owned, precision = int(found[1]), int(found[2])
                share = owned / 4096 if owned is not None else None
                sweep.append({"bits": bits, "scale": scale, "precision": precision, "owned": share,
                              "buckets_ns": times["allb"], "table_ns": times["nob"]})
                rows.append([precision and f"2^{precision.bit_length() - 1}", f"{share:.0%}" if share else "-",
                             f"{times['allb']:.2f}", f"{times['nob']:.2f}",
                             "buckets" if times["allb"] < times["nob"] else "table"])
        results["buckets"] = sweep
        table("rANS buckets against the slot table, ns a symbol",
              ["precision", "owned", "buckets", "table", "faster"], rows)

    if every("ablation"):
        rows, record = [], []
        for name in ("splat", "aerial", "spinning", "terrestrial"):
            for threads in (1, THREADS):
                best = compare({k: exe[k] for k in ("head", "head2", "nob", "nop", "nos")},
                               ["dec", cloud(name), str(ITERS), str(threads), "search", "5"])
                h = best["head"][0]
                record.append({"cloud": name, "threads": threads, **{k: v[0] for k, v in best.items()}})
                rows.append([name, threads, f"{h:.4f}", pct(best["head2"][0], h), pct(best["nob"][0], h),
                             pct(best["nop"][0], h), pct(best["nos"][0], h)])
        results["ablation"] = record
        table("Decode without each technique (slower is positive), search order",
              ["cloud", "threads", "head s", "head2 (floor)", "no buckets", "no pairs", "no steps"], rows)

    if every("inner"):
        rows, record = [], []
        for name in ("aerial", "terrestrial", "spinning", "splat"):
            for order in ("plain", "search"):
                best = compare({k: exe[k] for k in ("head", "head2", "noi")},
                               ["enc", cloud(name), str(ITERS), str(THREADS), order, "5"])
                h = best["head"][0]
                record.append({"cloud": name, "order": order, **{k: v[0] for k, v in best.items()}})
                rows.append([name, order, f"{h:.4f}", pct(best["head2"][0], h), pct(best["noi"][0], h)])
        results["inner"] = record
        table(f"Encode without an attribute's own threads, {THREADS} threads",
              ["cloud", "order", "head s", "head2 (floor)", "no inner threads"], rows)

    if every("order"):
        rows, record = [], []
        shapes = ["uniform", "aerial", "spinning", "terrestrial", "lattice", "far_away", "skewed", "splat"]
        for name in shapes + [s + "_shuffled" for s in shapes]:
            plain = compare({"head": exe["head"]}, ["enc", cloud(name), "1", str(THREADS), "plain", "5"], 1)["head"][1]
            curve = compare({"head": exe["head"]}, ["enc", cloud(name), "1", str(THREADS), "curve", "5"], 1)["head"][1]
            best = compare({k: exe[k] for k in ("head", "head2", "pick")},
                           ["enc", cloud(name), str(ITERS), str(THREADS), "search", "5"])
            (ht, hb, _), (pt, pb, _) = best["head"], best["pick"]
            low = min(plain, curve, hb, pb)
            record.append({"cloud": name, "plain": plain, "curve": curve, "head_bytes": hb, "pick_bytes": pb,
                           "head_s": ht, "head2_s": best["head2"][0], "pick_s": pt})
            rows.append([name, f"{hb / low - 1:+.2%}", f"{pb / low - 1:+.2%}", f"{ht:.4f}",
                         pct(best["head2"][0], ht), pct(pt, ht)])
        results["order"] = record
        table("Order choice: estimate with 3% (head) against trial encode (pick); bytes over the "
              "smallest of input, curve, head and pick",
              ["cloud", "head bytes", "pick bytes", "head s", "head2 (floor)", "pick time"], rows)

    if every("rule"):
        rows, record = [], []
        for name in ("aerial", "spinning", "terrestrial", "splat"):
            for order in ("plain", "search"):
                for threads in (1, THREADS):
                    best = compare({k: exe[k] for k in ("head", "head2", "q34")},
                                   ["dec", cloud(name), str(ITERS), str(threads), order, "5"])
                    h = best["head"][0]
                    record.append({"cloud": name, "order": order, "threads": threads,
                                   **{k: v[0] for k, v in best.items()}})
                    rows.append([name, order, threads, f"{h:.4f}", pct(best["head2"][0], h), pct(best["q34"][0], h)])
        results["rule"] = record
        table("Bucket rule: L2 clause (head) against three quarters alone (q34), decode",
              ["cloud", "order", "threads", "head s", "head2 (floor)", "q34"], rows)

    text = "\n".join(summary)
    target = os.environ.get("GITHUB_STEP_SUMMARY")
    if target:
        with open(target, "a", encoding="utf-8") as f:
            f.write(text + "\n")
    print(text)
    with open(os.path.join(WORK, "results.json"), "w", encoding="utf-8") as f:
        json.dump(results, f, indent=1)


if __name__ == "__main__":
    main()
