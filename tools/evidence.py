#!/usr/bin/env python3
"""Bulk-export Ghidra evidence for the game functions this project cares about.

Walks the call graph outward from a set of anchor functions and writes one
Markdown file per function containing its signature, decompilation,
disassembly, callers and callees. The result is a local, greppable snapshot of
what the Windows Ghidra knows, so a session can answer "who else touches this"
with `grep` instead of a few dozen MCP round-trips.

    python3 tools/evidence.py                 # depth 1 from the default anchors
    python3 tools/evidence.py --depth 2 --max-functions 3000
    python3 tools/evidence.py --anchor 1403856b0 --depth 2 --out /tmp/loader

Default anchors are every game-exe `FUN_1xxxxxxxx` named in `docs/*.md` or
already dumped in `analysis/*.c` - i.e. everything we have ever written about.
Addresses outside the game module ([0x140000000, 0x160000000)) are dropped, so
the dead CDLoot.asi names at 0x180000000 do not become anchors.

Needs the Windows Ghidra running with the GhidraMCP extension listening (the
same server the `ghidra` MCP bridge talks to; this speaks its HTTP API
directly, so it works whether or not the MCP client is connected). The host is
probed on 127.0.0.1 then the WSL default gateway, matching
`~/.local/bin/ghidra-mcp-bridge-win`.

Output is a snapshot of one game build and goes stale on an update. It is
regenerable and belongs beside `analysis/` in .gitignore, never in a commit.
Re-run after an update and diff the two trees: the functions whose
decompilation moved are the candidate breakage list.

Resumable: a function whose file already exists is not re-fetched (its graph
edges are read back out of the file's metadata line). Pass --force to redo.
"""
import argparse
import concurrent.futures
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent

# The game module: base 0x140000000, image a little under 0x18000000 bytes.
# Anything outside this is not CrimsonDesert.exe (0x180000000 was CDLoot.asi).
MOD_LO = 0x140000000
MOD_HI = 0x160000000

APPMANIFEST = os.environ.get(
    "CD_APPMANIFEST",
    "/mnt/f/SteamLibrary/steamapps/appmanifest_3321460.acf")

FUN_RE = re.compile(r"\bFUN_([0-9a-fA-F]{9,})\b")
CALL_RE = re.compile(r"^\s*([0-9a-f]+):\s+CALL\s+0x([0-9a-f]+)\s*$", re.M)
JMP_RE = re.compile(r"^\s*([0-9a-f]+):\s+JMP\s+0x([0-9a-f]+)\s*$", re.M)
XREF_RE = re.compile(r"^From ([0-9a-f]+)(?: in (\S+))? \[(\w+)\]", re.M)
META_RE = re.compile(r"^<!-- evidence 1 (\{.*\}) -->$", re.M)


def eprint(*a):
    print(*a, file=sys.stderr, flush=True)


# --- transport ---------------------------------------------------------------

def _fetch(host, path, timeout):
    url = f"{host}/{path}"
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return r.read().decode("utf-8", "replace")


def discover_host(explicit, port):
    """127.0.0.1 first, then the WSL default gateway (Windows host under NAT)."""
    if explicit:
        return explicit.rstrip("/")
    cands = ["127.0.0.1"]
    try:
        out = subprocess.run(["ip", "route", "show", "default"],
                             capture_output=True, text=True, timeout=5).stdout
        parts = out.split()
        if "via" in parts:
            cands.append(parts[parts.index("via") + 1])
    except (OSError, subprocess.SubprocessError, ValueError, IndexError):
        pass
    for h in cands:
        base = f"http://{h}:{port}"
        try:
            if _fetch(base, "methods?offset=0&limit=1", 4).strip():
                return base
        except (urllib.error.URLError, OSError, TimeoutError):
            continue
    return None


class Ghidra:
    def __init__(self, host, timeout):
        self.host = host
        self.timeout = timeout
        self.calls = 0
        self.failures = 0

    def get(self, path, **params):
        q = urllib.parse.urlencode(params)
        self.calls += 1
        try:
            return _fetch(self.host, f"{path}?{q}" if q else path, self.timeout)
        except (urllib.error.URLError, OSError, TimeoutError) as e:
            self.failures += 1
            return f"[evidence.py: request failed: {e}]"


# --- parsing -----------------------------------------------------------------

def in_module(va):
    return MOD_LO <= va < MOD_HI


def parse_callees(asm):
    """Direct `CALL 0x...` targets, in first-seen order.

    Indirect calls (`CALL qword ptr [...]`) are deliberately not followed: the
    target is a runtime value and the address in the brackets is a slot, not a
    function.
    """
    seen, out = set(), []
    for _site, tgt in CALL_RE.findall(asm):
        va = int(tgt, 16)
        if in_module(va) and va not in seen:
            seen.add(va)
            out.append(va)
    return out


def parse_tailcalls(asm, lo, hi):
    """Unconditional JMPs that leave the function: tail calls and thunk stubs.

    This exe is full of 5-byte `jmp` thunks - cold-code layout puts a stub at
    0x1417xxxxx that jumps to the real body at 0x14cxxxxxx - so a walk that
    follows only CALL dead-ends at every one of them and never reaches the
    implementation. Jumps that stay inside [lo, hi] are the function's own
    branches and loops, and are not followed.
    """
    seen, out = set(), []
    for _site, tgt in JMP_RE.findall(asm):
        va = int(tgt, 16)
        if in_module(va) and not (lo <= va <= hi) and va not in seen:
            seen.add(va)
            out.append(va)
    return out


def parse_body(body):
    """`"1403856b0 - 14038595e"` -> (0x1403856b0, 0x14038595e); (0, 0) if unparsable."""
    try:
        lo, hi = (int(x.strip(), 16) for x in body.split("-", 1))
        return lo, hi
    except ValueError:
        return 0, 0


def parse_callers(xrefs):
    """(site, containing-function-name, kind) triples from an xrefs_to dump."""
    return [(int(s, 16), name or "", kind) for s, name, kind in XREF_RE.findall(xrefs)]


def entry_of(name):
    """A `FUN_<hex>` name encodes its own entry point. Others we cannot resolve."""
    m = FUN_RE.fullmatch(name or "")
    if not m:
        return None
    va = int(m.group(1), 16)
    return va if in_module(va) else None


def default_anchors():
    """Every in-module FUN_ address named in docs/ or already dumped in analysis/."""
    found = set()
    sources = [
        # Prose: every FUN_ we bothered to name in a findings or reference doc.
        ("docs/*.md", None),
        # Dumps: only the `// ==== FUN_x ====` headers. The FUN_ names inside a
        # decompiled body are its callees, and depth 1 reaches those anyway;
        # taking them as anchors makes 100 anchors into 600.
        ("analysis/*.c", re.compile(r"^// ==== (FUN_[0-9a-fA-F]+)", re.M)),
    ]
    for pat, header_re in sources:
        for p in sorted(ROOT.glob(pat)):
            try:
                text = p.read_text(errors="replace")
            except OSError:
                continue
            if header_re is not None:
                text = "\n".join(header_re.findall(text))
            for hx in FUN_RE.findall(text):
                va = int(hx, 16)
                if in_module(va):
                    found.add(va)
    return sorted(found)


def build_id():
    try:
        text = Path(APPMANIFEST).read_text(errors="replace")
    except OSError:
        return "unknown"
    m = re.search(r'"buildid"\s+"(\d+)"', text)
    return m.group(1) if m else "unknown"


# --- one function ------------------------------------------------------------

def clip(text, limit, what):
    """Truncate an over-long section, saying so in-band rather than silently."""
    if len(text) <= limit:
        return text
    return (text[:limit]
            + f"\n\n[evidence.py: {what} truncated at {limit} bytes "
              f"of {len(text)}; re-fetch with --max-section-bytes to get more]\n")


def fetch_function(g, va, caller_limit, max_body, max_section):
    a = f"{va:x}"
    info = g.get("get_function_by_address", address=a)
    if info.startswith("No function"):
        return None
    name = ""
    sig = ""
    body = ""
    for line in info.splitlines():
        if line.startswith("Function: "):
            name = line[len("Function: "):].split(" at ")[0].strip()
        elif line.startswith("Signature: "):
            sig = line[len("Signature: "):].strip()
        elif line.startswith("Body: "):
            body = line[len("Body: "):].strip()
    lo, hi = parse_body(body)
    if hi > lo and hi - lo > max_body:
        # Not a real function. Record it so the walk does not retry it, and
        # contribute nothing: its callee list is noise, not a call graph.
        return {
            "va": va, "name": name or f"FUN_{a}", "signature": sig, "body": body,
            "callees": [], "tailcalls": [], "callers": [],
            "callers_truncated": False, "oversized": hi - lo,
            "asm": "", "decompile": "",
        }
    asm = g.get("disassemble_function", address=a)
    dec = g.get("decompile_function_by_address", address=a)
    xrefs = g.get("xrefs_to", address=a, limit=caller_limit)
    callers = parse_callers(xrefs)
    return {
        "va": va, "name": name or f"FUN_{a}", "signature": sig, "body": body,
        "callees": parse_callees(asm),
        "tailcalls": parse_tailcalls(asm, lo, hi),
        "callers": [[s, n, k] for s, n, k in callers],
        "callers_truncated": len(callers) >= caller_limit,
        "oversized": 0,
        "asm": clip(asm, max_section, "disassembly"),
        "decompile": clip(dec, max_section, "decompilation"),
    }


def render(rec, depth, anchor_note):
    meta = meta_out(rec, depth)
    if rec.get("oversized"):
        return "\n".join([
            f"<!-- evidence 1 {json.dumps(meta, separators=(',', ':'))} -->",
            f"# {rec['name']} @ 0x{rec['va']:x} (SKIPPED)",
            "",
            f"Body `{rec['body']}` spans {rec['oversized']:,} bytes. That is not a",
            "function: Ghidra's auto-analysis glues runs of unanalysed bytes into one",
            "oversized entry. Nothing was exported and it contributes no call edges.",
            "Raise --max-body-bytes to export it anyway.",
            "",
        ])
    lines = [
        f"<!-- evidence 1 {json.dumps(meta, separators=(',', ':'))} -->",
        f"# {rec['name']} @ 0x{rec['va']:x}",
        "",
        f"- Signature: `{rec['signature'] or 'unknown'}`",
        f"- Body: `{rec['body'] or 'unknown'}`",
        f"- Reached at depth {depth}{anchor_note}",
        f"- {len(rec['callers'])} caller reference(s)"
        f"{' (TRUNCATED)' if rec['callers_truncated'] else ''},"
        f" {len(rec['callees'])} direct callee(s)",
        "", "## Callers", "",
    ]
    if rec["callers"]:
        for site, name, kind in rec["callers"]:
            lines.append(f"- `0x{site:x}` in `{name or '?'}` [{kind}]")
    else:
        lines.append("- none found")
    if rec["tailcalls"]:
        lines += ["", "## Tail calls / thunk targets", ""]
        lines += [f"- `0x{c:x}` (`FUN_{c:x}`)" for c in rec["tailcalls"]]
    lines += ["", "## Callees (direct calls only)", ""]
    if rec["callees"]:
        lines += [f"- `0x{c:x}` (`FUN_{c:x}`)" for c in rec["callees"]]
    else:
        lines.append("- none found")
    lines += ["", "## Decompilation", "", "```c", rec["decompile"].strip(), "```",
              "", "## Disassembly", "", "```asm", rec["asm"].strip(), "```", ""]
    return "\n".join(lines)


def meta_out(rec, depth):
    """The graph facts about one function, addresses as hex strings.

    This is what goes in the file's metadata line, so it has to stay readable
    and greppable: a decimal `5372401328` matches nothing anyone would search
    for.
    """
    return {
        "va": f"{rec['va']:x}",
        "name": rec["name"],
        "depth": depth,
        "callees": [f"{c:x}" for c in rec["callees"]],
        "tailcalls": [f"{c:x}" for c in rec["tailcalls"]],
        "callers": [[f"{site:x}", n, k] for site, n, k in rec["callers"]],
        "callers_truncated": rec["callers_truncated"],
        "oversized": rec.get("oversized", 0),
    }


def meta_in(obj):
    """Inverse of meta_out: hex strings back to ints for the walk."""
    return {
        "va": int(obj["va"], 16),
        "name": obj.get("name", ""),
        "depth": obj.get("depth"),
        "callees": [int(c, 16) for c in obj.get("callees") or []],
        "tailcalls": [int(c, 16) for c in obj.get("tailcalls") or []],
        "callers": [[int(site, 16), n, k] for site, n, k in obj.get("callers") or []],
        "callers_truncated": bool(obj.get("callers_truncated")),
        "oversized": int(obj.get("oversized") or 0),
    }


def read_meta(path):
    try:
        head = path.read_text(errors="replace")[:65536]
    except OSError:
        return None
    m = META_RE.search(head)
    if not m:
        return None
    try:
        return meta_in(json.loads(m.group(1)))
    except (json.JSONDecodeError, ValueError, TypeError, KeyError):
        return None


# --- the walk ----------------------------------------------------------------

def main():
    ap = argparse.ArgumentParser(
        description="Export a bounded Ghidra evidence tree around anchor functions.")
    ap.add_argument("--anchor", action="append", default=[], metavar="ADDR",
                    help="anchor address in hex (repeatable); default: every "
                         "FUN_ named in docs/ or analysis/")
    ap.add_argument("--anchors-file", metavar="PATH",
                    help="file of anchor addresses, one hex address per line "
                         "('#' comments and blank lines ignored); combines with "
                         "--anchor. Use it when the anchor set is a few hundred "
                         "addresses and will not fit on a command line.")
    ap.add_argument("--depth", type=int, default=1,
                    help="call-graph hops from the anchors (default 1)")
    ap.add_argument("--max-functions", type=int, default=1500,
                    help="hard cap on functions written (default 1500)")
    ap.add_argument("--max-callers-expand", type=int, default=40, metavar="N",
                    help="do not expand through a function with more than N "
                         "callers; its callers are still recorded (default 40). "
                         "This is what stops the walk drowning in hub functions "
                         "like operator new.")
    ap.add_argument("--max-body-bytes", type=lambda x: int(x, 0), default=0x100000,
                    metavar="N",
                    help="skip a function whose body spans more than N bytes "
                         "(default 0x100000). Ghidra's auto-analysis glues runs "
                         "of unanalysed bytes into single bogus 'functions' - one "
                         "here spans 222MB and reports 14994 callees - and both "
                         "its disassembly and its callee list poison the walk.")
    ap.add_argument("--max-section-bytes", type=lambda x: int(x, 0),
                    default=2 * 1024 * 1024, metavar="N",
                    help="truncate a decompilation or disassembly longer than N "
                         "bytes (default 2MiB). A backstop for the above.")
    ap.add_argument("--caller-limit", type=int, default=200,
                    help="max caller references fetched per function (default 200)")
    ap.add_argument("--jobs", type=int, default=4, help="parallel requests (default 4)")
    ap.add_argument("--timeout", type=int, default=180, help="per-request seconds")
    ap.add_argument("--out", default=str(ROOT / "evidence"), help="output directory")
    ap.add_argument("--host", default=os.environ.get("GHIDRA_MCP_HOST"),
                    help="e.g. http://172.23.144.1:8081 (default: probe)")
    ap.add_argument("--port", type=int,
                    default=int(os.environ.get("GHIDRA_MCP_PORT", "8081")))
    ap.add_argument("--force", action="store_true", help="re-fetch existing files")
    ap.add_argument("--dry-run", action="store_true",
                    help="print the anchor set and exit without contacting Ghidra")
    args = ap.parse_args()

    given = list(args.anchor)
    if args.anchors_file:
        try:
            for line in Path(args.anchors_file).read_text().splitlines():
                line = line.split("#", 1)[0].strip()
                if line:
                    given.append(line)
        except OSError as e:
            eprint(f"evidence: cannot read {args.anchors_file}: {e}")
            return 1
    try:
        anchors = [int(a, 16) for a in given] if given else default_anchors()
    except ValueError as e:
        eprint(f"evidence: bad anchor address: {e}")
        return 1
    anchors = [a for a in anchors if in_module(a)]
    if not anchors:
        eprint("evidence: no anchors (nothing in docs/ or analysis/, and none given)")
        return 1
    eprint(f"evidence: {len(anchors)} anchor(s), depth {args.depth}, "
           f"cap {args.max_functions}")
    if args.dry_run:
        for a in anchors:
            print(f"{a:x}")
        return 0

    host = discover_host(args.host, args.port)
    if not host:
        eprint(f"evidence: no GhidraMCP server answering on port {args.port} "
               f"(127.0.0.1 or the default gateway). Is Ghidra running?")
        return 1
    eprint(f"evidence: using {host}")

    out = Path(args.out)
    fndir = out / "fn"
    fndir.mkdir(parents=True, exist_ok=True)
    g = Ghidra(host, args.timeout)

    anchor_set = set(anchors)
    nodes = {}          # va -> {name, callees, callers, depth, truncated}
    frontier = list(dict.fromkeys(anchors))
    started = time.time()

    for depth in range(args.depth + 1):
        frontier = [va for va in frontier if va not in nodes]
        if not frontier:
            break
        room = args.max_functions - len(nodes)
        if room <= 0:
            eprint(f"evidence: cap of {args.max_functions} reached, stopping")
            break
        if len(frontier) > room:
            eprint(f"evidence: depth {depth} has {len(frontier)} functions, "
                   f"taking {room} to stay under the cap")
            frontier = frontier[:room]
        eprint(f"evidence: depth {depth}: {len(frontier)} function(s)")

        def work(va):
            path = fndir / f"{va:x}.md"
            if path.exists() and not args.force:
                meta = read_meta(path)
                if meta:
                    return va, meta, True
            rec = fetch_function(g, va, args.caller_limit,
                                 args.max_body_bytes, args.max_section_bytes)
            if rec is None:
                return va, None, False
            note = " (anchor)" if va in anchor_set else ""
            tmp = path.with_suffix(".md.tmp")
            tmp.write_text(render(rec, depth, note))
            tmp.replace(path)
            return va, meta_in(meta_out(rec, depth)), False

        done = 0
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as ex:
            for va, meta, cached in ex.map(work, frontier):
                done += 1
                if meta is None:
                    nodes[va] = None       # not a function; remember, do not retry
                    continue
                meta.setdefault("depth", depth)
                nodes[va] = meta
                if done % 25 == 0 or done == len(frontier):
                    eprint(f"  {done}/{len(frontier)} "
                           f"({g.calls} requests, {g.failures} failed)")

        if depth == args.depth:
            break
        nxt = []
        for va in frontier:
            meta = nodes.get(va)
            if not meta:
                continue
            callers = meta.get("callers") or []
            if len(callers) > args.max_callers_expand or meta.get("callers_truncated"):
                # A hub. Its callers are recorded in its own file; expanding
                # through them is what turns a walk into the whole program.
                caller_entries = []
            else:
                caller_entries = [e for e in (entry_of(c[1]) for c in callers) if e]
            reached = (list(meta.get("callees") or [])
                       + list(meta.get("tailcalls") or []))
            for t in reached + caller_entries:
                if t not in nodes:
                    nxt.append(t)
        frontier = list(dict.fromkeys(nxt))

    # --- index and graph ---
    # Built from every file in the tree, not just this run's walk: a targeted
    # top-up (--anchor / --anchors-file) would otherwise overwrite the index
    # with its handful of functions and lose the rest.
    real = {}
    for f in sorted(fndir.glob("*.md")):
        m = read_meta(f)
        if m:
            real[m["va"]] = m
    for va, m in nodes.items():           # this run wins on anything it refreshed
        if m:
            real[va] = m
    walked = sum(1 for m in nodes.values() if m)
    rows = ["\t".join(("address", "name", "depth", "callers", "callees", "anchor"))]
    for va in sorted(real):
        m = real[va]
        rows.append("\t".join((
            f"{va:x}", m.get("name", ""), str(m.get("depth", "")),
            str(len(m.get("callers") or [])) + ("+" if m.get("callers_truncated") else ""),
            str(len(m.get("callees") or [])),
            "yes" if va in anchor_set else "")))
    (out / "index.tsv").write_text("\n".join(rows) + "\n")

    edges = sorted({(f"{va:x}", f"{c:x}")
                    for va, m in real.items() for c in (m.get("callees") or [])})
    (out / "graph.json").write_text(json.dumps({
        "build": build_id(),
        "generated": time.strftime("%Y-%m-%d"),
        "anchors": [f"{a:x}" for a in anchors],
        "nodes": {f"{va:x}": real[va].get("name", "") for va in sorted(real)},
        "calls": [list(e) for e in edges],
    }, indent=1) + "\n")

    missing = sum(1 for m in nodes.values() if not m)
    (out / "README.md").write_text(f"""# evidence/

Generated by `tools/evidence.py` on {time.strftime("%Y-%m-%d")} against game
build **{build_id()}**. Not a commit artifact - regenerate it, do not edit it.

- `fn/<address>.md` - one file per function: signature, callers, direct
  callees, decompilation, disassembly.
- `index.tsv` - address, name, depth, caller/callee counts, anchor flag.
- `graph.json` - the call edges, for anything that wants the graph.

{len(real)} function(s) in the tree; this run walked {walked} from
{len(anchors)} anchor(s) at depth {args.depth}.

Ghidra's project is a fresh auto-analysis, so names are bare `FUN_`. What each
one *means* lives in `docs/reference-internals.md`; this tree is the raw
material that document is written from, and it goes stale on a game update.

Regenerate, and diff against the old tree to find what an update moved:

    python3 tools/evidence.py --depth {args.depth} --max-functions {args.max_functions}
""")

    eprint(f"evidence: {walked} function(s) this run, {len(real)} in {out} "
           f"({missing} address(es) were not functions, "
           f"{g.calls} requests, {g.failures} failed, "
           f"{time.time() - started:.0f}s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
