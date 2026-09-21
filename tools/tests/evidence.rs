//! End-to-end tests for `evidence`, against a fake GhidraMCP server.
//!
//! The real server is a Ghidra extension on a Windows box, so the interesting
//! half of this tool - which call edges the walk is allowed to follow - would
//! otherwise be untested on any machine that is not sitting in front of a
//! loaded project. The synthetic program below is small but is shaped around
//! the five deliberate properties of the walk that `tools/README.md` lists,
//! one function per property, so a regression in any of them fails a test here
//! rather than silently producing a 9 GB tree six months from now.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const EXE: &str = env!("CARGO_BIN_EXE_evidence");

// --- the fake server ---------------------------------------------------------

/// A GhidraMCP-shaped HTTP server on a loopback port, counting its requests.
struct Fake {
    port: u16,
    requests: Arc<AtomicUsize>,
}

impl Fake {
    fn start() -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let port = listener.local_addr().expect("a bound address").port();
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&requests);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let counter = Arc::clone(&counter);
                std::thread::spawn(move || serve(stream, &counter));
            }
        });
        Fake { port, requests }
    }

    fn host(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::Relaxed)
    }

    fn reset(&self) {
        self.requests.store(0, Ordering::Relaxed);
    }
}

fn serve(mut stream: TcpStream, counter: &AtomicUsize) {
    let mut reader = BufReader::new(stream.try_clone().expect("a cloned socket"));
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.is_empty() {
        return;
    }
    // Drain the headers; every request this tool makes is a bodiless GET.
    loop {
        let mut h = String::new();
        match reader.read_line(&mut h) {
            Ok(0) => break,
            Ok(_) if h == "\r\n" || h == "\n" => break,
            Ok(_) => {}
            Err(_) => return,
        }
    }
    counter.fetch_add(1, Ordering::Relaxed);
    let target = line.split_whitespace().nth(1).unwrap_or("/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let params: HashMap<&str, &str> =
        query.split('&').filter_map(|kv| kv.split_once('=')).collect();
    let addr = params.get("address").copied().unwrap_or("");
    let body = match path.trim_start_matches('/') {
        "methods" => "FUN_140100000\nFUN_140200000\n".to_string(),
        "get_function_by_address" => match program(addr) {
            Some(f) => format!(
                "Function: {} at 0x{addr}\nSignature: {}\nBody: {}\n",
                f.name, f.signature, f.body
            ),
            None => format!("No function at address {addr}\n"),
        },
        "disassemble_function" => program(addr).map_or_else(
            || format!("No function at address {addr}\n"),
            |f| f.asm.to_string(),
        ),
        "decompile_function_by_address" => program(addr).map_or_else(
            || format!("No function at address {addr}\n"),
            |f| f.decompile.to_string(),
        ),
        "xrefs_to" => {
            let limit: usize = params.get("limit").and_then(|s| s.parse().ok()).unwrap_or(100);
            let xs = program(addr).map(|f| f.callers).unwrap_or_default();
            let text: String = xs
                .iter()
                .take(limit)
                .map(|(site, name, kind)| {
                    if name.is_empty() {
                        format!("From {site} [{kind}]\n")
                    } else {
                        format!("From {site} in {name} [{kind}]\n")
                    }
                })
                .collect();
            if text.is_empty() {
                "No references found\n".to_string()
            } else {
                text
            }
        }
        _ => {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
            return;
        }
    };
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
}

struct Fun {
    name: &'static str,
    signature: &'static str,
    body: &'static str,
    asm: &'static str,
    decompile: &'static str,
    callers: Vec<(&'static str, &'static str, &'static str)>,
}

/// The synthetic program. Every entry exists to exercise one rule of the walk.
fn program(addr: &str) -> Option<Fun> {
    let leaf = |va: &'static str, callers: Vec<(&'static str, &'static str, &'static str)>| Fun {
        name: Box::leak(format!("FUN_{va}").into_boxed_str()),
        signature: Box::leak(format!("void FUN_{va}(void)").into_boxed_str()),
        body: Box::leak(format!("{va} - {}f", &va[..va.len() - 1]).into_boxed_str()),
        asm: Box::leak(format!("{va}: PUSH RBP\n{}1: RET\n", &va[..va.len() - 1]).into_boxed_str()),
        decompile: Box::leak(
            format!("void FUN_{va}(void)\n\n{{\n  return;\n}}\n").into_boxed_str(),
        ),
        callers,
    };
    Some(match addr {
        // The anchor. One direct call repeated (de-duplicated), two indirect
        // calls (never followed), a JMP inside its own body (a loop, not a tail
        // call), and a CALL and a JMP to 0x18xxxxxxx (outside the game module).
        "140100000" => Fun {
            name: "FUN_140100000",
            signature: "undefined8 FUN_140100000(longlong param_1)",
            body: "140100000 - 1401000ff",
            asm: "140100000: PUSH RBP\n\
                  140100004: CALL 0x140200000\n\
                  140100009: CALL qword ptr [0x140900000]\n\
                  14010000f: CALL qword ptr [RAX + 0x18]\n\
                  140100013: CALL 0x140200000\n\
                  140100018: CALL 0x141700000\n\
                  14010001d: CALL 0x140300000\n\
                  140100022: CALL 0x140310000\n\
                  140100027: CALL 0x140400000\n\
                  14010002c: CALL 0x140500000\n\
                  140100031: CALL 0x180001000\n\
                  140100036: JMP 0x140100040\n\
                  14010003b: JMP 0x180002000\n\
                  140100040: RET\n",
            decompile: "undefined8 FUN_140100000(longlong param_1)\n\n{\n  return 0;\n}\n",
            callers: vec![
                ("140a00010", "FUN_140a00000", "CALL"),
                ("140a10010", "FUN_140a10000", "CALL"),
            ],
        },
        // A 5-byte thunk whose only instruction jumps to the real body. Without
        // tail-call following the walk stops dead here.
        "141700000" => Fun {
            name: "FUN_141700000",
            signature: "void FUN_141700000(void)",
            body: "141700000 - 141700004",
            asm: "141700000: JMP 0x14c000000\n",
            decompile: "void FUN_141700000(void)\n\n{\n  FUN_14c000000();\n  return;\n}\n",
            callers: vec![("140100018", "FUN_140100000", "CALL")],
        },
        // Reachable only through that thunk.
        "14c000000" => leaf("14c000000", vec![("141700000", "FUN_141700000", "JUMP")]),
        // Three callers: one resolvable FUN_, one with no containing function,
        // one whose FUN_ address is outside the module. Only the first expands.
        "140200000" => Fun {
            name: "FUN_140200000",
            signature: "void FUN_140200000(void)",
            body: "140200000 - 14020003f",
            asm: "140200000: CALL 0x140210000\n140200005: RET\n",
            decompile: "void FUN_140200000(void)\n\n{\n  FUN_140210000();\n  return;\n}\n",
            callers: vec![
                ("140600010", "FUN_140600000", "CALL"),
                ("140700010", "", "CALL"),
                ("180001010", "FUN_180001000", "CALL"),
            ],
        },
        // A hub: more callers than --max-callers-expand. Its callers are
        // recorded and none of them are expanded; its callee still is.
        "140300000" => Fun {
            name: "FUN_140300000",
            signature: "void FUN_140300000(void)",
            body: "140300000 - 14030003f",
            asm: "140300000: CALL 0x140320000\n140300005: RET\n",
            decompile: "void FUN_140300000(void)\n\n{\n  FUN_140320000();\n  return;\n}\n",
            callers: vec![
                ("140b00010", "FUN_140b00000", "CALL"),
                ("140b10010", "FUN_140b10000", "CALL"),
                ("140b20010", "FUN_140b20000", "CALL"),
                ("140b30010", "FUN_140b30000", "CALL"),
            ],
        },
        // Six callers, so any --caller-limit at or below six comes back full:
        // the truncated case, which must also contribute no frontier.
        "140310000" => Fun {
            name: "FUN_140310000",
            signature: "void FUN_140310000(void)",
            body: "140310000 - 14031000f",
            asm: "140310000: RET\n",
            decompile: "void FUN_140310000(void)\n\n{\n  return;\n}\n",
            callers: (0..6)
                .map(|n| {
                    let site: &'static str = Box::leak(format!("140c{n}0010").into_boxed_str());
                    let name: &'static str = Box::leak(format!("FUN_140c{n}0000").into_boxed_str());
                    (site, name, "CALL")
                })
                .collect(),
        },
        // 0x10000000 bytes of "function": the oversized stub. Its asm and
        // decompilation must never be requested at all.
        "140400000" => Fun {
            name: "FUN_140400000",
            signature: "undefined FUN_140400000()",
            body: "140400000 - 150400000",
            asm: "NEVER REQUESTED\n",
            decompile: "NEVER REQUESTED\n",
            callers: vec![],
        },
        "140210000" => leaf("140210000", vec![("140200000", "FUN_140200000", "CALL")]),
        "140320000" => leaf("140320000", vec![("140300000", "FUN_140300000", "CALL")]),
        "140600000" => leaf("140600000", vec![]),
        "140a00000" => leaf("140a00000", vec![("140100000", "FUN_140100000", "CALL")]),
        "140a10000" => leaf("140a10000", vec![("140100000", "FUN_140100000", "CALL")]),
        // 140500000 is deliberately absent: an address that is not a function.
        _ => return None,
    })
}

// --- harness -----------------------------------------------------------------

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(EXE)
        .args(args)
        .current_dir(dir)
        .env("CD_APPMANIFEST", dir.join("appmanifest.acf"))
        .output()
        .expect("evidence runs")
}

fn walk(fake: &Fake, out: &Path, extra: &[&str]) -> Output {
    let host = fake.host();
    let mut args: Vec<String> = vec![
        "--anchor".into(),
        "140100000".into(),
        "--depth".into(),
        "2".into(),
        "--max-callers-expand".into(),
        "3".into(),
        "--caller-limit".into(),
        "5".into(),
        "--max-functions".into(),
        "12".into(),
        "--host".into(),
        host,
        "--out".into(),
        out.display().to_string(),
    ];
    args.extend(extra.iter().map(|s| (*s).to_string()));
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run(out.parent().unwrap_or(out), &refs)
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("reading {}: {e}", p.display()))
}

fn tree_files(out: &Path) -> Vec<(String, String)> {
    let mut all = Vec::new();
    for name in ["index.tsv", "graph.json", "README.md"] {
        all.push((name.to_string(), read(&out.join(name))));
    }
    let mut fns: Vec<PathBuf> = std::fs::read_dir(out.join("fn"))
        .expect("an fn/ directory")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    fns.sort();
    for p in fns {
        all.push((
            p.file_name().expect("a file name").to_string_lossy().into_owned(),
            read(&p),
        ));
    }
    all
}

// --- the walk ----------------------------------------------------------------

#[test]
fn walk_records_every_deliberate_property() {
    let fake = Fake::start();
    let tmp = tempfile::tempdir().expect("a temp dir");
    let out = tmp.path().join("evidence");
    let o = walk(&fake, &out, &[]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let err = String::from_utf8_lossy(&o.stderr);

    // --max-functions is a hard cap and the run says so when it truncates.
    assert!(
        err.contains("evidence: depth 2 has 4 functions, taking 3 to stay under the cap"),
        "{err}"
    );
    assert!(err.contains("(1 address(es) were not functions"), "{err}");

    let index = read(&out.join("index.tsv"));
    // Indirect calls are not followed, the repeated direct call is counted
    // once, the out-of-module call is dropped: six callees, not nine.
    assert!(index.contains("140100000\tFUN_140100000\t0\t2\t6\tyes\n"), "{index}");
    // The intra-function JMP produced no tail call and no file for 140100040.
    assert!(!out.join("fn/140100040.md").exists());
    // The tail call did: 14c000000 is reachable no other way.
    assert!(out.join("fn/14c000000.md").exists(), "{index}");
    let thunk = read(&out.join("fn/141700000.md"));
    assert!(thunk.contains("## Tail calls / thunk targets"), "{thunk}");
    assert!(thunk.contains("- `0x14c000000` (`FUN_14c000000`)"), "{thunk}");

    // A hub is recorded but contributes no frontier: its four callers all
    // exist in the program and none of them were walked.
    for hub_caller in ["140b00000", "140b10000", "140b20000", "140b30000"] {
        assert!(!index.contains(hub_caller), "hub caller expanded: {index}");
    }
    // Its callee still is.
    assert!(read(&out.join("graph.json")).contains("\"140320000\""), "hub callee lost");

    // A truncated caller list is marked and expands nothing.
    assert!(index.contains("140310000\tFUN_140310000\t1\t5+\t0\t\n"), "{index}");
    for truncated_caller in ["140c00000", "140c10000"] {
        assert!(!index.contains(truncated_caller), "truncated caller expanded: {index}");
    }

    // The oversized entry gets the one-paragraph stub and no call edges, and
    // cost exactly one request rather than a 200 MB disassembly.
    let over = read(&out.join("fn/140400000.md"));
    assert!(over.contains("# FUN_140400000 @ 0x140400000 (SKIPPED)"), "{over}");
    assert!(over.contains("spans 268,435,456 bytes"), "{over}");
    assert!(!over.contains("NEVER REQUESTED"), "{over}");
    assert!(index.contains("140400000\tFUN_140400000\t1\t0\t0\t\n"), "{index}");

    // A caller with no containing function, and one outside the module, are
    // both dropped; the resolvable one is walked.
    assert!(index.contains("140600000"), "{index}");
    assert!(!index.contains("180001000"), "{index}");
}

#[test]
fn oversized_and_absent_addresses_cost_one_request_each() {
    let fake = Fake::start();
    let tmp = tempfile::tempdir().expect("a temp dir");
    let out = tmp.path().join("evidence");
    // Two addresses, one oversized and one not a function: four requests would
    // mean the body check ran too late, or the "No function" reply was ignored.
    let o = run(
        tmp.path(),
        &[
            "--anchor", "140400000",
            "--anchor", "140500000",
            "--depth", "0",
            "--host", &fake.host(),
            "--out", &out.display().to_string(),
        ],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(fake.requests(), 2);
    assert!(out.join("fn/140400000.md").exists());
    assert!(!out.join("fn/140500000.md").exists());
}

// --- resume ------------------------------------------------------------------

#[test]
fn a_second_run_refetches_nothing_and_force_refetches_everything() {
    let fake = Fake::start();
    let tmp = tempfile::tempdir().expect("a temp dir");
    let out = tmp.path().join("evidence");
    assert!(walk(&fake, &out, &[]).status.success());
    let first = tree_files(&out);
    let full = fake.requests();
    assert!(full > 30, "the first run should do real work, did {full}");

    fake.reset();
    assert!(walk(&fake, &out, &[]).status.success());
    // One request, not none: 140500000 has no file because it is not a
    // function, so it is asked about again every run.
    assert_eq!(fake.requests(), 1, "a resumed run re-fetched functions");
    assert_eq!(first, tree_files(&out), "a resumed run changed the tree");

    fake.reset();
    assert!(walk(&fake, &out, &["--force"]).status.success());
    assert_eq!(fake.requests(), full, "--force did not re-fetch");
    assert_eq!(first, tree_files(&out), "--force changed the tree");
}

#[test]
fn a_corrupt_metadata_line_is_refetched_not_trusted() {
    let broken = [
        // Unparsable JSON.
        r#"<!-- evidence 1 {"va":"140210000", -->"#,
        // No `va` at all.
        r#"<!-- evidence 1 {"name":"X","depth":9} -->"#,
        // `va` present but not a hex string.
        r#"<!-- evidence 1 {"va":"zzz","depth":9} -->"#,
        // `va` as a number rather than the hex string the format uses.
        r#"<!-- evidence 1 {"va":5369823232,"depth":9} -->"#,
        // A caller triple that is a pair.
        r#"<!-- evidence 1 {"va":"140210000","callers":[["1402",""]]} -->"#,
        // No metadata line at all.
        "# hand-written notes",
    ];
    for line in broken {
        let fake = Fake::start();
        let tmp = tempfile::tempdir().expect("a temp dir");
        let out = tmp.path().join("evidence");
        assert!(walk(&fake, &out, &[]).status.success());
        let good = read(&out.join("fn/140210000.md"));

        let mut text = good.clone();
        let end = text.find('\n').expect("a first line");
        text.replace_range(..end, line);
        std::fs::write(out.join("fn/140210000.md"), &text).expect("writing the corrupt file");

        fake.reset();
        assert!(walk(&fake, &out, &[]).status.success());
        // One request for the address that is not a function, four to rebuild
        // the file whose line could not be read.
        assert_eq!(fake.requests(), 5, "not re-fetched after corruption: {line}");
        assert_eq!(read(&out.join("fn/140210000.md")), good, "not repaired: {line}");
    }
}

#[test]
fn a_metadata_line_without_a_depth_is_kept_and_indexed_as_none() {
    // Not a case this tool can produce - only a hand-edited line gets here -
    // but the Python indexed it as the literal `None` and a tree topped up by
    // either implementation has to agree about what is already exported.
    let fake = Fake::start();
    let tmp = tempfile::tempdir().expect("a temp dir");
    let out = tmp.path().join("evidence");
    assert!(walk(&fake, &out, &[]).status.success());

    let path = out.join("fn/140210000.md");
    let mut text = read(&path);
    let end = text.find('\n').expect("a first line");
    text.replace_range(
        ..end,
        r#"<!-- evidence 1 {"va":"140210000","name":"FUN_140210000","callees":[],"tailcalls":[],"callers":[],"callers_truncated":false,"oversized":0} -->"#,
    );
    std::fs::write(&path, &text).expect("writing the file");

    fake.reset();
    assert!(walk(&fake, &out, &[]).status.success());
    assert_eq!(fake.requests(), 1, "a depth-less but valid line was re-fetched");
    let index = read(&out.join("index.tsv"));
    assert!(index.contains("140210000\tFUN_140210000\tNone\t0\t0\t\n"), "{index}");
}

// --- the index and the graph -------------------------------------------------

#[test]
fn depth_zero_rebuilds_the_index_from_the_whole_tree_without_asking_ghidra() {
    let fake = Fake::start();
    let tmp = tempfile::tempdir().expect("a temp dir");
    let out = tmp.path().join("evidence");
    assert!(walk(&fake, &out, &[]).status.success());
    let rows = read(&out.join("index.tsv")).lines().count();
    assert_eq!(rows, 12, "header plus eleven functions");

    std::fs::remove_file(out.join("index.tsv")).expect("removing the index");
    std::fs::remove_file(out.join("graph.json")).expect("removing the graph");
    fake.reset();
    // A dead port: a targeted top-up of a function already on disk must not
    // contact Ghidra at all, which is what makes this a cheap regeneration.
    let o = run(
        tmp.path(),
        &[
            "--anchor", "140210000",
            "--depth", "0",
            "--host", "http://127.0.0.1:1",
            "--out", &out.display().to_string(),
        ],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(fake.requests(), 0);

    let index = read(&out.join("index.tsv"));
    assert_eq!(index.lines().count(), 12, "a top-up truncated the index: {index}");
    // The anchor column follows this run, the rows do not.
    assert!(index.contains("140210000\tFUN_140210000\t2\t1\t0\tyes\n"), "{index}");
    assert!(index.contains("140100000\tFUN_140100000\t0\t2\t6\t\n"), "{index}");
    let graph = read(&out.join("graph.json"));
    assert!(graph.contains("\"anchors\": [\n  \"140210000\"\n ]"), "{graph}");
    assert!(graph.contains("\"14c000000\": \"FUN_14c000000\""), "{graph}");

    // And a file that is gone drops out of both.
    std::fs::remove_file(out.join("fn/14c000000.md")).expect("removing a function file");
    assert!(run(
        tmp.path(),
        &[
            "--anchor", "140210000",
            "--depth", "0",
            "--host", "http://127.0.0.1:1",
            "--out", &out.display().to_string(),
        ],
    )
    .status
    .success());
    assert_eq!(read(&out.join("index.tsv")).lines().count(), 11);
    assert!(!read(&out.join("graph.json")).contains("\"14c000000\": "));
}

// --- anchors -----------------------------------------------------------------

/// A throwaway checkout: `repo_root()` looks for justfile + flake.nix.
fn fake_repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("a temp dir");
    let root = tmp.path();
    std::fs::write(root.join("justfile"), "").expect("writing justfile");
    std::fs::write(root.join("flake.nix"), "").expect("writing flake.nix");
    std::fs::create_dir(root.join("docs")).expect("creating docs");
    std::fs::create_dir(root.join("analysis")).expect("creating analysis");
    std::fs::write(
        root.join("docs/findings.md"),
        "FUN_140385000 is the loader; see also FUN_1404f0000.\n\
         FUN_180001000 is CDLoot.asi and must not become an anchor.\n\
         FUN_1234 is too short to be an address.\n",
    )
    .expect("writing a doc");
    std::fs::write(
        root.join("docs/other.md"),
        "FUN_140385000 again, plus FUN_1402d1c40.\n",
    )
    .expect("writing a doc");
    // A dated record one level down: docs/findings/ and docs/archive/ are
    // where the session records live, and they must still feed the anchors.
    std::fs::create_dir(root.join("docs/findings")).expect("creating docs/findings");
    std::fs::write(
        root.join("docs/findings/2026-09-12-sub.md"),
        "FUN_1405ab000 is named only here.\n",
    )
    .expect("writing a nested doc");
    std::fs::write(
        root.join("analysis/dump.c"),
        "// ==== FUN_1406aa000 ====\n\
         void FUN_1406aa000(void) { FUN_140999000(); }\n\
         // ==== FUN_180002000 ====\n\
         void f(void) {}\n",
    )
    .expect("writing a dump");
    // A file with the wrong extension in each directory, to prove the globs.
    std::fs::write(root.join("docs/notes.txt"), "FUN_140fff000\n").expect("writing a stray");
    std::fs::write(root.join("analysis/stray.h"), "FUN_140eee000\n").expect("writing a stray");
    tmp
}

#[test]
fn default_anchors_come_from_docs_and_analysis_headers_only() {
    let repo = fake_repo();
    let o = run(repo.path(), &["--dry-run"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(
        String::from_utf8_lossy(&o.stdout),
        // Sorted, de-duplicated, in-module only.
        "1402d1c40\n140385000\n1404f0000\n1405ab000\n1406aa000\n"
    );
    // FUN_140999000 is a callee inside a decompiled body, not a header: taking
    // those as anchors is what turned 100 anchors into 600.
    assert!(!String::from_utf8_lossy(&o.stdout).contains("140999000"));
}

#[test]
fn the_anchor_set_does_not_depend_on_the_working_directory() {
    // `just evidence` runs this from the repo root and a session runs it from
    // wherever it happens to be; the corpus is found through repo_root().
    let repo = fake_repo();
    let from_root = run(repo.path(), &["--dry-run"]);
    let from_docs = run(&repo.path().join("docs"), &["--dry-run"]);
    let from_analysis = run(&repo.path().join("analysis"), &["--dry-run"]);
    assert_eq!(from_root.stdout, from_docs.stdout);
    assert_eq!(from_root.stdout, from_analysis.stdout);
    assert_eq!(from_root.stderr, from_docs.stderr);
}

#[test]
fn explicit_anchors_replace_the_corpus_and_keep_their_order_and_repeats() {
    let repo = fake_repo();
    let o = run(
        repo.path(),
        &["--dry-run", "--anchor", "1404f0000", "--anchor", "140385000", "--anchor", "1404f0000"],
    );
    assert!(o.status.success());
    // Not sorted and not de-duplicated: only the frontier is, and graph.json
    // records the anchors as given.
    assert_eq!(String::from_utf8_lossy(&o.stdout), "1404f0000\n140385000\n1404f0000\n");
    assert!(String::from_utf8_lossy(&o.stderr).starts_with("evidence: 3 anchor(s), depth 1"));
}

#[test]
fn an_anchors_file_combines_with_anchor_and_ignores_comments() {
    let repo = fake_repo();
    let list = repo.path().join("anchors.txt");
    std::fs::write(
        &list,
        "# the loader\n\
         140385000\n\
         \n\
         \t1404f0000\t# with a trailing comment\n\
         180001000\n",
    )
    .expect("writing the anchor list");
    let o = run(
        repo.path(),
        &["--dry-run", "--anchor", "1402d1c40", "--anchors-file", &list.display().to_string()],
    );
    assert!(o.status.success());
    // --anchor first, then the file, out-of-module dropped.
    assert_eq!(String::from_utf8_lossy(&o.stdout), "1402d1c40\n140385000\n1404f0000\n");
}

#[test]
fn out_of_module_anchors_alone_are_an_error_and_a_bad_one_is_rejected() {
    let repo = fake_repo();
    let o = run(repo.path(), &["--dry-run", "--anchor", "180001000"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("evidence: no anchors"));

    let o = run(repo.path(), &["--dry-run", "--anchor", "nothex"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("bad anchor address"));

    let o = run(repo.path(), &["--dry-run", "--anchors-file", "/nonexistent/list.txt"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("cannot read"));
}

#[test]
fn no_server_answering_is_an_error_not_an_empty_tree() {
    let repo = fake_repo();
    let out = repo.path().join("evidence");
    // Port 1 on loopback, and the gateway probe after it, both refuse.
    let o = run(repo.path(), &["--anchor", "140385000", "--port", "1", "--out", &out.display().to_string()]);
    assert_eq!(o.status.code(), Some(1));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("no GhidraMCP server answering on port 1"), "{err}");
    assert!(err.contains("Is Ghidra running?"), "{err}");
    assert!(!out.exists(), "a failed probe created the output directory");
}

#[test]
fn the_probe_finds_a_server_on_loopback_without_an_explicit_host() {
    let fake = Fake::start();
    let repo = fake_repo();
    let out = repo.path().join("evidence");
    let o = run(
        repo.path(),
        &[
            "--anchor", "140100000",
            "--depth", "0",
            "--port", &fake.port.to_string(),
            "--out", &out.display().to_string(),
        ],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains(&format!("evidence: using http://127.0.0.1:{}", fake.port)), "{err}");
    // The probe itself is a request, and is not counted in the run's own total.
    assert_eq!(fake.requests(), 5);
    assert!(err.contains("4 requests, 0 failed"), "{err}");
}

// --- clipping ----------------------------------------------------------------

#[test]
fn an_over_long_section_is_clipped_in_band() {
    let fake = Fake::start();
    let tmp = tempfile::tempdir().expect("a temp dir");
    let out = tmp.path().join("evidence");
    let o = run(
        tmp.path(),
        &[
            "--anchor", "140100000",
            "--depth", "0",
            "--max-section-bytes", "100",
            "--host", &fake.host(),
            "--out", &out.display().to_string(),
        ],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let text = read(&out.join("fn/140100000.md"));
    assert!(
        text.contains("[evidence: disassembly truncated at 100 bytes of 392; \
                       re-fetch with --max-section-bytes to get more]"),
        "{text}"
    );
    // Clipping the text does not clip the graph: the callees were parsed from
    // the full response before the section was cut.
    assert!(read(&out.join("index.tsv")).contains("\t6\tyes\n"), "callees lost to clipping");
}

#[test]
fn both_spellings_of_a_byte_limit_are_accepted() {
    let fake = Fake::start();
    let tmp = tempfile::tempdir().expect("a temp dir");
    for (n, flag) in ["0x20000000", "536870912"].iter().enumerate() {
        let out = tmp.path().join(format!("evidence{n}"));
        let o = run(
            tmp.path(),
            &[
                "--anchor", "140400000",
                "--depth", "0",
                "--max-body-bytes", flag,
                "--host", &fake.host(),
                "--out", &out.display().to_string(),
            ],
        );
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        // Above the cap the oversized entry exports normally.
        assert!(!read(&out.join("fn/140400000.md")).contains("(SKIPPED)"));
    }
}

#[test]
fn a_negative_depth_walks_nothing_and_only_rebuilds() {
    let fake = Fake::start();
    let tmp = tempfile::tempdir().expect("a temp dir");
    let out = tmp.path().join("evidence");
    assert!(walk(&fake, &out, &[]).status.success());

    std::fs::remove_file(out.join("index.tsv")).expect("removing the index");
    fake.reset();
    // argparse took --depth as a plain int, so a negative one parses and makes
    // the level loop empty. Nothing is fetched, everything is rebuilt.
    let o = run(
        tmp.path(),
        &[
            "--anchor", "140100000",
            "--depth", "-1",
            "--host", &fake.host(),
            "--out", &out.display().to_string(),
        ],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(fake.requests(), 0);
    assert_eq!(read(&out.join("index.tsv")).lines().count(), 12);
    assert!(String::from_utf8_lossy(&o.stderr).contains("0 function(s) this run, 11 in"));
}
