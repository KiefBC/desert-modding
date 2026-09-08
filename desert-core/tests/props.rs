//! Property tests for the pure byte parsers.
//!
//! The game hands these functions untrusted bytes on every load — a record the
//! loader is about to parse, the mapped image, an ini the player edited — and a
//! panic in the game process is a crash to desktop. So the property that matters
//! everywhere here is "never panics, and what it returns is consistent with what
//! it was given". Each property runs 512 cases.
//!
//! An integration test, so `clippy.toml` exempts it from the unwrap/index/panic
//! lints the shipped code is held to.

use std::collections::HashSet;

use desert_core::gimmick::{self, BLOCK, ITEM_AT, ITEM_TAIL_AT, MAX_AT, MAX_COUNT, MIN_AT};
use desert_core::pattern::Pattern;
use desert_core::schema::Kind;
use desert_core::{ini, pe, rtti, schema};
use proptest::prelude::*;

fn cfg() -> ProptestConfig {
    ProptestConfig { cases: 512, ..ProptestConfig::default() }
}

// ---------------------------------------------------------------------------
// Builders (no slice indexing, so the file needs no allow)
// ---------------------------------------------------------------------------

/// One well-formed resource-output block, laid out field by field.
fn block(item: u32, min: u64, max: u64) -> Vec<u8> {
    let mut b = Vec::with_capacity(BLOCK);
    b.push(1u8); //  +0  flag
    b.extend_from_slice(&[0; 4]); //  +1
    b.extend_from_slice(&item.to_le_bytes()); //  +5  item
    b.extend_from_slice(&[0; 33]); //  +9
    b.extend_from_slice(&min.to_le_bytes()); // +42  min
    b.extend_from_slice(&max.to_le_bytes()); // +50  max
    b.extend_from_slice(&[0xFF, 0xFF]); // +58
    b.extend_from_slice(&[0; 4]); // +60
    b.extend_from_slice(&item.to_le_bytes()); // +64  item again
    b
}

/// A record the scanner must find exactly one output list in: header, filler,
/// `u32 count`, `count` blocks, filler. Returns the record and the offset of
/// the count. Every generator below keeps `0x01` out of the filler, the key,
/// the name and the yields, so the only byte a block can start at is a real
/// block start and the list is located unambiguously.
fn record(key: u32, name: &str, blocks: &[(u32, u64, u64)]) -> (Vec<u8>, usize) {
    let mut r = Vec::new();
    r.extend_from_slice(&key.to_le_bytes());
    r.extend_from_slice(&(name.len() as u32).to_le_bytes());
    r.extend_from_slice(name.as_bytes());
    r.push(0);
    r.extend_from_slice(&[0xAB; 16]);
    let at = r.len();
    r.extend_from_slice(&(blocks.len() as u32).to_le_bytes());
    for &(item, min, max) in blocks {
        r.extend_from_slice(&block(item, min, max));
    }
    r.extend_from_slice(&[0xAB; 8]);
    (r, at)
}

const OPT_SIZE: usize = 240;

/// A PE32+ file whose headers parse but whose section table is whatever the
/// caller passes: the point is to drive `file_to_image` with hostile
/// `(virtual_size, virtual_address, raw_size, raw_offset)` quadruples.
fn pe_file(image_base: u64, size_of_image: u32, sections: &[(u32, u32, u32, u32)]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"MZ");
    b.resize(0x3C, 0);
    b.extend_from_slice(&0x80u32.to_le_bytes()); // e_lfanew
    b.resize(0x80, 0);
    b.extend_from_slice(b"PE\0\0"); // +0
    b.extend_from_slice(&0x8664u16.to_le_bytes()); // +4  machine
    b.extend_from_slice(&(sections.len() as u16).to_le_bytes()); // +6  NumberOfSections
    b.extend_from_slice(&[0; 12]); // +8
    b.extend_from_slice(&(OPT_SIZE as u16).to_le_bytes()); // +20 SizeOfOptionalHeader
    b.extend_from_slice(&[0; 2]); // +22 Characteristics
    let opt = b.len();
    b.extend_from_slice(&0x20Bu16.to_le_bytes()); // +0  PE32+ magic
    b.extend_from_slice(&[0; 22]);
    b.extend_from_slice(&image_base.to_le_bytes()); // +24 ImageBase
    b.extend_from_slice(&[0; 24]);
    b.extend_from_slice(&size_of_image.to_le_bytes()); // +56 SizeOfImage
    b.resize(opt + OPT_SIZE, 0);
    for &(vsize, vaddr, rsize, roff) in sections {
        b.extend_from_slice(b".text\0\0\0"); // +0  name
        b.extend_from_slice(&vsize.to_le_bytes()); // +8
        b.extend_from_slice(&vaddr.to_le_bytes()); // +12
        b.extend_from_slice(&rsize.to_le_bytes()); // +16
        b.extend_from_slice(&roff.to_le_bytes()); // +20
        b.extend_from_slice(&[0; 12]);
        b.extend_from_slice(&0x6000_0020u32.to_le_bytes()); // +36 Characteristics
    }
    b.resize(b.len() + 512, 0xCC); // a little section data to copy
    b
}

/// `Pattern::parse`'s tokenisation, repeated here because `Pattern`'s bytes are
/// private: `None` for the same inputs `Pattern::parse` rejects.
fn tokens(text: &str) -> Option<Vec<Option<u8>>> {
    let mut out = Vec::new();
    for tok in text.split_whitespace() {
        match tok {
            "??" | "?" => out.push(None),
            t if t.len() == 2 => out.push(Some(u8::from_str_radix(t, 16).ok()?)),
            _ => return None,
        }
    }
    if out.is_empty() || out.iter().all(Option::is_none) {
        return None;
    }
    Some(out)
}

fn pattern_text(toks: &[Option<u8>]) -> String {
    toks.iter()
        .map(|t| match t {
            Some(b) => format!("{b:02X}"),
            None => "??".to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// gimmick
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(cfg())]

    /// A header is never read past the end of the record, and the name it
    /// reports is exactly the printable ASCII run at +8.
    #[test]
    fn parse_header_stays_inside_the_record(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        if let Some(h) = gimmick::parse_header(&bytes) {
            let len = h.name.len();
            prop_assert!((1..=255).contains(&len));
            prop_assert_eq!(h.body, 8 + len + 1);
            prop_assert!(h.body <= bytes.len());
            let name = &bytes[8..8 + len];
            prop_assert!(name.iter().all(|&b| (0x20..=0x7E).contains(&b)));
            prop_assert_eq!(name, h.name.as_bytes());
            prop_assert_eq!(bytes[8 + len], 0);
        }
    }

    /// The two content resolvers survive arbitrary bytes and an arbitrary
    /// table name: never a panic, and anything they do return points inside
    /// the image they were given.
    #[test]
    fn manager_slot_resolver_never_panics_and_stays_inside(
        bytes in prop::collection::vec(any::<u8>(), 0..8192),
        name in prop::collection::vec(any::<u8>(), 0..24),
    ) {
        if let Ok(rva) = gimmick::resolve_manager_slot(&bytes, &name) {
            prop_assert!(rva < bytes.len(), "slot rva 0x{:X} outside a {}-byte image", rva, bytes.len());
        }
        const BASE: usize = 0x1_4000_0000;
        if let Ok(va) = gimmick::resolve_record_loader(&bytes, BASE) {
            prop_assert!(va >= BASE);
            prop_assert!(va - BASE < bytes.len());
        }
    }

    /// Every list the scanner reports has a plausible count and fits whole.
    #[test]
    fn output_lists_fit_and_are_bounded(bytes in prop::collection::vec(any::<u8>(), 0..4096)) {
        let lists = gimmick::output_lists(&bytes);
        let mut prev_end = 0usize;
        for (at, count) in lists {
            prop_assert!((1..=MAX_COUNT).contains(&count));
            let end = at + 4 + count as usize * BLOCK;
            prop_assert!(end <= bytes.len());
            prop_assert!(at >= prev_end, "lists must not overlap");
            prev_end = end;
        }
    }

    /// `apply` writes every edit `multiply` produced and nothing else: the only
    /// bytes that change are the eight of each edited `u64`.
    #[test]
    fn multiply_then_apply_touches_only_the_yield_fields(
        bytes in prop::collection::vec(any::<u8>(), 0..4096),
        mult in 1u32..=16,
    ) {
        let edits = gimmick::multiply(&bytes, mult);
        if mult <= 1 {
            prop_assert!(edits.is_empty(), "a multiplier of {} is a no-op", mult);
        }
        let mut copy = bytes.clone();
        prop_assert_eq!(gimmick::apply(&mut copy, &edits), edits.len());
        prop_assert_eq!(copy.len(), bytes.len());

        let mut touched: HashSet<usize> = HashSet::new();
        for e in &edits {
            prop_assert_eq!(e.new, e.old.saturating_mul(u64::from(mult)));
            prop_assert!(e.offset + 8 <= bytes.len());
            prop_assert_eq!(u64::from_le_bytes(bytes[e.offset..e.offset + 8].try_into().unwrap()), e.old);
            prop_assert_eq!(u64::from_le_bytes(copy[e.offset..e.offset + 8].try_into().unwrap()), e.new);
            touched.extend(e.offset..e.offset + 8);
        }
        for (i, (before, after)) in bytes.iter().zip(copy.iter()).enumerate() {
            if !touched.contains(&i) {
                prop_assert_eq!(before, after, "byte {} changed outside an edit", i);
            }
        }
    }

    /// On a record built the way the table stores one, both yields of every
    /// block come out multiplied and nothing else moves.
    #[test]
    fn multiply_scales_every_yield_of_a_well_formed_record(
        key in 2u32..=255,
        name in "[a-z_]{2,20}",
        blocks in prop::collection::vec((2u32..=255, 2u64..=8, 2u64..=8), 1..5),
        mult in 2u32..=16,
    ) {
        let blocks: Vec<(u32, u64, u64)> =
            blocks.into_iter().map(|(i, a, b)| (i, a.min(b), a.max(b))).collect();
        let (rec, at) = record(key, &name, &blocks);
        prop_assert_eq!(gimmick::output_lists(&rec), vec![(at, blocks.len() as u32)]);

        let edits = gimmick::multiply(&rec, mult);
        prop_assert_eq!(edits.len(), blocks.len() * 2);
        let mut copy = rec.clone();
        prop_assert_eq!(gimmick::apply(&mut copy, &edits), edits.len());

        for (n, &(_, min, max)) in blocks.iter().enumerate() {
            let b = at + 4 + n * BLOCK;
            for (field, want) in [(MIN_AT, min), (MAX_AT, max)] {
                let o = b + field;
                let got = u64::from_le_bytes(copy[o..o + 8].try_into().unwrap());
                // `multiply` saturates; these values are far from the ceiling,
                // so the saturating product is the plain one.
                prop_assert_eq!(got, want.saturating_mul(u64::from(mult)));
                prop_assert_eq!(got, want * u64::from(mult));
            }
        }
    }

    /// `output_blocks` is the read-only view of exactly the blocks `multiply`
    /// edits: same blocks, same order, and the pair of edits for block `n` is
    /// its two scalars at their two offsets, carrying its two vanilla values.
    /// That correspondence is what the live re-apply path stands on - it
    /// rebuilds the edit from a remembered block long after the bytes are gone.
    #[test]
    fn output_blocks_are_the_blocks_multiply_edits(
        bytes in prop::collection::vec(any::<u8>(), 0..4096),
        mult in 2u32..=16,
    ) {
        let blocks = gimmick::output_blocks(&bytes);
        let edits = gimmick::multiply(&bytes, mult);
        prop_assert_eq!(edits.len(), blocks.len() * 2);

        let mut prev_end = 0usize;
        for (n, b) in blocks.iter().enumerate() {
            // Inside the buffer, whole, and never overlapping the block before.
            prop_assert!(b.offset >= prev_end);
            prop_assert!(b.offset + BLOCK <= bytes.len());
            prev_end = b.offset + BLOCK;

            // The signature keeps both copies of the item id in step, so the
            // one at ITEM_TAIL_AT is the one at ITEM_AT.
            let tail = u32::from_le_bytes(
                bytes[b.offset + ITEM_TAIL_AT..b.offset + ITEM_TAIL_AT + 4].try_into().unwrap());
            let head = u32::from_le_bytes(
                bytes[b.offset + ITEM_AT..b.offset + ITEM_AT + 4].try_into().unwrap());
            prop_assert_eq!(b.item, tail);
            prop_assert_eq!(b.item, head);
            prop_assert!(1 <= b.min && b.min <= b.max && b.max <= gimmick::MAX_QTY);

            let (lo, hi) = (&edits[n * 2], &edits[n * 2 + 1]);
            prop_assert_eq!(lo.offset, b.offset + MIN_AT);
            prop_assert_eq!(hi.offset, b.offset + MAX_AT);
            prop_assert_eq!(lo.old, b.min);
            prop_assert_eq!(hi.old, b.max);
            prop_assert_eq!(lo.new, b.min.saturating_mul(u64::from(mult)));
            prop_assert_eq!(hi.new, b.max.saturating_mul(u64::from(mult)));
        }
    }
}

// ---------------------------------------------------------------------------
// pe
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(cfg())]

    /// Arbitrary bytes are rejected, not parsed off the end.
    #[test]
    fn pe_parse_and_layout_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..8192)) {
        if let Some(h) = pe::parse(&bytes) {
            if let Some(img) = pe::file_to_image(&bytes) {
                prop_assert_eq!(img.len(), h.size_of_image as usize);
            }
        }
    }

    /// A parseable PE whose section table is nonsense still lays out: every
    /// out-of-range or overflowing section is skipped, not copied.
    #[test]
    fn pe_layout_survives_a_hostile_section_table(
        image_base in any::<u64>(),
        size_of_image in 0u32..0x20000,
        sections in prop::collection::vec(
            (any::<u32>(), any::<u32>(), any::<u32>(), any::<u32>()), 0..6),
    ) {
        let f = pe_file(image_base, size_of_image, &sections);
        let h = pe::parse(&f).expect("the synthetic headers parse");
        prop_assert_eq!(h.image_base, image_base);
        prop_assert_eq!(h.sections.len(), sections.len());
        let img = pe::file_to_image(&f).expect("layout of a parseable file");
        prop_assert_eq!(img.len(), size_of_image as usize);
    }
}

// ---------------------------------------------------------------------------
// rtti
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(cfg())]

    /// The three RTTI scanners walk arbitrary image bytes without panicking and
    /// only ever report offsets inside the image.
    #[test]
    fn rtti_scanners_stay_inside_the_image(
        img in prop::collection::vec(any::<u8>(), 0..4096),
        name in "[.@?A-Za-z0-9]{0,12}",
        td_rva in 0usize..4096,
        image_base in any::<u64>(),
        col_rva in 0usize..4096,
    ) {
        for td in rtti::find_type_descriptors(&img, &name) {
            prop_assert!(td + 0x10 < img.len());
        }
        for col in rtti::find_object_locators(&img, td_rva) {
            prop_assert!(col + 24 <= img.len());
        }
        let _ = rtti::find_vtables(&img, image_base, col_rva);
        let _ = rtti::vtables_for_class(&img, image_base, &name);
    }
}

// ---------------------------------------------------------------------------
// pattern
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(cfg())]

    /// Any string is either a pattern or a `None`, never a panic.
    #[test]
    fn pattern_parse_never_panics(text in ".{0,64}") {
        prop_assert_eq!(Pattern::parse(&text).is_some(), tokens(&text).is_some());
    }

    /// `find_all` honours its limit and every hit is a real match: in range, and
    /// equal to the pattern on each of its literal positions.
    #[test]
    fn pattern_find_all_respects_limit_and_hits_match(
        toks in prop::collection::vec(prop::option::of(any::<u8>()), 1..12),
        hay in prop::collection::vec(any::<u8>(), 0..512),
        limit in 0usize..8,
    ) {
        let text = pattern_text(&toks);
        let Some(p) = Pattern::parse(&text) else { return Ok(()) };
        let hits = p.find_all(&hay, limit);
        prop_assert!(hits.len() <= limit);
        for &o in &hits {
            prop_assert!(o < hay.len());
            prop_assert!(o + p.len() <= hay.len());
            for (i, t) in toks.iter().enumerate() {
                if let Some(want) = t {
                    prop_assert_eq!(hay[o + i], *want, "hit {} differs at +{}", o, i);
                }
            }
        }
    }

    /// The anchored scan finds exactly what a naive offset-by-offset scan finds.
    #[test]
    fn pattern_find_all_agrees_with_a_naive_scan(
        toks in prop::collection::vec(prop::option::of(0u8..4), 1..8),
        hay in prop::collection::vec(0u8..4, 0..400),
    ) {
        let text = pattern_text(&toks);
        let Some(p) = Pattern::parse(&text) else { return Ok(()) };
        let naive: Vec<usize> = (0..hay.len().saturating_sub(toks.len() - 1))
            .filter(|&o| toks.iter().enumerate().all(|(i, t)| t.is_none_or(|w| hay[o + i] == w)))
            .collect();
        prop_assert_eq!(p.find_all(&hay, usize::MAX), naive.clone());
        prop_assert_eq!(p.find_all(&hay, 3), naive.iter().copied().take(3).collect::<Vec<_>>());
    }
}

// ---------------------------------------------------------------------------
// ini
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(cfg())]

    /// Any text tokenises, and every pair really came from a `key=value` line.
    #[test]
    fn ini_lines_and_key_names_never_panic(text in ".{0,256}") {
        for line in ini::lines(&text) {
            if let ini::Line::Pair(k, v) = line {
                prop_assert_eq!(k, k.trim());
                prop_assert_eq!(v, v.trim());
                prop_assert!(!k.contains('='));
            }
        }
        let _ = ini::vk_from_name(&text);
        let _ = ini::parse_bool(&text);
    }
}

// ---------------------------------------------------------------------------
// schema
// ---------------------------------------------------------------------------

/// The value texts a generated field block picks its `Default` from: some are
/// right for one kind, most are wrong for every kind.
const DEFAULTS: &[&str] = &["1", "0", "40", "6.5", "-3", "F10", "num5", "banner", "x", ""];
const BOUNDS: &[&str] = &["0", "1", "-5", "200", "60000", "x", ""];
const OPTION_LISTS: &[&str] = &["classic;parchment;banner", "a;b", "banner", ";;", ""];
const KINDS: &[&str] = &["bool", "int", "float", "choice", "key", "colour", ""];

/// One `[<key>]` block, occasionally a well-formed one.
fn field_block() -> impl Strategy<Value = String> {
    (
        "[A-Za-z][A-Za-z0-9_]{0,6}",
        prop::sample::select(KINDS),
        prop::sample::select(DEFAULTS),
        prop::sample::select(BOUNDS),
        prop::sample::select(BOUNDS),
        prop::sample::select(OPTION_LISTS),
        any::<(bool, bool)>(),
    )
        .prop_map(|(key, kind, default, min, max, options, (same_line, slider))| {
            let mut b = format!("[{key}]\nKind={kind}\nDefault={default}\nMin={min}\nMax={max}\n");
            b.push_str(&format!("Options={options}\n"));
            if same_line {
                b.push_str("SameLine=1\nHeading=Group:\nHelp=What it does.\n");
            }
            if slider {
                b.push_str("Widget=slider\nFormat=%dx\nLabel=A label\n");
            }
            b
        })
}

/// One `[preset:<label>]` block; its `Set` is as likely to name a field that
/// does not exist as one that does.
fn preset_block() -> impl Strategy<Value = String> {
    ("[A-Za-z ]{0,6}", "[A-Za-z0-9_=;. ]{0,24}", any::<bool>()).prop_map(|(label, set, hint)| {
        let mut b = format!("[preset:{label}]\nSet={set}\n");
        if hint {
            b.push_str("Hint=A hint.\n");
        }
        b
    })
}

/// Plausible schema text: a header that is right about as often as it is wrong,
/// then some field and preset blocks.
fn schema_text() -> impl Strategy<Value = String> {
    (
        "[A-Za-z ]{0,10}",
        prop::sample::select(vec!["DesertLooter.ini", "T.ini", "sub/T.ini", "..\\T.ini", "T", ""]),
        0u32..3,
        prop::collection::vec(field_block(), 0..5),
        prop::collection::vec(preset_block(), 0..3),
        any::<bool>(),
    )
        .prop_map(|(title, ini, schema, fields, presets, module)| {
            let mut t = format!("; a banner\n[overlay]\nSchema={schema}\nTitle={title}\nIni={ini}\n");
            t.push_str("Order=10\nNotice=A notice.\nPresetsLabel=Presets:\n");
            if module {
                t.push_str("Module=DesertLooter.asi\n");
            }
            for b in fields.iter().chain(presets.iter()) {
                t.push('\n');
                t.push_str(b);
            }
            t
        })
}

/// Everything a parsed section promises, checked on the section itself and on
/// the text `render` writes for it.
fn check_section(s: &schema::Section) -> Result<(), TestCaseError> {
    prop_assert!(s.schema_file_name().ends_with(schema::FILE_SUFFIX));
    prop_assert!(!s.ini.is_empty() && !s.title.is_empty());
    for f in &s.fields {
        // a field's own default is always a value it accepts
        let text = f.kind.default_text();
        let round = f.kind.normalize(&text);
        prop_assert_eq!(round.as_deref(), Some(text.as_str()));
        prop_assert_eq!(s.field(&f.key.to_ascii_lowercase()), Some(f));
    }
    for p in &s.presets {
        for (k, v) in &p.set {
            let Some(f) = s.field(k) else {
                return Err(TestCaseError::fail(format!("preset names unknown field {k:?}")));
            };
            let round = f.kind.normalize(v);
            prop_assert_eq!(round.as_deref(), Some(v.as_str()));
        }
    }
    // the ini the overlay would create from this schema reads back as itself
    for line in ini::lines(&schema::render_ini_defaults(s, "")) {
        if let ini::Line::Pair(k, v) = line {
            let Some(f) = s.field(k) else {
                return Err(TestCaseError::fail(format!("default ini line {k:?} is not a field")));
            };
            let round = f.kind.normalize(v);
            prop_assert_eq!(round.as_deref(), Some(v));
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(cfg())]

    /// Arbitrary text is either a rejected file or a section, never a panic;
    /// and what `render` writes for a section parses back to the same section.
    #[test]
    fn schema_parse_never_panics_and_render_round_trips(text in "(?s).{0,512}") {
        if let Ok((s, _)) = schema::parse(&text) {
            check_section(&s)?;
            let again = schema::parse(&schema::render(&s, "banner\nlines")).map(|(s, _)| s);
            prop_assert_eq!(again, Ok(s));
        }
    }

    /// The same, on text shaped like a schema file, so the `Ok` branch is the
    /// one being exercised.
    #[test]
    fn plausible_schema_text_round_trips(text in schema_text()) {
        if let Ok((s, _)) = schema::parse(&text) {
            check_section(&s)?;
            let again = schema::parse(&schema::render(&s, "banner")).map(|(s, _)| s);
            prop_assert_eq!(again, Ok(s));
        }
    }

    /// Every kind takes arbitrary value text without panicking, and what it
    /// gives back it accepts again.
    #[test]
    fn kind_normalize_never_panics(
        text in "(?s).{0,64}",
        (default, min, max) in (any::<i64>(), any::<i64>(), any::<i64>()),
        (fdefault, fmin, fmax) in (any::<f32>(), any::<f32>(), any::<f32>()),
    ) {
        let kinds = [
            Kind::Bool { default: default > 0 },
            Kind::Int { default, min, max, step: 1, slider: false, format: None },
            Kind::Float { default: fdefault, min: fmin, max: fmax, format: None },
            Kind::Choice { default: text.clone(), options: vec![text.clone(), "a".to_string()] },
            Kind::Key { default: "F10".to_string() },
        ];
        for kind in &kinds {
            let _ = kind.default_text();
            if let Some(once) = kind.normalize(&text) {
                prop_assert_eq!(kind.normalize(&once), Some(once.clone()));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ini::entries
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(cfg())]

    /// `entries` is `lines` plus the section headers: every entry that is not a
    /// header is the line `lines` reports at that position, in the same order.
    #[test]
    fn ini_entries_agree_with_lines(text in "(?s).{0,256}") {
        let mut want = ini::lines(&text);
        let mut sections = 0;
        for entry in ini::entries(&text) {
            match entry {
                ini::Entry::Section(name) => {
                    prop_assert_eq!(name, name.trim());
                    prop_assert!(!name.contains('\n'));
                    sections += 1;
                }
                ini::Entry::Pair(k, v) => {
                    prop_assert_eq!(k, k.trim());
                    prop_assert!(!k.contains('='));
                    prop_assert_eq!(want.next(), Some(ini::Line::Pair(k, v)));
                }
                ini::Entry::Bad(why) => prop_assert_eq!(want.next(), Some(ini::Line::Bad(why))),
            }
        }
        prop_assert_eq!(want.next(), None);
        prop_assert_eq!(ini::entries(&text).count(), ini::lines(&text).count() + sections);
    }

    /// Every name the picker lists resolves, and canonicalising is idempotent.
    #[test]
    fn key_names_resolve_and_canonicalise(text in "(?s).{0,32}") {
        for name in ini::key_names() {
            prop_assert!(ini::vk_from_name(name).is_some());
        }
        if let Some(canon) = ini::canonical_key_name(&text) {
            prop_assert!(ini::key_names().contains(&canon.as_str()));
            prop_assert_eq!(ini::canonical_key_name(&canon), Some(canon.clone()));
        }
    }
}
