//! Hashing: SHA-256 itself, and the three hashes of `spec/schema/akr-lock.md` §3.
//!
//! The interesting tests here are not "does SHA-256 work" — that is three known vectors —
//! but the *exclusions*. A content hash that changed when a comment was added, or when a
//! record moved between files, would make `AKR-R051` fire on edits that changed nothing,
//! and people would stop writing comments and stop tidying files. Those properties are
//! what these tests pin.

use akr_core::hash::{
    Sha256, canonical_hash_input, content_hash, sha256_hex, source_file_hash, source_graph_hash,
};
use akr_core::model::ContentHash;

// -------------------------------------------------------------------------------------
// SHA-256
// -------------------------------------------------------------------------------------

#[test]
fn known_vectors() {
    // FIPS 180-4 / RFC 6234 test vectors.
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
}

#[test]
fn long_input_crosses_block_boundaries() {
    // A million 'a' characters, the classic multi-block vector.
    let mut hasher = Sha256::new();
    for _ in 0..1_000 {
        hasher.update(&[b'a'; 1_000]);
    }
    assert_eq!(
        hasher.finish().to_hex(),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[test]
fn incremental_matches_one_shot() {
    let data: Vec<u8> = (0u8..=255).cycle().take(1_000).collect();
    for split in [0usize, 1, 63, 64, 65, 127, 128, 500, 999, 1_000] {
        let mut hasher = Sha256::new();
        hasher.update(&data[..split]);
        hasher.update(&data[split..]);
        assert_eq!(
            hasher.finish().to_hex(),
            sha256_hex(&data),
            "split at {split}"
        );
    }
}

#[test]
fn digest_renders_with_the_sha256_prefix() {
    let hash = source_file_hash(b"");
    assert_eq!(
        hash.0,
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(hash.0.len(), "sha256:".len() + 64);
}

// -------------------------------------------------------------------------------------
// §3.1 source file hash — raw bytes on disk
// -------------------------------------------------------------------------------------

#[test]
fn source_file_hash_is_over_raw_bytes() {
    // Not canonical form: this hash answers "are the inputs on disk the same inputs?".
    // Trailing whitespace a formatter would strip must still change it.
    let a = source_file_hash(b"akr 0.1\nproject p\n");
    let b = source_file_hash(b"akr 0.1\nproject p  \n");
    assert_ne!(a, b);
}

// -------------------------------------------------------------------------------------
// §3.2 source-graph hash
// -------------------------------------------------------------------------------------

#[test]
fn source_graph_hash_is_order_independent() {
    let a = ContentHash("sha256:aa".to_owned());
    let b = ContentHash("sha256:bb".to_owned());
    let forwards = source_graph_hash([(".akr/a.akr", &a), (".akr/b.akr", &b)]);
    let backwards = source_graph_hash([(".akr/b.akr", &b), (".akr/a.akr", &a)]);
    assert_eq!(
        forwards, backwards,
        "entries are sorted here, not trusted from the caller"
    );
}

#[test]
fn source_graph_hash_changes_with_any_input() {
    let a = ContentHash("sha256:aa".to_owned());
    let b = ContentHash("sha256:bb".to_owned());
    let base = source_graph_hash([(".akr/a.akr", &a)]);
    assert_ne!(
        base,
        source_graph_hash([(".akr/a.akr", &b)]),
        "hash changed"
    );
    assert_ne!(
        base,
        source_graph_hash([(".akr/z.akr", &a)]),
        "path changed"
    );
    assert_ne!(
        base,
        source_graph_hash([(".akr/a.akr", &a), (".akr/b.akr", &b)]),
        "file added"
    );
}

#[test]
fn source_graph_serialisation_is_nul_separated() {
    // §3.2: `<path> NUL <file-hash> LF`. Pinned by construction so that a future
    // implementation cannot quietly pick a different separator.
    let hash = ContentHash("sha256:aa".to_owned());
    let expected = {
        let mut hasher = Sha256::new();
        hasher.update(b"p.akr");
        hasher.update(&[0x00]);
        hasher.update(b"sha256:aa");
        hasher.update(b"\n");
        hasher.finish().to_content_hash()
    };
    assert_eq!(source_graph_hash([("p.akr", &hash)]), expected);
}

#[test]
fn separator_prevents_path_ambiguity() {
    // Without the NUL, ("ab", "c") and ("a", "bc") would serialise identically.
    let one = ContentHash("sha256:c".to_owned());
    let two = ContentHash("sha256:bc".to_owned());
    assert_ne!(
        source_graph_hash([("ab", &one)]),
        source_graph_hash([("a", &two)])
    );
}

// -------------------------------------------------------------------------------------
// §3.3 revision content hash — the exclusions
// -------------------------------------------------------------------------------------

const RECORD: &str = "\
record fx.policy.a/1 : policy {
    title \"A policy\"
    state active
    scope [ all ]
    rule \"\"\"
        The rule.
        \"\"\"
}
";

#[test]
fn content_hash_excludes_comment_trivia() {
    // §3.3: adding a clarifying comment to a sealed record must not trip AKR-R051, or
    // people stop writing comments.
    let leading = "# Why this rule exists.\n".to_owned() + RECORD;
    let trailing = RECORD.replace("state active", "state active  # still in force");
    let inner = RECORD.replace(
        "    scope [ all ]\n",
        "    # project-wide on purpose\n    scope [ all ]\n",
    );
    assert_eq!(content_hash(RECORD), content_hash(&leading));
    assert_eq!(content_hash(RECORD), content_hash(&trailing));
    assert_eq!(content_hash(RECORD), content_hash(&inner));
}

#[test]
fn content_hash_includes_content() {
    let edited = RECORD.replace("The rule.", "The rule, amended.");
    assert_ne!(content_hash(RECORD), content_hash(&edited));
}

#[test]
fn content_hash_excludes_surrounding_file_content() {
    // Identity is the key, never the file (D-018). The function takes one record's text,
    // so moving a record between files cannot reach it — asserted rather than assumed.
    let sibling = RECORD.replace("fx.policy.a", "fx.policy.b");
    let first = format!("{RECORD}\n{sibling}");
    let second = format!("{sibling}\n{RECORD}");
    let extract = |text: &str| {
        let start = text.find("record fx.policy.a/1").expect("record present");
        let end = text[start..].find("\n}\n").expect("record ends") + start + 3;
        text[start..end].to_owned()
    };
    assert_eq!(
        content_hash(&extract(&first)),
        content_hash(&extract(&second))
    );
    assert_eq!(content_hash(&extract(&first)), content_hash(RECORD));
}

#[test]
fn hash_input_keeps_hash_characters_that_are_not_comments() {
    // `@key/2#anchor` is a reference, not a comment: the `#` is preceded by a letter.
    let with_anchor = "    supported_by [ @fx.obs.a/1#finding ]\n";
    assert_eq!(canonical_hash_input(with_anchor), with_anchor);
}

#[test]
fn hash_input_keeps_hash_characters_inside_strings() {
    let with_string = "    command \"grep '# TODO' src\"\n";
    assert_eq!(canonical_hash_input(with_string), with_string);
}

#[test]
fn hash_input_keeps_hash_characters_inside_prose() {
    // Prose is raw (D-007). A `#` in it is text, and stripping it would corrupt content.
    let prose = "\
record fx.policy.a/1 : policy {
    rule \"\"\"
        Use # to start a comment.
        \"\"\"
}
";
    assert!(canonical_hash_input(prose).contains("Use # to start a comment."));
}

#[test]
fn hash_input_ends_with_exactly_one_newline() {
    for text in [RECORD, &format!("{RECORD}\n\n\n"), RECORD.trim_end()] {
        let normalised = canonical_hash_input(text);
        assert!(normalised.ends_with('\n'));
        assert!(!normalised.ends_with("\n\n"), "input {text:?}");
    }
}
