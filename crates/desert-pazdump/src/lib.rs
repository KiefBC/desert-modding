//! Desert PazDump - a reverse-engineering aid, not a shipped mod.
//!
//! Hooks the archive decode switch of build 26480954 (RVA `0x15890B0`; `0x1588890`
//! on 26423880, `0x1587380` on 26361232: the function that decrypts and/or
//! LZ4-decompresses one `.paz` entry after it has been read from disk) and
//! writes every ENCRYPTED entry it sees to
//! `bin64/pazdump/`: the 12-byte decode header the game hands it (kind byte,
//! three flag bytes, the 4-byte key, the declared size), the ciphertext as
//! read from disk, the same buffer after the in-place decrypt, and the final
//! plaintext. The cipher itself lives in the virtualised code section (`.xtext`
//! on 26361232, `.text` on 26423880) and
//! cannot be read statically; these pairs are how its keystream and key
//! derivation are recovered offline (`analysis/FINDINGS.md` section 7).
//!
//! The decode function's prologue is observed AND its result is needed, so
//! this is not the observe-only `hook::install` shape: the prologue is
//! patched to jump to [`entry::detour`], which calls the original body through
//! a trampoline (the stolen 16 bytes plus a jump back) and inspects the output
//! buffers afterwards. The original always runs, unchanged.

pub const LOG_TAG: &str = "pazdump";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(windows)]
mod entry {
    use std::ffi::c_void;
    use std::collections::HashMap;
    use std::fs::File;
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::{BOOL, HMODULE, TRUE};
    use windows_sys::Win32::System::LibraryLoader::DisableThreadLibraryCalls;
    use windows_sys::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
    use windows_sys::Win32::System::Threading::{CreateThread, GetCurrentProcessId};

    use desert_core::{hook, log, module, safe};

    const GAME_EXE: &str = "CrimsonDesert.exe";
    const LOG_NAME: &str = "PazDump.log";
    const DIR_NAME: &str = "pazdump";
    /// `FUN_1415890B0` on build 26480954 (the same 48 bytes, once exe-wide, +0x820;
    /// callers `0x158A887`/`0x158B10D`), `FUN_141588890` on 26423880, `FUN_141587380` on 26361232:
    /// `(buf_in: &mut *u8, in_len: u32, header: *const u8, buf_out: &mut *u8,
    /// out_len: &mut u32) -> bool`. Re-found by its 16-byte prologue (45 hits
    /// exe-wide, one at the old RVA + 0x1510, the shift of the code around
    /// it), its caller `0x1589F70` (the old `0x1588A60` + 0x1510) and its LZ4
    /// callee `0x1467760` (old `0x1466440`).
    const DECODE_RVA: usize = 0x15890B0;
    const STOLEN: usize = 16;
    /// `mov [rsp+8],rbx; mov [rsp+0x10],edx; push rbp; push rsi; push rdi;
    /// push r12; push r13` - whole instructions, nothing RIP-relative.
    const PROLOGUE: [u8; STOLEN] = [
        0x48, 0x89, 0x5C, 0x24, 0x08, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41,
        0x55,
    ];
    /// Where, inside the stub page, the trampoline (stolen bytes + jump back)
    /// starts; the first 12 bytes are the jump to [`detour`].
    const TRAMP_OFF: usize = 0x20;
    const CAP_RAW: usize = 1 << 20;
    const CAP_PLAIN: usize = 4 << 20;
    const MAX_FILES: u32 = 20_000;
    /// `pazdump/worklist.tsv`: one archive entry per line (group, paz, offset,
    /// comp_size, size, flags hex, key hex, output path); when present, the
    /// plugin decodes every line through the game's own decode function and
    /// writes the plaintext under `pazdump/out/`.
    const WORKLIST: &str = "worklist.tsv";
    const ORACLE_DELAY_SECS: u64 = 20;

    type DecodeFn =
        unsafe extern "system" fn(*mut *mut u8, u32, *const u8, *mut *mut u8, *mut u32) -> u64;

    static TRAMPOLINE: AtomicUsize = AtomicUsize::new(0);
    static SEQ: AtomicU32 = AtomicU32::new(0);
    static CALLS: AtomicU32 = AtomicU32::new(0);
    static DIR: OnceLock<PathBuf> = OnceLock::new();

    fn logf(msg: String) {
        log::write_tagged(crate::LOG_TAG, &msg);
    }

    fn fnv64_update(mut h: u64, b: &[u8]) -> u64 {
        for &x in b {
            h ^= u64::from(x);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    /// FNV-1a 64 over `len` bytes at `ptr`, read in guarded chunks; `None` if
    /// any chunk is unreadable.
    fn fnv_region(ptr: usize, len: usize) -> Option<u64> {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut buf = vec![0u8; 64 << 10];
        let mut done = 0usize;
        while done < len {
            let n = (len - done).min(buf.len());
            let chunk = buf.get_mut(..n)?;
            if !safe::read_into(ptr + done, chunk) {
                return None;
            }
            h = fnv64_update(h, chunk);
            done += n;
        }
        Some(h)
    }

    fn copy_region(ptr: usize, len: usize, cap: usize) -> Vec<u8> {
        let n = len.min(cap);
        let mut v = vec![0u8; n];
        if n == 0 || ptr == 0 || !safe::read_into(ptr, &mut v) {
            v.clear();
        }
        v
    }

    #[allow(clippy::too_many_arguments)]
    fn write_dump(
        hdr: &[u8; 12],
        in_len: usize,
        out_len: usize,
        ret: u64,
        hash: Option<u64>,
        cipher: &[u8],
        decrypted: &[u8],
        plain: &[u8],
    ) {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let kind = hdr.first().copied().unwrap_or(0);
        logf(format!(
            "#{seq} kind={kind:02X} hdr={} in={in_len} out={out_len} ret={ret} fnv={:016x} saved={}/{}/{}",
            hook::hex(hdr),
            hash.unwrap_or(0),
            cipher.len(),
            decrypted.len(),
            plain.len()
        ));
        if seq >= MAX_FILES {
            return;
        }
        let Some(dir) = DIR.get() else { return };
        let path = dir.join(format!("{seq:06}.bin"));
        let mut body = Vec::with_capacity(64 + cipher.len() + decrypted.len() + plain.len());
        body.extend_from_slice(b"PZD1");
        body.extend_from_slice(hdr);
        body.extend_from_slice(&(in_len as u32).to_le_bytes());
        body.extend_from_slice(&(out_len as u32).to_le_bytes());
        body.extend_from_slice(&ret.to_le_bytes());
        body.extend_from_slice(&hash.unwrap_or(0).to_le_bytes());
        body.extend_from_slice(&(cipher.len() as u32).to_le_bytes());
        body.extend_from_slice(&(decrypted.len() as u32).to_le_bytes());
        body.extend_from_slice(&(plain.len() as u32).to_le_bytes());
        body.extend_from_slice(cipher);
        body.extend_from_slice(decrypted);
        body.extend_from_slice(plain);
        match std::fs::File::create(&path).and_then(|mut f| f.write_all(&body)) {
            Ok(()) => {}
            Err(e) => logf(format!("#{seq} WARN could not write {}: {e}", path.display())),
        }
    }

    /// Replaces the decode function's prologue. Same signature as the game's
    /// function; forwards every argument to the original body through the
    /// trampoline and records encrypted entries after it returns.
    unsafe extern "system" fn detour(
        p1: *mut *mut u8,
        p2: u32,
        p3: *const u8,
        p4: *mut *mut u8,
        p5: *mut u32,
    ) -> u64 {
        CALLS.fetch_add(1, Ordering::Relaxed);
        let mut hdr = [0u8; 12];
        let have_hdr = safe::read_into(p3 as usize, &mut hdr);
        let kind = hdr.first().copied().unwrap_or(0);
        let encrypted = have_hdr && (kind >> 4) != 0;
        let compressed = (kind & 0x0f) != 0;
        // Before the call: compressed entries are decrypted in place in the
        // input buffer and then expanded into the output buffer; plain-stored
        // entries were read straight into the output buffer and are decrypted
        // there.
        let (src, src_len) = if compressed {
            (safe::read_ptr(p1 as usize).unwrap_or(0), p2 as usize)
        } else {
            (
                safe::read_ptr(p4 as usize).unwrap_or(0),
                safe::read::<u32>(p5 as usize).map_or(0, |v| v as usize),
            )
        };
        let (cipher, hash) = if encrypted {
            (copy_region(src, src_len, CAP_RAW), fnv_region(src, src_len))
        } else {
            (Vec::new(), None)
        };

        let mut tramp = TRAMPOLINE.load(Ordering::Acquire);
        while tramp == 0 {
            // Only reachable in the instant between the prologue patch and
            // the store of the stub address that follows it in `install`.
            core::hint::spin_loop();
            tramp = TRAMPOLINE.load(Ordering::Acquire);
        }
        // SAFETY: `tramp` is the address `install` stored of a stub page that
        // holds the original function's first STOLEN bytes followed by a jump
        // to `target + STOLEN`; calling it is calling the original function,
        // and the arguments are the game's own, passed through untouched.
        let f: DecodeFn = unsafe { core::mem::transmute::<usize, DecodeFn>(tramp) };
        // SAFETY: as above - the original function with the original arguments.
        let ret = unsafe { f(p1, p2, p3, p4, p5) };

        if encrypted {
            let decrypted = copy_region(src, src_len, CAP_RAW);
            let out_ptr = safe::read_ptr(p4 as usize).unwrap_or(0);
            let out_len = safe::read::<u32>(p5 as usize).map_or(0, |v| v as usize);
            let plain = copy_region(out_ptr, out_len, CAP_PLAIN);
            write_dump(&hdr, src_len, out_len, ret, hash, &cipher, &decrypted, &plain);
        }
        ret
    }

    fn install(base: usize) -> Result<(), String> {
        let target = base + DECODE_RVA;
        let mut cur = [0u8; STOLEN];
        if !safe::read_into(target, &mut cur) {
            return Err(format!("0x{target:X} not readable"));
        }
        if cur != PROLOGUE {
            return Err(format!("prologue at 0x{target:X} is {}, expected {}", hook::hex(&cur), hook::hex(&PROLOGUE)));
        }
        let mut s = Vec::with_capacity(0x40);
        s.extend_from_slice(&[0x48, 0xB8]); // mov rax,detour
        s.extend_from_slice(&(detour as *const () as usize as u64).to_le_bytes());
        s.extend_from_slice(&[0xFF, 0xE0]); // jmp rax
        s.resize(TRAMP_OFF, 0xCC);
        s.extend_from_slice(&cur); // the stolen prologue
        s.extend_from_slice(&[0x48, 0xB8]); // mov rax,target+STOLEN
        s.extend_from_slice(&((target + STOLEN) as u64).to_le_bytes());
        s.extend_from_slice(&[0xFF, 0xE0]); // jmp rax
        // SAFETY: `target` is the decode function's entry (its first STOLEN
        // bytes were just compared byte for byte with the known prologue of
        // whole, position-independent instructions), and the stub ends by
        // jumping back to `target + STOLEN`, so the original function keeps
        // running in full. Installed from the plugin's first thread, before
        // the game has opened its archives.
        let h = unsafe { hook::install_raw(target, STOLEN, &s)? };
        TRAMPOLINE.store(h.stub + TRAMP_OFF, Ordering::Release);
        logf(format!(
            "hooked decode 0x{target:X} (rva 0x{DECODE_RVA:X}), stub 0x{:X}, trampoline 0x{:X}",
            h.stub,
            h.stub + TRAMP_OFF
        ));
        Ok(())
    }


    struct Work {
        group: String,
        paz: u32,
        offset: u64,
        comp: u32,
        size: u32,
        flags: u32,
        key: u32,
        out: String,
    }

    fn parse_work(line: &str) -> Option<Work> {
        let mut it = line.split('\t');
        let group = it.next()?.to_string();
        let paz = it.next()?.parse().ok()?;
        let offset = it.next()?.parse().ok()?;
        let comp = it.next()?.parse().ok()?;
        let size = it.next()?.parse().ok()?;
        let flags = u32::from_str_radix(it.next()?, 16).ok()?;
        let key = u32::from_str_radix(it.next()?, 16).ok()?;
        let out = it.next()?.to_string();
        Some(Work { group, paz, offset, comp, size, flags, key, out })
    }

    /// The oracle: decode every worklist entry through the game's decode
    /// function (reached through the trampoline, so the capture detour does
    /// not see these calls) and write the plaintext out.
    fn oracle(dir: &Path, tramp: usize) {
        let wl = dir.join(WORKLIST);
        let Ok(text) = std::fs::read_to_string(&wl) else {
            logf(format!("oracle: no {} - capture only", wl.display()));
            return;
        };
        let root = log::exe_dir().parent().map(Path::to_path_buf).unwrap_or_default();
        let outdir = dir.join("out");
        // SAFETY: `tramp` is the trampoline `install` stored: the original
        // decode function, entered past its patched prologue.
        let f: DecodeFn = unsafe { core::mem::transmute::<usize, DecodeFn>(tramp) };
        let mut files: HashMap<(String, u32), Option<File>> = HashMap::new();
        let (mut n, mut ok, mut bad, mut io) = (0u32, 0u32, 0u32, 0u32);
        logf(format!("oracle: {} lines, archives under {}", text.lines().count(), root.display()));
        for line in text.lines() {
            n += 1;
            let Some(w) = parse_work(line) else {
                bad += 1;
                continue;
            };
            let path = root.join(&w.group).join(format!("{}.paz", w.paz));
            let slot = files
                .entry((w.group.clone(), w.paz))
                .or_insert_with(|| File::open(&path).ok());
            let Some(file) = slot.as_mut() else {
                io += 1;
                continue;
            };
            let mut raw = vec![0u8; w.comp as usize];
            if file.seek(SeekFrom::Start(w.offset)).is_err() || file.read_exact(&mut raw).is_err() {
                io += 1;
                continue;
            }
            let kind = ((w.flags >> 16) & 0xff) as u8;
            let k = w.key.to_le_bytes();
            let s = w.size.to_le_bytes();
            let hdr: [u8; 12] = [kind, 0x02, 0x0E, 0x61, k[0], k[1], k[2], k[3], s[0], s[1], s[2], s[3]];
            // The decoder writes one NUL past the declared size.
            let mut out = vec![0u8; w.size as usize + 16];
            let mut in_ptr: *mut u8 = core::ptr::null_mut();
            if kind & 0x0f == 0 {
                // Stored: the game reads these straight into the output buffer
                // and decrypts there; the input pointer must be null.
                match out.get_mut(..raw.len()) {
                    Some(dst) => dst.copy_from_slice(&raw),
                    None => {
                        bad += 1;
                        continue;
                    }
                }
            } else {
                in_ptr = raw.as_mut_ptr();
            }
            let mut out_ptr = out.as_mut_ptr();
            let mut out_len = w.size;
            // SAFETY: the original decode function with buffers this thread
            // owns: `raw` holds `comp` readable-writable bytes (decrypted in
            // place), `out` holds `size + 16` bytes for the result and its
            // terminator, `hdr` is the 12-byte header the game builds for the
            // same entry, and every pointer outlives the call.
            let ret = unsafe { f(&mut in_ptr, w.comp, hdr.as_ptr(), &mut out_ptr, &mut out_len) };
            if ret & 0xff == 1 {
                let dest = outdir.join(&w.out);
                if let Some(parent) = dest.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let body = out.get(..w.size as usize).unwrap_or(&[]);
                match File::create(&dest).and_then(|mut fh| fh.write_all(body)) {
                    Ok(()) => ok += 1,
                    Err(e) => {
                        io += 1;
                        if io <= 20 {
                            logf(format!("oracle: cannot write {}: {e}", dest.display()));
                        }
                    }
                }
            } else {
                bad += 1;
                if bad <= 40 {
                    logf(format!(
                        "oracle: FAILED {} kind={kind:02X} key={:08X} comp={} size={} ret={ret}",
                        w.out, w.key, w.comp, w.size
                    ));
                }
            }
            if n % 5000 == 0 {
                logf(format!("oracle: {n} done, {ok} ok, {bad} failed, {io} io errors"));
            }
        }
        logf(format!("oracle done: {n} lines, {ok} ok, {bad} failed, {io} io errors -> {}", outdir.display()));
    }

    unsafe extern "system" fn main_thread(_param: *mut c_void) -> u32 {
        log::init(LOG_NAME);
        // SAFETY: `GetCurrentProcessId` takes no arguments and only reads this
        // process's own PEB; it is sound to call from any thread.
        let pid = unsafe { GetCurrentProcessId() };
        logf(format!("Desert PazDump {} loaded, pid {pid}", crate::VERSION));
        let dir = log::exe_dir().join(DIR_NAME);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            logf(format!("WARN could not create {}: {e}; nothing will be dumped", dir.display()));
            return 0;
        }
        let _ = DIR.set(dir);
        let Some(m) = module::MainModule::locate() else {
            logf("WARN main module not found".into());
            return 0;
        };
        match install(m.base) {
            Ok(()) => {
                std::thread::sleep(std::time::Duration::from_secs(ORACLE_DELAY_SECS));
                if let Some(dir) = DIR.get() {
                    oracle(dir, TRAMPOLINE.load(Ordering::Acquire));
                }
            }
            Err(e) => logf(format!("WARN not hooked: {e}")),
        }
        loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
            logf(format!(
                "stat: {} decode calls, {} encrypted entries dumped",
                CALLS.load(Ordering::Relaxed),
                SEQ.load(Ordering::Relaxed)
            ));
        }
    }

    fn host_exe_name() -> String {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default()
    }

    #[no_mangle]
    pub extern "system" fn DllMain(hinst: HMODULE, reason: u32, _reserved: *mut c_void) -> BOOL {
        if reason == DLL_PROCESS_ATTACH {
            // SAFETY: `hinst` is the handle the loader passed for this very
            // module, so it is a live HMODULE; the call only clears this
            // module's thread-attach notifications.
            unsafe {
                DisableThreadLibraryCalls(hinst);
            }
            if !host_exe_name().eq_ignore_ascii_case(GAME_EXE) {
                log::disable();
                return TRUE;
            }
            // SAFETY: every pointer argument is null except the entry point,
            // a `'static` function in this module, which is never unloaded.
            // Creating a thread is permitted under the loader lock; it does
            // not run until DllMain returns.
            unsafe {
                CreateThread(
                    core::ptr::null(),
                    0,
                    Some(main_thread),
                    core::ptr::null(),
                    0,
                    core::ptr::null_mut(),
                );
            }
        }
        TRUE
    }
}
