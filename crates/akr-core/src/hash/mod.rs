//! SHA-256, and the three hashes the design set defines.
//!
//! # Why SHA-256 lives here
//!
//! `docs/13-implementation-roadmap.md` §4 lists SHA-256 among the approved dependencies.
//! It is implemented in-crate rather than pulled in because the implementation is eighty
//! lines of arithmetic against a fixed test vector set, and because P1's "no runtime
//! dependencies" property is worth keeping for one more phase. Swapping in `sha2` later
//! is a change to [`Sha256`] alone; nothing outside this module knows how a digest is
//! produced.
//!
//! # The three hashes
//!
//! All three are specified in `spec/schema/akr-lock.md` §3 and
//! `docs/06-compiler-pipeline.md` §9.
//!
//! | Hash | Input | Function |
//! | --- | --- | --- |
//! | Source file (§3.1) | the file's **raw bytes on disk** | [`source_file_hash`] |
//! | Source graph (§3.2) | sorted `(path, file-hash)` pairs | [`source_graph_hash`] |
//! | Revision content (§3.3) | the **canonically formatted text of one record**, comment trivia removed | [`content_hash`] |
//!
//! [`content_hash`] takes text rather than a [`Record`](crate::model::Record) on purpose.
//! Canonical text is the formatter's output (phase P2), so the hash is defined over what
//! the formatter produces and this module never has to know how a record is written. See
//! [`canonical_hash_input`] for the one normalisation this module does apply.

use crate::model::ContentHash;
use std::fmt;

// -------------------------------------------------------------------------------------
// SHA-256
// -------------------------------------------------------------------------------------

const K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

const H0: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

/// A 32-byte SHA-256 digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest([u8; 32]);

impl Digest {
    /// The raw bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The digest as 64 lowercase hex digits, with no prefix.
    #[must_use]
    pub fn to_hex(self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            out.push(char::from_digit(u32::from(byte >> 4), 16).expect("nibble"));
            out.push(char::from_digit(u32::from(byte & 0x0f), 16).expect("nibble"));
        }
        out
    }

    /// The digest in the form the lock and the view banners use: `sha256:` plus 64 hex.
    #[must_use]
    pub fn to_content_hash(self) -> ContentHash {
        ContentHash(format!("sha256:{}", self.to_hex()))
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// An incremental SHA-256 hasher (FIPS 180-4).
#[derive(Debug, Clone)]
pub struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffered: usize,
    length: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    /// A fresh hasher.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: H0,
            buffer: [0u8; 64],
            buffered: 0,
            length: 0,
        }
    }

    /// Feeds bytes in.
    pub fn update(&mut self, data: &[u8]) {
        self.length = self.length.wrapping_add(data.len() as u64);
        let mut rest = data;
        if self.buffered > 0 {
            let want = 64 - self.buffered;
            let take = want.min(rest.len());
            self.buffer[self.buffered..self.buffered + take].copy_from_slice(&rest[..take]);
            self.buffered += take;
            rest = &rest[take..];
            if self.buffered == 64 {
                let block = self.buffer;
                self.compress(&block);
                self.buffered = 0;
            }
        }
        #[allow(clippy::chunks_exact_to_as_chunks)]
        let mut chunks = rest.chunks_exact(64);
        for chunk in &mut chunks {
            let mut block = [0u8; 64];
            block.copy_from_slice(chunk);
            self.compress(&block);
        }
        let tail = chunks.remainder();
        if !tail.is_empty() {
            self.buffer[..tail.len()].copy_from_slice(tail);
            self.buffered = tail.len();
        }
    }

    /// Finishes and returns the digest.
    #[must_use]
    pub fn finish(mut self) -> Digest {
        let bits = self.length.wrapping_mul(8);
        self.update_raw(&[0x80]);
        while self.buffered != 56 {
            self.update_raw(&[0x00]);
        }
        self.update_raw(&bits.to_be_bytes());
        let mut out = [0u8; 32];
        for (i, word) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        Digest(out)
    }

    /// Feeds padding bytes without counting them in the length.
    fn update_raw(&mut self, data: &[u8]) {
        for &byte in data {
            self.buffer[self.buffered] = byte;
            self.buffered += 1;
            if self.buffered == 64 {
                let block = self.buffer;
                self.compress(&block);
                self.buffered = 0;
            }
        }
    }

    #[allow(clippy::many_single_char_names)]
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
            let ch = (e & f) ^ ((!e) & g);
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
        for (slot, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }
}

/// SHA-256 of a byte slice.
#[must_use]
pub fn sha256(data: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finish()
}

/// SHA-256 of a byte slice, as 64 lowercase hex digits.
#[must_use]
pub fn sha256_hex(data: &[u8]) -> String {
    sha256(data).to_hex()
}

// -------------------------------------------------------------------------------------
// The three AKR hashes
// -------------------------------------------------------------------------------------

/// The source file hash: SHA-256 over the file's **raw bytes on disk**
/// (`spec/schema/akr-lock.md` §3.1).
///
/// Raw bytes, not canonical form, because this hash answers "are the inputs on disk the
/// same inputs?" — a question about the filesystem, not about meaning. A file that is not
/// canonical fails `akr fmt --check` before the lock is ever consulted.
#[must_use]
pub fn source_file_hash(bytes: &[u8]) -> ContentHash {
    sha256(bytes).to_content_hash()
}

/// The source-graph hash: SHA-256 over `path NUL file-hash LF` for every source file,
/// sorted bytewise by path (`spec/schema/akr-lock.md` §3.2).
///
/// The `file-hash` written into the serialisation is the full `sha256:`-prefixed form, as
/// it appears in the lock. Entries are sorted here rather than trusted from the caller,
/// so that a caller which walks the filesystem in any order still gets the one right
/// answer.
#[must_use]
pub fn source_graph_hash<'a>(
    entries: impl IntoIterator<Item = (&'a str, &'a ContentHash)>,
) -> ContentHash {
    let mut pairs: Vec<(&str, &ContentHash)> = entries.into_iter().collect();
    pairs.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    pairs.dedup_by(|a, b| a.0 == b.0);

    let mut hasher = Sha256::new();
    for (path, hash) in pairs {
        hasher.update(path.as_bytes());
        hasher.update(&[0x00]);
        hasher.update(hash.0.as_bytes());
        hasher.update(b"\n");
    }
    hasher.finish().to_content_hash()
}

/// The revision content hash: SHA-256 over the canonically formatted text of one record
/// (`spec/schema/akr-lock.md` §3.3).
///
/// The input is the record's text alone — from the `record` keyword through its closing
/// brace inclusive — not the file it sits in. Surrounding file content is excluded by
/// construction: moving a record between files, or reordering it, cannot change its hash,
/// because the surrounding text is never passed in.
///
/// Comment trivia is removed before hashing, by [`canonical_hash_input`]. Adding a
/// clarifying comment to a sealed record must not trip `AKR-R051`, or people stop writing
/// comments — which is the opposite of what the format wants.
///
/// # Seam
///
/// Canonical text comes from the phase P2 formatter. Until that lands, callers pass text
/// that is already canonical, which every committed `.akr` file is.
#[must_use]
pub fn content_hash(canonical_record_text: &str) -> ContentHash {
    sha256(canonical_hash_input(canonical_record_text).as_bytes()).to_content_hash()
}

/// Normalises canonical record text for hashing: removes comment trivia, drops the lines
/// that were nothing but a comment, strips trailing whitespace, and guarantees exactly one
/// trailing newline.
///
/// # What counts as a comment
///
/// `#` to end of line (D-006), but only where a `#` can start one: at the beginning of a
/// line, or preceded by whitespace, and not inside a quoted string or a `"""` prose block.
/// A `#` immediately preceded by a non-space character is part of a reference anchor
/// (`@key/2#anchor`) and is left alone.
///
/// # Seam
///
/// This is a deliberately small lexical pass, not a parser. When the P2 lexer lands, the
/// comment-stripping step should be replaced by "re-emit the token stream without trivia",
/// which is the same answer arrived at more cheaply. The property tests in
/// `tests/hashing.rs` pin the behaviour so the swap is checkable.
#[must_use]
pub fn canonical_hash_input(canonical_record_text: &str) -> String {
    let mut out = String::with_capacity(canonical_record_text.len());
    let mut in_prose = false;

    for line in canonical_record_text.lines() {
        if in_prose {
            out.push_str(line.trim_end());
            out.push('\n');
            if line.contains("\"\"\"") {
                in_prose = false;
            }
            continue;
        }

        let (code, had_comment) = strip_line_comment(line);
        let trimmed = code.trim_end();

        // A line that was nothing but a comment disappears entirely; a line that had a
        // trailing comment keeps its code.
        if had_comment && trimmed.is_empty() {
            continue;
        }
        out.push_str(trimmed);
        out.push('\n');

        if opens_prose(trimmed) {
            in_prose = true;
        }
    }

    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Whether this line opens a prose block that a later line closes.
///
/// A line with an odd number of `"""` markers leaves prose open.
fn opens_prose(line: &str) -> bool {
    line.matches("\"\"\"").count() % 2 == 1
}

/// Splits a line at the start of its comment, if it has one.
///
/// Returns the code portion and whether a comment was removed. Quoted strings are
/// respected, so a `#` inside `"..."` is not a comment.
fn strip_line_comment(line: &str) -> (&str, bool) {
    let bytes = line.as_bytes();
    let mut in_string = false;
    let mut escaped = false;
    let mut i = 0;

    while i < bytes.len() {
        let byte = bytes[i];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match byte {
            b'"' => {
                // A `"""` marker opens prose rather than a string; leave the rest alone.
                if bytes[i..].starts_with(b"\"\"\"") {
                    return (line, false);
                }
                in_string = true;
            }
            b'#' => {
                let preceded_by_space = i == 0 || bytes[i - 1].is_ascii_whitespace();
                if preceded_by_space {
                    return (&line[..i], true);
                }
            }
            _ => {}
        }
        i += 1;
    }
    (line, false)
}
