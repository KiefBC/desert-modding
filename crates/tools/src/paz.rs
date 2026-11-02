//! The game's data archives: a group's `0.pamt` index and its `<n>.paz`
//! containers, read offline.
//!
//! The install is 36 groups `0000`..`0035`, each `<group>/0.pamt` plus
//! `<group>/<n>.paz`. The index layout (`analysis/FINDINGS.md` section 7):
//!
//! ```text
//! u32 bodyHash            lookup3(seed 0xC5EDE) of bytes 12..end
//! u16 pazCount, u16 pad
//! u32 version             0x610E0232
//! pazCount x { u32 index, u32 hash, u32 size }        one per <n>.paz
//! u32 n1; u8 A[n1]        directory name trie
//! u32 n2; u8 B[n2]        file name trie
//! u32 c3; c3 x { u32 dirHash, u32 nameOff, u32 firstFile, u32 fileCount }
//! u32 c4; c4 x { u32 nameOff, u32 offset, u32 compSize, u32 size, u32 flags }
//! u32 n5; u8 S[n5]        NUL-separated extension table
//! ```
//!
//! Both tries are runs of nodes `{varint parentDelta, varint nameLen, name}`,
//! where `parentDelta` is the distance back to the parent node's start (0 =
//! root) and a name is the concatenation of the fragments along the parent
//! chain. `flags`: bits 0-7 pick the `<n>.paz`, bits 16-19 are the compression
//! kind (0 stored, 2 LZ4 block), bits 20-23 the encryption kind, bits 24-31 the
//! extension index.
//!
//! Every read here is bounds-checked and returns an error rather than
//! panicking: a stub group (`0016` is 96 bytes of index) or a future layout
//! change must say so, not abort a listing of the other 35. Encrypted entries
//! are refused, never returned as ciphertext.

use std::cell::OnceCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, ensure, Context, Result};

/// The group the static-info tables live in, tried first by [`Archive::find`].
pub const DEFAULT_GROUP: &str = "0008";

/// The deepest parent chain a trie name may have. Real names are a handful of
/// fragments deep; this only stops a corrupt trie from looping.
const MAX_CHAIN: usize = 256;

/// One `<n>.paz` named by the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PazInfo {
    pub index: u32,
    pub hash: u32,
    pub size: u32,
}

/// One directory of the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dir {
    pub hash: u32,
    pub name: String,
    pub first_file: u32,
    pub file_count: u32,
}

/// One file of the index, its name already resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub dir: String,
    /// The file name with its extension.
    pub name: String,
    pub offset: u32,
    pub comp_size: u32,
    pub size: u32,
    pub flags: u32,
}

impl Entry {
    /// Which `<n>.paz` holds the bytes.
    pub fn paz(&self) -> u32 {
        self.flags & 0xff
    }

    /// The compression kind: 0 stored, 2 LZ4 block.
    pub fn compression(&self) -> u32 {
        (self.flags >> 16) & 0xF
    }

    /// The encryption kind: 0 none.
    pub fn encryption(&self) -> u32 {
        (self.flags >> 20) & 0xF
    }

    /// `dir/name`, or just `name` for a file in the root directory.
    pub fn path(&self) -> String {
        if self.dir.is_empty() {
            self.name.clone()
        } else {
            format!("{}/{}", self.dir, self.name)
        }
    }
}

/// A parsed `0.pamt`.
#[derive(Debug, Clone)]
pub struct Pamt {
    pub body_hash: u32,
    pub version: u32,
    pub pazs: Vec<PazInfo>,
    pub dirs: Vec<Dir>,
    pub entries: Vec<Entry>,
}

/// A bounds-checked little-endian cursor.
struct Cursor<'a> {
    b: &'a [u8],
    o: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize, what: &str) -> Result<&'a [u8]> {
        let end = self.o.checked_add(n).ok_or_else(|| anyhow!("{what}: length overflows"))?;
        let s = self
            .b
            .get(self.o..end)
            .ok_or_else(|| anyhow!("{what}: {n} bytes at {:#x} run past the end ({:#x})", self.o, self.b.len()))?;
        self.o = end;
        Ok(s)
    }

    fn u16(&mut self, what: &str) -> Result<u16> {
        let s = self.take(2, what)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }

    fn u32(&mut self, what: &str) -> Result<u32> {
        let s = self.take(4, what)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    /// `count` records of `width` bytes each, checked against the remaining
    /// length before anything is allocated.
    fn table(&mut self, count: u32, width: usize, what: &str) -> Result<&'a [u8]> {
        let n = (count as usize).checked_mul(width).ok_or_else(|| anyhow!("{what}: count overflows"))?;
        self.take(n, what)
    }
}

fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// One trie node: its parent's start, if any, and its fragment.
type Nodes<'a> = HashMap<usize, (Option<usize>, &'a [u8])>;

fn read_varint(b: &[u8], o: &mut usize) -> Option<usize> {
    let mut v: u64 = 0;
    for shift in (0..64).step_by(7) {
        let c = *b.get(*o)?;
        *o += 1;
        v |= u64::from(c & 0x7f) << shift;
        if c < 0x80 {
            return usize::try_from(v).ok();
        }
    }
    None
}

/// Parse a whole trie blob. `varint` picks the node encoding: varint/varint
/// (every group seen) or u8/u8 (the fallback `pamt_names.py` also tries).
/// Every parent must be a node already seen and the blob must be consumed
/// exactly.
fn parse_trie(b: &[u8], varint: bool) -> Result<Nodes<'_>> {
    let mut nodes = Nodes::new();
    let mut o = 0;
    while o < b.len() {
        let start = o;
        let (delta, len) = if varint {
            let d = read_varint(b, &mut o).ok_or_else(|| anyhow!("node at {start:#x}: bad parent varint"))?;
            let l = read_varint(b, &mut o).ok_or_else(|| anyhow!("node at {start:#x}: bad length varint"))?;
            (d, l)
        } else {
            let d = *b.get(o).ok_or_else(|| anyhow!("node at {start:#x}: truncated"))?;
            let l = *b.get(o + 1).ok_or_else(|| anyhow!("node at {start:#x}: truncated"))?;
            o += 2;
            (usize::from(d), usize::from(l))
        };
        let end = o.checked_add(len).ok_or_else(|| anyhow!("node at {start:#x}: length overflows"))?;
        let name = b.get(o..end).ok_or_else(|| anyhow!("node at {start:#x}: name runs past the end"))?;
        o = end;
        let parent = if delta == 0 {
            None
        } else {
            let p = start.checked_sub(delta).ok_or_else(|| anyhow!("node at {start:#x}: parent before the blob"))?;
            ensure!(nodes.contains_key(&p), "node at {start:#x}: parent {p:#x} is not a node");
            Some(p)
        };
        nodes.insert(start, (parent, name));
    }
    Ok(nodes)
}

/// The name of the node at `off`: every fragment from the root down. Bytes
/// are read as Latin-1, as the Python reference did.
fn name_of(nodes: &Nodes<'_>, off: u32, what: &str) -> Result<String> {
    let mut parts = Vec::new();
    let mut at = Some(off as usize);
    while let Some(o) = at {
        ensure!(parts.len() < MAX_CHAIN, "{what}: name at {off:#x} is deeper than {MAX_CHAIN} fragments");
        let (parent, frag) = nodes.get(&o).ok_or_else(|| anyhow!("{what}: {o:#x} is not a trie node"))?;
        parts.push(*frag);
        at = *parent;
    }
    Ok(parts.iter().rev().flat_map(|p| p.iter()).map(|&c| char::from(c)).collect())
}

/// Parse a `0.pamt`.
pub fn parse_pamt(b: &[u8]) -> Result<Pamt> {
    let mut c = Cursor { b, o: 0 };
    let body_hash = c.u32("body hash")?;
    let paz_count = c.u16("paz count")?;
    let _pad = c.u16("pad")?;
    let version = c.u32("version")?;
    let raw = c.table(u32::from(paz_count), 12, "paz table")?;
    let pazs = raw
        .as_chunks::<12>().0.iter()
        .map(|r| PazInfo { index: le32(r, 0), hash: le32(r, 4), size: le32(r, 8) })
        .collect();

    let n1 = c.u32("dir trie length")?;
    let a = c.take(n1 as usize, "dir trie")?;
    let n2 = c.u32("file trie length")?;
    let bt = c.take(n2 as usize, "file trie")?;
    let c3 = c.u32("dir count")?;
    let raw_dirs = c.table(c3, 16, "dir table")?;
    let c4 = c.u32("file count")?;
    let raw_files = c.table(c4, 20, "file table")?;
    let n5 = c.u32("extension table length")?;
    let s = c.take(n5 as usize, "extension table")?;
    ensure!(c.o == b.len(), "{} bytes left over after the extension table at {:#x}", b.len() - c.o, c.o);

    let exts: Vec<String> = s.split(|&x| x == 0).map(|e| e.iter().map(|&x| char::from(x)).collect()).collect();

    let (ta, tb) = match (parse_trie(a, true), parse_trie(bt, true)) {
        (Ok(ta), Ok(tb)) => (ta, tb),
        (va, vb) => match (parse_trie(a, false), parse_trie(bt, false)) {
            (Ok(ta), Ok(tb)) => (ta, tb),
            _ => {
                let why = va.err().or(vb.err()).map(|e| e.to_string()).unwrap_or_default();
                bail!("the name tries parse neither as varint nor as u8 nodes (varint: {why})")
            }
        },
    };

    let mut dirs = Vec::with_capacity(c3 as usize);
    for r in raw_dirs.as_chunks::<16>().0 {
        let (hash, name_off, first_file, file_count) = (le32(r, 0), le32(r, 4), le32(r, 8), le32(r, 12));
        let end = u64::from(first_file) + u64::from(file_count);
        ensure!(end <= u64::from(c4), "dir {name_off:#x}: files {first_file}..{end} past the {c4} files");
        dirs.push(Dir { hash, name: name_of(&ta, name_off, "dir")?, first_file, file_count });
    }

    let mut dir_of: Vec<Option<usize>> = vec![None; c4 as usize];
    for (di, d) in dirs.iter().enumerate() {
        for slot in &mut dir_of[d.first_file as usize..(d.first_file + d.file_count) as usize] {
            ensure!(slot.is_none(), "file claimed by two directories ({})", d.name);
            *slot = Some(di);
        }
    }

    let mut entries = Vec::with_capacity(c4 as usize);
    for (i, r) in raw_files.as_chunks::<20>().0.iter().enumerate() {
        let (name_off, offset, comp_size, size, flags) = (le32(r, 0), le32(r, 4), le32(r, 8), le32(r, 12), le32(r, 16));
        let di = dir_of[i].ok_or_else(|| anyhow!("file {i} is in no directory"))?;
        let ext_i = (flags >> 24) as usize;
        let ext = exts.get(ext_i).ok_or_else(|| anyhow!("file {i}: extension {ext_i} past the {} in the table", exts.len()))?;
        let mut name = name_of(&tb, name_off, "file")?;
        name.push_str(ext);
        entries.push(Entry { dir: dirs[di].name.clone(), name, offset, comp_size, size, flags });
    }

    Ok(Pamt { body_hash, version, pazs, dirs, entries })
}

/// Decode one LZ4 block (no frame). The output must come out at exactly
/// `size` bytes; anything else (a truncated sequence, an offset reaching
/// before the start, output past `size`) is an error.
pub fn lz4_block(src: &[u8], size: usize) -> Result<Vec<u8>> {
    fn more(src: &[u8], i: &mut usize, mut n: usize, what: &str) -> Result<usize> {
        loop {
            let b = *src.get(*i).ok_or_else(|| anyhow!("lz4: {what} length runs past the input"))?;
            *i += 1;
            n = n.checked_add(usize::from(b)).ok_or_else(|| anyhow!("lz4: {what} length overflows"))?;
            if b != 255 {
                return Ok(n);
            }
        }
    }

    let mut dst: Vec<u8> = Vec::with_capacity(size);
    let mut i = 0;
    while i < src.len() {
        let token = src[i];
        i += 1;
        let mut lit = usize::from(token >> 4);
        if lit == 15 {
            lit = more(src, &mut i, lit, "literal")?;
        }
        let end = i.checked_add(lit).ok_or_else(|| anyhow!("lz4: literal length overflows"))?;
        let lits = src.get(i..end).ok_or_else(|| anyhow!("lz4: {lit} literals at {i:#x} run past the input"))?;
        ensure!(dst.len() + lit <= size, "lz4: output exceeds the declared {size} bytes");
        dst.extend_from_slice(lits);
        i = end;
        if i >= src.len() {
            break;
        }
        let off = src.get(i..i + 2).ok_or_else(|| anyhow!("lz4: match offset at {i:#x} truncated"))?;
        let off = usize::from(off[0]) | (usize::from(off[1]) << 8);
        i += 2;
        let mut ml = usize::from(token & 15);
        if ml == 15 {
            ml = more(src, &mut i, ml, "match")?;
        }
        ml += 4;
        ensure!(off != 0 && off <= dst.len(), "lz4: match offset {off} at {i:#x} reaches before the output");
        ensure!(dst.len() + ml <= size, "lz4: output exceeds the declared {size} bytes");
        let start = dst.len() - off;
        if off >= ml {
            dst.extend_from_within(start..start + ml);
        } else {
            for k in 0..ml {
                let c = dst[start + k];
                dst.push(c);
            }
        }
    }
    ensure!(dst.len() == size, "lz4: decoded {} bytes, the index says {size}", dst.len());
    Ok(dst)
}

/// One group directory: its index and where its containers are.
#[derive(Debug)]
pub struct Group {
    pub dir: PathBuf,
    pub pamt: Pamt,
    /// Containers on disk that are LONGER than the index says, as
    /// `(file, bytes on disk, bytes in the index)`. Every entry still lies
    /// inside the indexed length, so these read correctly; they are kept so a
    /// caller can say so. On the 26361232 install `0004/1.paz` and `0004/3.paz`
    /// carry a few KB past their indexed size and `0009/40.paz` is a
    /// 2026-09-18 file of 913 MB that the 2026-10-30 index lists at 147 MB.
    pub oversize: Vec<(PathBuf, u64, u32)>,
    by_path: HashMap<String, usize>,
}

impl Group {
    /// Read `<dir>/0.pamt` and check each container that exists against the
    /// size the index records for it. A container SHORTER than the index says
    /// is an error (some entry would read past its end); a longer one is
    /// recorded in [`Group::oversize`] and allowed.
    pub fn open(dir: &Path) -> Result<Group> {
        let idx = dir.join("0.pamt");
        let bytes = std::fs::read(&idx).with_context(|| format!("cannot read {}", idx.display()))?;
        let pamt = parse_pamt(&bytes).with_context(|| format!("cannot parse {}", idx.display()))?;
        let mut oversize = Vec::new();
        for p in &pamt.pazs {
            let file = dir.join(format!("{}.paz", p.index));
            if let Ok(meta) = std::fs::metadata(&file) {
                ensure!(
                    meta.len() >= u64::from(p.size),
                    "{} is {} bytes, the index says {}",
                    file.display(),
                    meta.len(),
                    p.size
                );
                if meta.len() > u64::from(p.size) {
                    oversize.push((file, meta.len(), p.size));
                }
            }
        }
        for e in &pamt.entries {
            let p = pamt.pazs.iter().find(|p| p.index == e.paz());
            let p = p.ok_or_else(|| anyhow!("{}: names {}.paz, which the index does not list", e.path(), e.paz()))?;
            ensure!(
                u64::from(e.offset) + u64::from(e.comp_size) <= u64::from(p.size),
                "{}: {} bytes at {:#x} run past {}.paz's indexed {} bytes",
                e.path(),
                e.comp_size,
                e.offset,
                p.index,
                p.size
            );
        }
        let by_path = pamt.entries.iter().enumerate().map(|(i, e)| (e.path(), i)).collect();
        Ok(Group { dir: dir.to_path_buf(), pamt, oversize, by_path })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.pamt.entries
    }

    /// The entry at `path` (`dir/name`), if this group has one.
    pub fn find(&self, path: &str) -> Option<&Entry> {
        self.by_path.get(path).map(|&i| &self.pamt.entries[i])
    }

    /// The raw bytes of `e` as stored in its container, before decoding.
    pub fn read_raw(&self, e: &Entry) -> Result<Vec<u8>> {
        let file = self.dir.join(format!("{}.paz", e.paz()));
        let mut f = File::open(&file).with_context(|| format!("cannot open {}", file.display()))?;
        f.seek(SeekFrom::Start(u64::from(e.offset)))?;
        let mut buf = vec![0; e.comp_size as usize];
        f.read_exact(&mut buf)
            .with_context(|| format!("{}: {} bytes at {:#x} run past the end", file.display(), e.comp_size, e.offset))?;
        Ok(buf)
    }

    /// The decoded bytes of `e`. Encrypted entries and compression kinds
    /// other than stored and LZ4 block are errors.
    pub fn read(&self, e: &Entry) -> Result<Vec<u8>> {
        ensure!(e.encryption() == 0, "{}: encrypted (kind {}, flags {:#x}), not readable offline", e.path(), e.encryption(), e.flags);
        match e.compression() {
            0 => {
                ensure!(e.comp_size == e.size, "{}: stored, but {} bytes on disk for {}", e.path(), e.comp_size, e.size);
                self.read_raw(e)
            }
            2 => lz4_block(&self.read_raw(e)?, e.size as usize).with_context(|| e.path()),
            k => bail!("{}: compression kind {k} is not supported (flags {:#x})", e.path(), e.flags),
        }
    }
}

/// Every group of an install, opened on demand.
pub struct Archive {
    pub game_dir: PathBuf,
    names: Vec<String>,
    groups: Vec<OnceCell<Option<Group>>>,
}

impl Archive {
    /// List the groups under `game_dir`: every directory holding a `0.pamt`.
    /// Nothing is parsed yet.
    pub fn open(game_dir: &Path) -> Result<Archive> {
        let mut names = Vec::new();
        for de in std::fs::read_dir(game_dir).with_context(|| format!("cannot list {}", game_dir.display()))? {
            let de = de?;
            if de.path().join("0.pamt").is_file() {
                names.push(de.file_name().to_string_lossy().into_owned());
            }
        }
        names.sort();
        ensure!(!names.is_empty(), "no <group>/0.pamt under {}", game_dir.display());
        let groups = names.iter().map(|_| OnceCell::new()).collect();
        Ok(Archive { game_dir: game_dir.to_path_buf(), names, groups })
    }

    /// The group names, sorted.
    pub fn group_names(&self) -> &[String] {
        &self.names
    }

    /// One group, parsed on first use. A group that fails to parse is
    /// remembered as such; [`Archive::find`] skips it and this says why.
    pub fn group(&self, name: &str) -> Result<&Group> {
        let i = self.names.iter().position(|n| n == name).ok_or_else(|| anyhow!("no group {name} under {}", self.game_dir.display()))?;
        if let Some(g) = self.groups[i].get() {
            return g.as_ref().ok_or_else(|| anyhow!("group {name} does not parse"));
        }
        match Group::open(&self.game_dir.join(name)) {
            Ok(g) => Ok(self.groups[i].get_or_init(|| Some(g)).as_ref().unwrap()),
            Err(e) => {
                let _ = self.groups[i].set(None);
                Err(e)
            }
        }
    }

    /// Where `path` lives: [`DEFAULT_GROUP`] first, then every other group in
    /// order, skipping groups that do not parse.
    pub fn find(&self, path: &str) -> Option<(&str, &Entry)> {
        let order = std::iter::once(DEFAULT_GROUP).chain(self.names.iter().map(String::as_str).filter(|n| *n != DEFAULT_GROUP));
        for name in order {
            let Ok(g) = self.group(name) else { continue };
            if let Some(e) = g.find(path) {
                return Some((name, e));
            }
        }
        None
    }

    /// The decoded bytes of the file at `path`.
    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        let (group, e) = self.find(path).ok_or_else(|| anyhow!("{path} is in no group under {}", self.game_dir.display()))?;
        self.group(group)?.read(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn varint(out: &mut Vec<u8>, mut v: usize) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return;
            }
            out.push(b | 0x80);
        }
    }

    /// Append a node; returns its start. `parent` is the parent's start.
    fn node(trie: &mut Vec<u8>, parent: Option<usize>, name: &[u8]) -> usize {
        let start = trie.len();
        varint(trie, parent.map_or(0, |p| start - p));
        varint(trie, name.len());
        trie.extend_from_slice(name);
        start
    }

    fn put(out: &mut Vec<u8>, v: &[u32]) {
        for x in v {
            out.extend_from_slice(&x.to_le_bytes());
        }
    }

    /// Two dirs (`gamedata/bin`, `ui`), three files (`gamedata/bin/a.staticinfobody`,
    /// `gamedata/bin/ab.staticinfoheader`, `ui/x`).
    fn sample() -> Vec<u8> {
        let mut a = Vec::new();
        let gd = node(&mut a, None, b"gamedata");
        let bin = node(&mut a, Some(gd), b"/bin");
        let ui = node(&mut a, None, b"ui");
        let mut b = Vec::new();
        let fa = node(&mut b, None, b"a");
        let fab = node(&mut b, Some(fa), b"b");
        let fx = node(&mut b, None, b"x");
        let exts = b"\0.staticinfobody\0.staticinfoheader";

        let mut out = Vec::new();
        put(&mut out, &[0x1234_5678]);
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        put(&mut out, &[0x610E_0232]);
        put(&mut out, &[0, 0xAA, 100, 1, 0xBB, 200]);
        put(&mut out, &[a.len() as u32]);
        out.extend_from_slice(&a);
        put(&mut out, &[b.len() as u32]);
        out.extend_from_slice(&b);
        put(&mut out, &[2, 0x11, bin as u32, 0, 2, 0x22, ui as u32, 2, 1]);
        put(&mut out, &[3]);
        put(&mut out, &[fa as u32, 0, 10, 10, 0x0100_0000]);
        put(&mut out, &[fab as u32, 10, 4, 8, 0x0202_0001]);
        put(&mut out, &[fx as u32, 15, 7, 7, 0x0030_0000]);
        put(&mut out, &[exts.len() as u32]);
        out.extend_from_slice(exts);
        out
    }

    #[test]
    fn parses_a_hand_built_index() {
        let p = parse_pamt(&sample()).unwrap();
        assert_eq!(p.version, 0x610E_0232);
        assert_eq!(p.pazs, vec![PazInfo { index: 0, hash: 0xAA, size: 100 }, PazInfo { index: 1, hash: 0xBB, size: 200 }]);
        assert_eq!(p.dirs.len(), 2);
        assert_eq!(p.dirs[0].name, "gamedata/bin");
        let paths: Vec<String> = p.entries.iter().map(Entry::path).collect();
        assert_eq!(paths, ["gamedata/bin/a.staticinfobody", "gamedata/bin/ab.staticinfoheader", "ui/x"]);
        let e = &p.entries[1];
        assert_eq!((e.paz(), e.compression(), e.encryption()), (1, 2, 0));
        assert_eq!(p.entries[2].encryption(), 3);
    }

    #[test]
    fn leftover_or_truncated_bytes_are_errors() {
        let mut b = sample();
        b.push(0);
        assert!(parse_pamt(&b).is_err());
        let b = sample();
        for cut in [0, 3, 11, 40, b.len() - 1] {
            assert!(parse_pamt(&b[..cut]).is_err(), "cut at {cut}");
        }
    }

    #[test]
    fn a_huge_count_is_an_error_not_an_allocation() {
        let mut b = sample();
        // The paz count is u16; make the dir trie length absurd instead.
        let at = 12 + 24;
        b[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_pamt(&b).is_err());
    }

    #[test]
    fn varint_trie_with_a_multi_fragment_name() {
        let mut t = Vec::new();
        let r = node(&mut t, None, b"a");
        let long = vec![b'y'; 200]; // a two-byte length varint
        let mid = node(&mut t, Some(r), &long);
        // Pad so the next parent delta also needs two bytes.
        let mut last = mid;
        for _ in 0..3 {
            last = node(&mut t, Some(last), b"_");
        }
        let leaf = node(&mut t, Some(last), b"z");
        let nodes = parse_trie(&t, true).unwrap();
        let want = format!("a{}___z", "y".repeat(200));
        assert_eq!(name_of(&nodes, leaf as u32, "t").unwrap(), want);
        assert!(name_of(&nodes, leaf as u32 + 1, "t").is_err());
    }

    #[test]
    fn an_unresolved_parent_is_an_error() {
        let mut t = Vec::new();
        node(&mut t, None, b"ab");
        // A node whose delta lands inside the first node, not at its start.
        let start = t.len();
        varint(&mut t, start - 1);
        varint(&mut t, 1);
        t.push(b'c');
        assert!(parse_trie(&t, true).is_err());
        assert!(parse_trie(&[0x80], true).is_err());
    }

    #[test]
    fn u8_trie_fallback() {
        // u8/u8: parent delta 0, length 200 (0xC8 is a varint continuation byte).
        let mut t = vec![0, 200];
        t.extend(std::iter::repeat_n(b'q', 200));
        assert!(parse_trie(&t, true).is_err());
        let nodes = parse_trie(&t, false).unwrap();
        assert_eq!(name_of(&nodes, 0, "t").unwrap().len(), 200);
    }

    #[test]
    fn lz4_literals_only() {
        assert_eq!(lz4_block(&[0x50, b'h', b'e', b'l', b'l', b'o'], 5).unwrap(), b"hello");
        assert_eq!(lz4_block(&[], 0).unwrap(), b"");
    }

    #[test]
    fn lz4_overlapping_match() {
        // One literal 'a', then offset 1 length 4+3: eight 'a's in all.
        assert_eq!(lz4_block(&[0x13, b'a', 1, 0], 8).unwrap(), b"aaaaaaaa");
        // Offset 2 over "ab": "abababab" plus a trailing literal block.
        assert_eq!(lz4_block(&[0x22, b'a', b'b', 2, 0, 0x10, b'!'], 9).unwrap(), b"abababab!");
    }

    #[test]
    fn lz4_length_extension() {
        // 15 + 255 + 5 = 275 literals, then a match of 4 + 15 + 255 + 1 = 275 at offset 1.
        let mut src = vec![0xFF, 255, 5];
        src.extend(std::iter::repeat_n(b'x', 275));
        src.extend_from_slice(&[1, 0, 255, 1]);
        let out = lz4_block(&src, 550).unwrap();
        assert_eq!(out, vec![b'x'; 550]);
    }

    #[test]
    fn lz4_errors() {
        // Truncated literals, truncated offset, truncated length extension.
        assert!(lz4_block(&[0x50, b'h', b'e'], 5).is_err());
        assert!(lz4_block(&[0x13, b'a', 1], 8).is_err());
        assert!(lz4_block(&[0xF0, 255], 300).is_err());
        // Offset before the start, and offset 0.
        assert!(lz4_block(&[0x13, b'a', 2, 0], 8).is_err());
        assert!(lz4_block(&[0x13, b'a', 0, 0], 8).is_err());
        // Wrong final size, short and long.
        assert!(lz4_block(&[0x13, b'a', 1, 0], 9).is_err());
        assert!(lz4_block(&[0x13, b'a', 1, 0], 7).is_err());
    }

    #[test]
    fn group_reads_stored_and_lz4_and_refuses_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let mut idx = sample();
        // Fix the container sizes to what is written below.
        let mut paz0 = b"0123456789".to_vec();
        paz0.extend_from_slice(&[0; 12]);
        let paz1: Vec<u8> = {
            let mut v = vec![0u8; 10];
            v.extend_from_slice(&[0x13, b'a', 1, 0]);
            v.extend_from_slice(&[0; 8]);
            v
        };
        idx[20..24].copy_from_slice(&(paz0.len() as u32).to_le_bytes());
        idx[32..36].copy_from_slice(&(paz1.len() as u32).to_le_bytes());
        std::fs::write(dir.path().join("0.pamt"), &idx).unwrap();
        std::fs::write(dir.path().join("0.paz"), &paz0).unwrap();
        std::fs::write(dir.path().join("1.paz"), &paz1).unwrap();
        let g = Group::open(dir.path()).unwrap();
        let a = g.find("gamedata/bin/a.staticinfobody").unwrap();
        assert_eq!(g.read(a).unwrap(), b"0123456789");
        let ab = g.find("gamedata/bin/ab.staticinfoheader").unwrap();
        assert_eq!(g.read(ab).unwrap(), b"aaaaaaaa");
        // The same bytes declared one byte longer: the size check refuses them.
        assert!(g.read(&Entry { size: 9, ..ab.clone() }).is_err());
        // A compression kind other than stored and LZ4 block.
        let kind3 = Entry { flags: 0x0103_0000, ..a.clone() };
        assert!(g.read(&kind3).unwrap_err().to_string().contains("compression kind 3"));
        let x = g.find("ui/x").unwrap();
        assert!(g.read(x).unwrap_err().to_string().contains("encrypted"));

        assert!(g.oversize.is_empty());

        // A longer container is allowed and noted; a shorter one is an error.
        let mut longer = paz1.clone();
        longer.push(0);
        std::fs::write(dir.path().join("1.paz"), &longer).unwrap();
        assert_eq!(Group::open(dir.path()).unwrap().oversize.len(), 1);
        std::fs::write(dir.path().join("1.paz"), b"short").unwrap();
        assert!(Group::open(dir.path()).is_err());
    }
}
