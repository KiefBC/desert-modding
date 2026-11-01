# Ghidra and analysis

Sources: `CLAUDE.md` (Ghidra, Analysis tooling), `tools/README.md`, `justfile`, `docs/findings/2026-09-24-build-25477059-retarget.md` §4 and §6. Evidence: `docs/reference-internals.md` §19 and §19.5.

## Setup

Ghidra runs **on Windows**, not in the dev shell: the GhidraMCP extension is version-matched to the Windows install. The extension is the upstream GhidraMCP release, in `%APPDATA%\ghidra\ghidra_<version>\Extensions\GhidraMCP`; its declared `ghidraVersion` is older than the installed Ghidra and it loads anyway. It binds the wildcard address, so WSL reaches it on the Windows host IP with no `.wslconfig` change. Do not install the nixpkgs build from `/etc/nixos/pkgs/ghidra-mcp.nix` on Windows: it is patched to loopback-only.

Claude Code reaches it through the `ghidra` MCP server: `ghidra-mcp-bridge`, a stdio process on the Linux side, speaks HTTP to the server inside Ghidra. The bridge must be in `environment.systemPackages` in `/etc/nixos/configuration.nix`, not only in a `let` binding, or every MCP launch dies on a failed `exec`. `~/.local/bin/ghidra-mcp-bridge-win` probes `127.0.0.1` then the default gateway (which changes across reboots under NAT) and execs the bridge against whichever answers.

**Ports.** GhidraMCP defaults to 8080 and the setup here uses 8081. It reads the port once, at plugin construction. If the port is taken it logs `BindException` to `%APPDATA%\ghidra\ghidra_<version>\application.log`, still reports "loaded!", and has no listener and no retry. Changing the port in Tool Options needs a Ghidra restart.

## The two failure modes

- **"Connection refused"** is the HTTP side. Check in order: Ghidra running; `application.log` says "HTTP server started on port N"; `curl http://<gateway>:N/segments` from WSL; the MCP client restarted since the last config change.
- **"Connection closed"** (`CONNECTION_CLOSED`) is the bridge dying at launch and says nothing about Ghidra. Check `command -v ghidra-mcp-bridge`, then drive the handshake by hand; a healthy bridge answers with a `serverInfo` naming `ghidra-mcp`:

```bash
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"probe","version":"0"}}}' | ~/.local/bin/ghidra-mcp-bridge-win
```

The client spawns the bridge only at startup, so a Nix-side fix needs a Claude Code restart. Until then `curl` against the HTTP API (`/methods`, `/searchFunctions`, `/segments`, `POST /decompile`) is a full stand-in.

## The project

The Windows project is a fresh auto-analysis: over 250k functions, almost all `FUN_`-named, no classes or namespaces. `docs/reference-internals.md` and `analysis/` are the only record of what the functions are.

- **`searchFunctions` cannot find a string constant.** It searches symbols, and there are none worth the name. `grep` over `evidence/` answers that question; it is how the 149-table inventory in `docs/reference-internals.md` §19 was found.
- **Every function has a phantom `[DATA]` xref.** It points into the PE exception directory: the function's own `RUNTIME_FUNCTION` record. Do not chase it (§19.5).
- **Derive the exception directory; never trust a section name.** Read data directory entry 3 of the optional header. Section names are scrambled per build: on 25246367 the directory was `.link`; on 25477059 it is all of `.trace`, RVA `0x170DE000`, size `0x2CC250` (retarget note §6).

```bash
python3 - <<'PY'
import struct
d = open("/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64/CrimsonDesert.exe","rb").read(0x800)
pe = struct.unpack_from("<I", d, 0x3c)[0]; opt = pe + 24
base = struct.unpack_from("<Q", d, opt + 24)[0]
rva, sz = struct.unpack_from("<II", d, opt + 112 + 3 * 8)
print(f"exception directory: 0x{base+rva:X} - 0x{base+rva+sz-1:X}")
PY
```

## Offline tools

All are Rust binaries in `tools/`, built by `just tools`; [Tools](../repo/tools/index.md) is the full manual.

- **Table source**, in order: `CD_DMM_TABLE`; else `bin64/DesertTooling.gimmickinfo.bin` from a `DumpTable=1` + `DryRun=1` launch; else DMM's backup copy, which DMM deletes at will (retarget note §4).
- **Pinned calibration**: `gen-collect-names` (`CALIBRATION`) and `items` (`EXPECT_*`) refuse a table that does not match their build's counts and anchors. After an update, re-pin; never override.
- `just evidence`: one Markdown file per function plus `index.tsv` and `graph.json`, exported over Ghidra's HTTP API. The walk has deliberate bounds (hub callers not expanded, tail calls followed, indirect calls not followed, a `--max-functions` cap, oversized entries stubbed); read the `evidence` section of the tools manual first. Resumable, and a failed fetch is cached as if done (a known defect).
- `xrefs <rva>`: rel32 calls and jumps, RIP-relative `lea`/`mov`, and pointer cells holding the VA. `--selfcheck` diffs against the reference loop. One REX instruction reads as two references.
- `dis <start> <end>`: disassemble an RVA range (add `0x140000000` for Ghidra's VA).
- `fieldnames`: field names of every static-info record class from the game's Korean per-field read-failure strings, into `analysis/fieldnames.json` (§19.9).
- `sigscan` prints **file offsets**, `xrefs` takes RVAs. Convert before passing one to the other.
