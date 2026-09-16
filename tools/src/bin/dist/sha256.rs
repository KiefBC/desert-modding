//! SHA-256, because `dist` publishes checksums and nothing else here needs one.
//!
//! The shell version piped the zips through `sha256sum`, which is one more
//! thing that has to be on PATH for a release to build. There is no hash crate
//! in `tools/Cargo.toml` and this is why: FIPS 180-4 is sixty lines, it never
//! changes, and the alternative is a dependency (plus its transitive tree) in
//! the supply chain of the artefacts people download. The tests below check it
//! against the published FIPS vectors, and `tests/dist.rs` checks a real zip's
//! digest against coreutils' `sha256sum`, so "I wrote my own crypto" here means
//! "I wrote the one function every implementation agrees byte for byte on, and
//! proved it agrees".

/// The 64 round constants: the fractional parts of the cube roots of the first
/// 64 primes (FIPS 180-4 section 4.2.2).
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// A streaming SHA-256. Streaming rather than one-shot so a multi-megabyte
/// `.asi` is hashed as it is read instead of a second copy of it living in
/// memory beside the one the zip writer already holds.
pub struct Sha256 {
    state: [u32; 8],
    /// Bytes not yet part of a full 64-byte block.
    buf: [u8; 64],
    buffered: usize,
    /// Total message length, in bits: the value the padding ends with.
    bits: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    pub fn new() -> Self {
        Self {
            // Fractional parts of the square roots of the first eight primes.
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buf: [0; 64],
            buffered: 0,
            bits: 0,
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.bits = self.bits.wrapping_add((data.len() as u64) * 8);
        if self.buffered > 0 {
            let take = (64 - self.buffered).min(data.len());
            self.buf[self.buffered..self.buffered + take].copy_from_slice(&data[..take]);
            self.buffered += take;
            data = &data[take..];
            if self.buffered < 64 {
                // The whole call fitted inside the partial block. Returning
                // here rather than falling through is not an optimisation: the
                // tail handling below assigns `buffered` outright, so carrying
                // on would set it to zero and silently drop the bytes just
                // copied in. `streaming_matches_one_shot` is what caught that.
                return;
            }
            let block = self.buf;
            self.compress(&block);
            self.buffered = 0;
        }
        // `as_chunks` rather than `chunks_exact(64)`: it hands back
        // `&[[u8; 64]]`, so each block is already the fixed-size array
        // `compress` wants and there is no per-block copy into a temporary.
        // Clippy's `chunks_exact_to_as_chunks` asks for this, and only the
        // dev shell's toolchain is new enough to say so - see the note in
        // tools/README.md about linting under the pinned Rust, not the
        // system one.
        let (blocks, rest) = data.as_chunks::<64>();
        for block in blocks {
            self.compress(block);
        }
        self.buf[..rest.len()].copy_from_slice(rest);
        self.buffered = rest.len();
    }

    pub fn finish(mut self) -> [u8; 32] {
        // Padding: 0x80, then zeroes, then the bit length big-endian, arranged
        // so the total is a whole number of blocks. The length is captured
        // before padding is appended, which is why `update` must not be used
        // to append it.
        let bits = self.bits;
        let mut tail = [0u8; 72];
        tail[0] = 0x80;
        let pad = if self.buffered < 56 {
            56 - self.buffered
        } else {
            120 - self.buffered
        };
        tail[pad..pad + 8].copy_from_slice(&bits.to_be_bytes());
        let n = pad + 8;
        // Re-entering `update` would double-count these bytes in `bits`; undo
        // that rather than duplicating the buffering logic.
        self.update(&tail[..n]);
        self.bits = bits;

        let mut out = [0u8; 32];
        for (i, word) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (s, v) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }
}

/// Lowercase hex, the form `sha256sum` prints and `--check` parses.
pub fn hex(digest: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(data: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(data);
        hex(&h.finish())
    }

    /// The vectors from FIPS 180-4's appendix and NIST's byte-oriented test
    /// set. The empty and one-block cases would pass for an implementation
    /// with a broken multi-block carry; the 56-byte and 1 000 000-byte ones
    /// are the ones that catch it.
    #[test]
    fn fips_vectors() {
        assert_eq!(
            digest(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            digest(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            digest(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            digest(b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu"),
            "cf5b16a778af8380036ce59e7b0492370b249b11e8f07a51afac45037afee9d1"
        );
        let million = vec![b'a'; 1_000_000];
        assert_eq!(
            digest(&million),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    /// Every split of a message across two `update` calls must give the same
    /// digest as one. The padding block boundary (55/56/64 bytes) is where a
    /// hand-written implementation goes wrong, and a `dist` that hashed the
    /// first megabyte of an `.asi` correctly and the rest not would publish a
    /// SHA256SUMS that `sha256sum --check` rejects only after the release is
    /// already tagged.
    #[test]
    fn streaming_matches_one_shot() {
        let message: Vec<u8> = (0..200u32).map(|i| (i % 251) as u8).collect();
        let whole = digest(&message);
        for split in 0..message.len() {
            let mut h = Sha256::new();
            h.update(&message[..split]);
            h.update(&message[split..]);
            assert_eq!(hex(&h.finish()), whole, "split at {split}");
        }
    }
}
