//! `retarget` against a synthetic PE, one case per subcommand.
//!
//! The real-exe numbers are pinned in `src/bin/retarget/expect.rs` and run
//! under `just tools-check-game`. These run everywhere: a hand-built PE32+
//! (`common/synth_pe.rs`) with a caller, a callee with a 13-byte
//! position-independent prologue, and one static-info access chain laid out
//! the way the 26636972 codegen lays it out (a getter whose slot load and
//! manager reads lead to a call into an accessor function, the head 0x31 into
//! that function, the `rbp` tail, the rbp-relative stream fill and the loader
//! call 0x7B past the accessor's call, into a loader opening with the 15-byte
//! prologue), so every subcommand has something real to find and every exit
//! code can be driven both ways. Through 26480954 the fixture was the older
//! template: the slot load 0x33 before the head in the accessor's own
//! function, the `r11` tail and the rsp-relative fill.

#[path = "common/synth_pe.rs"]
mod synth_pe;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use synth_pe::{rel32, SynthPe, RDATA};

const CALLER: u32 = 0x1000;
const CALLEE: u32 = 0x1100;
const GETTER: u32 = 0x1700;
const SLOT_LOAD: u32 = GETTER + 8;
/// The getter's `E8` into the accessor function.
const GETTER_CALL: u32 = GETTER + 52;
const ACCESSOR_FN: u32 = 0x1800;
const HEAD: u32 = ACCESSOR_FN + 0x31;
const SLOT: u32 = RDATA + 0x800;
const DEFAULT: u32 = RDATA + 0x810;
const NAME: u32 = RDATA + 0x100;
const ACCESSOR_CALL: u32 = HEAD + 18;
const LOADER_CALL: u32 = ACCESSOR_CALL + 0x7B;
const LOADER: u32 = 0x1A00;
/// `mov rax,[rbx]; mov [rbp-0x10],rax; mov eax,[rbx+0xc]; mov [rbp-0x8],eax;
/// mov eax,[rdx+0x4]; mov [rbp-0x4],eax`: the stream fill the `gimmickinfo`
/// accessor emits on 26636972, ending where the loader call starts. Against
/// the loader call's `lea r9,[rbp-0x30]` that is buffer +0x20, size +0x28,
/// cursor +0x2c.
const FILL: [u8; 19] = [
    0x48, 0x8B, 0x03, 0x48, 0x89, 0x45, 0xF0, 0x8B, 0x43, 0x0C, 0x89, 0x45, 0xF8, 0x8B, 0x42, 0x04, 0x89, 0x45, 0xFC,
];
/// `mov [rsp+0x20],rbx; mov [rsp+0x10],rdx; push rbp; push rsi`: the callee's
/// 12-byte prologue (the record loader's through 26480954).
const CALLEE_PROLOGUE: [u8; 12] = [0x48, 0x89, 0x5C, 0x24, 0x20, 0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56];
/// `mov [rsp+0x20],rbx; mov [rsp+0x10],rdx; mov [rsp+0x8],rcx`: the record
/// loader's prologue on 26636972 (`census::LOADER_PROLOGUE`).
const LOADER_PROLOGUE: [u8; 15] =
    [0x48, 0x89, 0x5C, 0x24, 0x20, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4C, 0x24, 0x08];

fn image() -> SynthPe {
    let mut pe = SynthPe::default();
    // sub rsp,0x28; call CALLEE; add rsp,0x28; ret
    let mut caller = vec![0x48, 0x83, 0xEC, 0x28, 0xE8];
    caller.extend_from_slice(&rel32(CALLER + 9, CALLEE));
    caller.extend_from_slice(&[0x48, 0x83, 0xC4, 0x28, 0xC3]);
    pe.code(CALLER, &caller).function(CALLER, CALLER + 0x40);

    // The 12-byte prologue plus `push rdi` (13 bytes, all safe), then
    // `lea rax,[rip+..]` (RIP-relative: the end of the safe counts), ret.
    let mut callee = CALLEE_PROLOGUE.to_vec();
    callee.push(0x57);
    callee.extend_from_slice(&[0x48, 0x8D, 0x05]);
    callee.extend_from_slice(&rel32(CALLEE + 20, RDATA));
    callee.push(0xC3);
    pe.code(CALLEE, &callee).function(CALLEE, CALLEE + 0x40);

    // The getter: sub rsp,0x28; movzx r8d,word [rcx]; mov rcx,[rip+SLOT];
    // cmp r8d,[rcx+0x28]; jb +12; mov rax,[rip+DEFAULT]; add rsp,0x28; ret;
    // mov rax,[rcx+0x20]; lea rdx,[r8+r8*2]; cmp qword [rax+rdx*8+8],0;
    // lea rdx,[rsp+0x30]; call ACCESSOR_FN; add rsp,0x28; ret
    let mut g = vec![0x48, 0x83, 0xEC, 0x28, 0x44, 0x0F, 0xB7, 0x01, 0x48, 0x8B, 0x0D];
    g.extend_from_slice(&rel32(SLOT_LOAD + 7, SLOT));
    g.extend_from_slice(&[0x44, 0x3B, 0x41, 0x28, 0x72, 0x0C, 0x48, 0x8B, 0x05]);
    g.extend_from_slice(&rel32(GETTER + 28, DEFAULT));
    g.extend_from_slice(&[0x48, 0x83, 0xC4, 0x28, 0xC3]);
    g.extend_from_slice(&[0x48, 0x8B, 0x41, 0x20, 0x4B, 0x8D, 0x14, 0x40, 0x48, 0x83, 0x7C, 0xD0, 0x08, 0x00]);
    g.extend_from_slice(&[0x48, 0x8D, 0x54, 0x24, 0x30, 0xE8]);
    assert_eq!(GETTER + g.len() as u32 - 1, GETTER_CALL);
    g.extend_from_slice(&rel32(GETTER_CALL + 5, ACCESSOR_FN));
    g.extend_from_slice(&[0x48, 0x83, 0xC4, 0x28, 0xC3]);
    pe.code(GETTER, &g).function(GETTER, GETTER + 0x80);

    // The accessor function (its start is the getter's call target; no
    // RUNTIME_FUNCTION of its own). The head:
    // xor r9d,r9d; lea r8,[rip+NAME]; mov rdx,[rcx+8]; lea rcx,[rbp+0x40]; call LOADER
    pe.code(ACCESSOR_FN, &[0x90; 0x31]);
    let mut head = vec![0x45, 0x33, 0xC9, 0x4C, 0x8D, 0x05];
    head.extend_from_slice(&rel32(HEAD + 10, NAME));
    head.extend_from_slice(&[0x48, 0x8B, 0x51, 0x08, 0x48, 0x8D, 0x4D, 0x40, 0xE8]);
    head.extend_from_slice(&rel32(ACCESSOR_CALL + 5, LOADER));
    pe.code(HEAD, &head);
    // lea r9,[rbp-0x30]; movzx r8d,r14w; mov rdx,rdi; mov rcx,rsi; call LOADER
    let mut call = vec![0x4C, 0x8D, 0x4D, 0xD0, 0x45, 0x0F, 0xB7, 0xC6, 0x48, 0x8B, 0xD7, 0x48, 0x8B, 0xCE, 0xE8];
    call.extend_from_slice(&rel32(LOADER_CALL + 19, LOADER));
    pe.code(LOADER_CALL - FILL.len() as u32, &FILL);
    pe.code(LOADER_CALL, &call);
    pe.code(LOADER, &LOADER_PROLOGUE).function(LOADER, LOADER + 0x20);
    pe.data(NAME, b"gimmickinfo\0");
    pe
}

struct Fixture {
    dir: tempfile::TempDir,
    exe: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let exe = image().write(dir.path(), "synth.exe");
        std::fs::write(dir.path().join("names.txt"), "# synthetic\nGimmickInfo\n").unwrap();
        Fixture { dir, exe }
    }

    fn file(&self, name: &str, text: &str) -> PathBuf {
        let p = self.dir.path().join(name);
        std::fs::write(&p, text).unwrap();
        p
    }

    fn names(&self) -> PathBuf {
        self.dir.path().join("names.txt")
    }

    fn run(&self, args: &[&str], build: &str) -> Output {
        let manifest = self.file("appmanifest.acf", &format!("\"AppState\"\n{{\n\t\"buildid\"\t\t\"{build}\"\n}}\n"));
        Command::new(env!("CARGO_BIN_EXE_retarget"))
            .args(args)
            .arg("--exe")
            .arg(&self.exe)
            .env("CD_APPMANIFEST", manifest)
            .output()
            .expect("cannot run retarget")
    }
}

fn text(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn p(path: &Path) -> &str {
    path.to_str().unwrap()
}

const GOOD_SIGS: &str = "# callee         - function start\n\
                         48 89 5C 24 20 48 89 54 24 10 55 56 57\n\
                         # caller         - function start\n\
                         48 83 EC 28 E8 ?? ?? ?? ?? 48 83 C4 28 C3\n";

/// The caller's signature with its last fixed byte changed: dead, with a
/// 12-byte prefix still hitting once at the caller.
const DEAD_SIG: &str = "# moved          - the caller, recompiled\n\
                        48 83 EC 28 E8 ?? ?? ?? ?? 48 83 C4 30 C3\n";

#[test]
fn triage_exits_zero_when_every_signature_hits_once() {
    let f = Fixture::new();
    let sigs = f.file("sigs.txt", GOOD_SIGS);
    let o = f.run(&["triage", "--sigs", p(&sigs)], "1");
    let s = text(&o);
    assert_eq!(o.status.code(), Some(0), "{s}");
    assert!(s.contains("callee"), "{s}");
    assert!(s.contains(&format!("+{CALLEE:#x} (file {:#x})", SynthPe::file_offset(CALLEE))), "{s}");
    assert!(s.contains("summary: 2 ok, 0 dead, 0 ambiguous"), "{s}");
}

#[test]
fn triage_exits_one_on_a_dead_required_signature_and_shows_what_is_left() {
    let f = Fixture::new();
    let sigs = f.file("sigs.txt", &format!("{GOOD_SIGS}{DEAD_SIG}"));
    let o = f.run(&["triage", "--sigs", p(&sigs)], "1");
    let s = text(&o);
    assert_eq!(o.status.code(), Some(1), "{s}");
    assert!(s.contains("DEAD"), "{s}");
    assert!(s.contains(&format!("longest prefix 12/14 bytes, 1 hit(s): +{CALLER:#x}")), "{s}");
    assert!(s.contains("summary: 2 ok, 1 dead, 0 ambiguous"), "{s}");

    // The same signature marked optional no longer fails the run.
    let optional = DEAD_SIG.replace("the caller, recompiled", "[optional] the caller, recompiled");
    let sigs = f.file("sigs2.txt", &format!("{GOOD_SIGS}{optional}"));
    let o = f.run(&["triage", "--sigs", p(&sigs)], "1");
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("dead (optional)"));

    // TSV: one header and one row per signature.
    let o = f.run(&["triage", "--tsv", "--sigs", p(&sigs)], "1");
    assert_eq!(text(&o).lines().count(), 4);
    assert!(text(&o).starts_with("name\tline\t"));
}

#[test]
fn calls_names_each_target_with_its_pdata_status_and_callers() {
    let f = Fixture::new();
    let o = f.run(&["calls", "1000", "10", "--callers"], "1");
    let s = text(&o);
    assert_eq!(o.status.code(), Some(0), "{s}");
    assert!(s.contains("1 rel32 call/jmp target(s)"), "{s}");
    assert!(s.contains(&format!("+0x1004 (+0x4) call -> +{CALLEE:#x}  pdata start, 1 call site(s) in 1 function(s)")), "{s}");
    assert!(s.contains("pattern 48 89 5C 24 20 48 89 54 24 10 55 56 57 48 8D 05  (1 hit(s) exe-wide)"), "{s}");
}

#[test]
fn callers_groups_references_by_enclosing_function() {
    let f = Fixture::new();
    let o = f.run(&["callers", "1100"], "1");
    let s = text(&o);
    assert_eq!(o.status.code(), Some(0), "{s}");
    assert!(s.contains(&format!("+{CALLER:#x} [pdata]  1 ref(s)")), "{s}");
    assert!(s.contains("call           at +0x1004 (+0x4)"), "{s}");
    assert!(s.contains("1 code reference(s) in 1 function(s) (1 from pdata, 0 by heuristic), 0 pointer cell(s)"), "{s}");
}

#[test]
fn prologue_lists_the_safe_counts_and_exits_two_on_an_unsafe_one() {
    let f = Fixture::new();
    let o = f.run(&["prologue", "1100", "0x18"], "1");
    let s = text(&o);
    assert_eq!(o.status.code(), Some(0), "{s}");
    assert!(s.contains("safe stolen counts (>= 12): 0xc (12), 0xd (13)"), "{s}");
    assert!(s.contains("RIP"), "the lea is flagged: {s}");

    let o = f.run(&["prologue", "1100", "--stolen", "12"], "1");
    assert_eq!(o.status.code(), Some(0));
    assert!(text(&o).contains("[0x48, 0x89, 0x5C, 0x24, 0x20, 0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56]"), "{}", text(&o));

    let o = f.run(&["prologue", "1100", "--stolen", "0xd", "--tokens"], "1");
    assert_eq!(o.status.code(), Some(0));
    assert!(text(&o).contains("0x57, // push rdi"), "{}", text(&o));

    // 11 is short of the 12-byte patch; 20 runs through the RIP-relative lea.
    for k in ["11", "20"] {
        let o = f.run(&["prologue", "1100", "--stolen", k], "1");
        assert_eq!(o.status.code(), Some(2), "--stolen {k}: {}", text(&o));
        assert!(text(&o).contains("is NOT safe"));
    }

    // A file offset is accepted with `f:` and printed as the RVA.
    let off = format!("f:{:x}", SynthPe::file_offset(CALLEE));
    let o = f.run(&["prologue", &off], "1");
    assert!(text(&o).starts_with(&format!("prologue at +{CALLEE:#x} (pdata start)")), "{}", text(&o));
}

#[test]
fn census_finds_the_planted_site_end_to_end() {
    let f = Fixture::new();
    let o = f.run(&["census", "--names", p(&f.names()), "--sites"], "1");
    let s = text(&o);
    assert_eq!(o.status.code(), Some(0), "{s}");
    assert!(s.contains("census: 1 getter(s), 1 accessor site(s), 1 loader call(s)"), "{s}");
    assert!(s.contains("registers mov rcx / cmp r8d: 1"), "{s}");
    assert!(s.contains("manager: count +0x28, entries +0x20, stride 24, object +0x8: 1"), "{s}");
    assert!(s.contains("slot load -> accessor call: 0x2c: 1"), "{s}");
    assert!(s.contains("prologue standalone 48 83 EC 28 44 0F B7 01: 1"), "{s}");
    assert!(s.contains("function start from a getter's call at 1, head offset into the function 0x31: 1"), "{s}");
    assert!(s.contains("tails: lea rcx,[rbp+d8]: 1"), "{s}");
    assert!(s.contains("0 accessor(s) without a getter; 0 with getters disagreeing on the slot; 1 distinct slot(s)"), "{s}");
    assert!(s.contains("loader calls: 1/1 found, 1 open with LOADER_PROLOGUE (15 bytes)"), "{s}");
    assert!(s.contains("distance from the head: 0x8d: 1"), "{s}");
    assert!(s.contains("0x7b..0x7b"), "{s}");
    assert!(s.contains(&format!("gimmickinfo: accessor function +{ACCESSOR_FN:#x}, head +{HEAD:#x} (+0x31 in)")), "{s}");
    assert!(s.contains(&format!("getter +{GETTER:#x} slot load +{SLOT_LOAD:#x}, slot +{SLOT:#x}, call +{GETTER_CALL:#x}")), "{s}");
    assert!(s.contains(&format!("-> loader +{LOADER:#x}, prologue = LOADER_PROLOGUE")), "{s}");
    assert!(s.contains("stream fill: buffer +0x20 size +0x28 cursor +0x2c at 1 of 1 sites\n"), "{s}");
    assert!(s.contains("    stream fill: buffer +0x20 size +0x28 cursor +0x2c\n"), "{s}");
    assert!(s.contains("fill 0x20/0x28/0x2c"), "the --sites column: {s}");
    assert!(!s.contains("NOT in the name list"), "{s}");

    // A list without the name: the site is reported as unknown.
    let other = f.file("other.txt", "ItemInfo\n");
    let s = text(&f.run(&["census", "--names", p(&other)], "1"));
    assert!(s.contains("1 site(s) pass a name not in the list"), "{s}");
}

#[test]
fn pe_prints_the_headers_and_the_exception_directory() {
    let f = Fixture::new();
    let o = f.run(&["pe"], "12345");
    let s = text(&o);
    assert_eq!(o.status.code(), Some(0), "{s}");
    assert!(s.contains("build (appmanifest) 12345"), "{s}");
    assert!(s.contains("image base          0x140000000"), "{s}");
    assert!(s.contains("SizeOfImage         0x5000"), "{s}");
    assert!(s.contains("timestamp           0x6ae58024"), "{s}");
    assert!(s.contains(".pdata"), "{s}");
    assert!(s.contains("3 exception     +0x4000 size 0x30  (.pdata)"), "{s}");
    assert!(s.contains("exception directory: VA 0x140004000 - 0x14000402f, 4 RUNTIME_FUNCTION entries (0 chained)"), "{s}");
    let sha = desert_tools::sha256::hex_digest(&std::fs::read(&f.exe).unwrap());
    assert!(s.contains(&sha), "{s}");
}

#[test]
fn report_exits_one_when_dirty_and_zero_when_clean() {
    let f = Fixture::new();
    let names = f.names();
    let dirty = f.file("dirty.txt", &format!("{GOOD_SIGS}{DEAD_SIG}"));
    let o = f.run(&["report", "--sigs", p(&dirty), "--names", p(&names)], "26636972");
    let s = text(&o);
    assert_eq!(o.status.code(), Some(1), "{s}");
    assert!(s.starts_with("# retarget report: build 26636972"), "{s}");
    assert!(s.contains("To re-derive: moved."), "{s}");
    assert!(s.contains("Verdict: work to do"), "{s}");

    let good = f.file("good.txt", GOOD_SIGS);
    let o = f.run(&["report", "--sigs", p(&good), "--names", p(&names)], "26636972");
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("Verdict: clean"));
    assert!(
        text(&o).contains("| `StreamLayout` | `hook.rs` (parsed at install), `gimmick_real.rs`, `expect.rs` | gimmickinfo buffer +0x20 size +0x28 cursor +0x2c;"),
        "{}",
        text(&o)
    );

    // A build that is not the calibrated one is never clean.
    let o = f.run(&["report", "--sigs", p(&good), "--names", p(&names)], "1");
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn a_bad_address_is_an_error() {
    let f = Fixture::new();
    for a in ["zz", "f:zz", "f:ffffff", "fffffff"] {
        let o = f.run(&["prologue", a], "1");
        assert_eq!(o.status.code(), Some(1), "{a}: {}", text(&o));
    }
}
