//! V-105: whether a recorded command names anything real (`AKR-G024`).
//!
//! Freshness asks whether a record can still go stale. This asks a different question —
//! whether a command the ledger records as a gate would execute anything if you ran it —
//! so it lives apart rather than sharing a module with freshness because both happen to
//! walk the ledger.
//!
//! The gate being protected is D-016's. An acceptance check with `method command` is
//! satisfied by its command's exit status, and `cargo test -p x a_name_that_is_gone`
//! prints "running 0 tests ... ok. 0 passed; 373 filtered out" and **exits 0**. A gate
//! that executes nothing is indistinguishable from one that passed, which defeats the
//! ledger's primary guarantee rather than its freshness machinery.
//!
//! Warning, never an error, and deliberately. A 16-workspace census in 2026-09 found 26
//! distinct phantom leaves and **every one had prior code history** — each named a test
//! that genuinely existed and was later removed. So this is decay rather than
//! carelessness, and on an `active` record a test that does not exist yet is a
//! specification, not a false gate.

use crate::diagnostics::{Diagnostic, RuleId, Subject};
use crate::git::{GitError, Repository, codes};
use crate::model::{CheckMethod, Commit, ContentSlot, ContentValue, Ledger, RevisionId};
use std::collections::BTreeSet;

/// V-105 belongs to the freshness/git rule range (`docs/10-freshness-and-git.md` §9).
const V105: RuleId = RuleId(105);

/// Identifier-shaped tokens in a command, keeping only those that could name a test.
///
/// A token is `[a-z0-9_]+` optionally joined by `::`. Only tokens carrying a `_` or a
/// `::` are kept, which is what separates `a_focused_test` and `module::tests` from
/// `cargo`, `test` and `workspace`. A command with no such token — `just check`,
/// `cargo check --workspace` — yields nothing and is never reported, so a gate that runs
/// a whole suite stays silent instead of becoming noise on every ledger.
fn command_tokens(command: &str) -> Vec<String> {
    let chars: Vec<char> = command.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if is_token_char(chars[i]) {
            let start = i;
            while i < chars.len() {
                if is_token_char(chars[i]) {
                    i += 1;
                } else if chars[i] == ':' && chars.get(i + 1) == Some(&':') {
                    i += 2;
                } else {
                    break;
                }
            }
            let token: String = chars[start..i].iter().collect();
            let token = token.trim_end_matches(':');
            if token.contains('_') || token.contains("::") {
                out.push(token.to_owned());
            }
        } else {
            i += 1;
        }
    }
    out.sort();
    out.dedup();
    out
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'
}

/// The pieces of a token that must be found in source for it to count as present.
///
/// `cargo test` filters are SUBSTRING matches against the full test path, so a
/// module-path filter like `keybindings::tests` never appears literally anywhere — source
/// has `mod keybindings;` in one file and `mod tests` in another, and the filter runs
/// perfectly. Looking the whole token up literally is precisely the false-positive class
/// this check exists to avoid: it accounted for two thirds of the first count in the
/// 2026-09 sweep, taking a true 4.2% to a reported 11.5% and turning Kitchen-Concept's
/// zero real phantoms into eleven apparent ones. So a `::` token resolves segment by
/// segment.
fn token_parts(token: &str) -> Vec<&str> {
    if token.contains("::") {
        token.split("::").filter(|part| !part.is_empty()).collect()
    } else {
        vec![token]
    }
}

/// Why a recorded command's exit status cannot be trusted, or `None` when it can.
///
/// A25 and A27. Both are the same defect as A8 seen from a different side: A8 is a gate
/// that runs nothing, these are gates whose *result* means nothing.
///
/// A shell pipeline reports the LAST stage's status, so `cargo test ... | tail -80` exits
/// 0 whenever `tail` succeeds no matter what cargo did. That is not theoretical — it was
/// demonstrated three times in one afternoon by three different agents during the 2026-09
/// sweep, once nearly sealing a fabricated pass into an evidence record. `|| true` and a
/// trailing `; true` are the same trick spelled differently.
///
/// The HTML-escaped operators come from records authored through a web or MCP surface
/// that escaped the text: pasted verbatim, `&amp;&amp;` is not `&&` and the command fails
/// rather than lying, but either way the slot does not hold what it claims to.
fn untrustworthy_status(command: &str) -> Option<&'static str> {
    if command.contains("&amp;&amp;") || command.contains("&amp;") {
        return Some(
            "it contains an HTML-escaped `&amp;`, so it is not the command it appears to be",
        );
    }
    // `||` is an alternation, not a pipe; check it first so it is not miscounted.
    let piped = command.replace("||", "").contains('|');
    if piped && !command.contains("pipefail") && !command.contains("PIPESTATUS") {
        return Some(
            "a shell pipeline reports only its LAST stage's status, so this exits 0 \
             whenever the final command succeeds regardless of what the real one did",
        );
    }
    if command.contains("|| true") || command.trim_end().ends_with("; true") {
        return Some("it forces success, so its exit status cannot fail");
    }
    None
}

/// One recorded command, and enough to point a reader at it.
struct Recorded {
    id: RevisionId,
    where_: String,
    command: String,
}

fn recorded_commands(ledger: &Ledger) -> Vec<Recorded> {
    let mut out = Vec::new();
    let mut records: Vec<_> = ledger.records().iter().collect();
    records.sort_by(|a, b| a.id.cmp(&b.id));
    for record in records {
        // A superseded or otherwise non-live revision is frozen: nobody can revise it to
        // repoint a renamed test, and its successor (if any) already carries the fix.
        // Checking it anyway would make a corrected papercut warn forever.
        if !record.is_live() {
            continue;
        }
        if let Some(acceptance) = &record.acceptance {
            for check in &acceptance.checks {
                if check.method == CheckMethod::Command
                    && let Some(command) = &check.command
                {
                    out.push(Recorded {
                        id: record.id.clone(),
                        where_: format!("check `{}`", check.id.as_str()),
                        command: command.clone(),
                    });
                }
            }
        }
        // Evidence `command` slots are the bigger half and nobody was watching them: the
        // same census found 1547 of them, 14 phantom, and 12 of those carrying
        // `result pass` — records that have actively misled a reader.
        if let Some(ContentValue::Text(command)) = record.content.get(&ContentSlot::Command) {
            out.push(Recorded {
                id: record.id.clone(),
                where_: "command".to_owned(),
                command: command.clone(),
            });
        }
    }
    out
}

/// Every recorded command whose test-name tokens name nothing in tracked source.
///
/// History enrichment is deliberately absent. `git log -S` per token would let the
/// diagnostic name the commit that removed the test, but it costs a process per token,
/// and the census already established the general truth for the help text. It belongs
/// behind an opt-in flag, not in the default path.
///
/// # Errors
/// [`GitError::CommandFailed`] if git cannot search the tree.
pub fn phantom_command_tokens(
    ledger: &Ledger,
    repository: &Repository,
    head: &Commit,
) -> Result<Vec<Diagnostic>, GitError> {
    let recorded = recorded_commands(ledger);
    let tokenised: Vec<Vec<String>> = recorded
        .iter()
        .map(|entry| command_tokens(&entry.command))
        .collect();

    // One `git grep` for every part of every token at once, for the same reason
    // `Repository::prime_ignored` batches: the per-needle form costs a process each.
    let mut parts: BTreeSet<&str> = BTreeSet::new();
    for tokens in &tokenised {
        for token in tokens {
            parts.extend(token_parts(token));
        }
    }
    let present = repository.tracked_source_hits(head, parts.iter().copied())?;

    let mut out = Vec::new();
    for (entry, tokens) in recorded.iter().zip(tokenised.iter()) {
        // A25/A27 first: a command whose status is meaningless is worth saying even when
        // its test names are all real, because the gate is unfalsifiable either way.
        if let Some(why) = untrustworthy_status(&entry.command) {
            out.push(
                Diagnostic::warning(
                    codes::G025,
                    V105,
                    Subject::Revision(entry.id.clone()),
                    format!(
                        "{}/{}: {} runs `{}`, and {why}",
                        entry.id.key, entry.id.revision, entry.where_, entry.command
                    ),
                )
                .help(
                    "a check is satisfied by its command's exit status, so record a \
                     command whose status means something",
                ),
            );
        }
        if tokens.is_empty() {
            continue;
        }
        // "None of them resolves", not "any of them fails": one resolving token means the
        // command can still execute something, so the gate is not empty.
        if tokens
            .iter()
            .any(|token| token_parts(token).iter().all(|part| present.contains(*part)))
        {
            continue;
        }
        let named = tokens.join(", ");
        out.push(
            Diagnostic::warning(
                codes::G024,
                V105,
                Subject::Revision(entry.id.clone()),
                format!(
                    "{}/{}: {} runs `{}`, and {} names nothing in tracked source",
                    entry.id.key,
                    entry.id.revision,
                    entry.where_,
                    entry.command,
                    if tokens.len() == 1 {
                        named
                    } else {
                        format!("none of {named}")
                    }
                ),
            )
            .help(
                "the test this names was most likely removed or renamed; a filter that \
                 matches nothing still exits 0, so this gate passes without running \
                 anything",
            ),
        );
    }
    Ok(out)
}
