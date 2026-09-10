//! The canonical formatter of D-012.
//!
//! One input, one output. Where a choice exists — slot order, array layout, prose
//! indentation, record order — the formatter decides, not the author. That is what makes
//! a reordered record produce no diff and a changed record produce a small one.

use super::cst::{Block, BodyItem, Comment, File, Item, Slot, Value};
use crate::model::{ContentSlot, Kind, Relation};

/// Four spaces per level (D-012).
const INDENT: usize = 4;
/// An array is emitted inline if the whole slot fits within this width (`docs/03` §6.4).
const WIDTH: usize = 96;

/// Formats a parsed file into canonical source.
///
/// The output always ends with exactly one newline.
#[must_use]
pub fn format(file: &File) -> String {
    let mut out = String::new();
    emit_comments(&mut out, &file.leading, 0, false);
    if file.blank_before_header && !file.leading.is_empty() {
        out.push('\n');
    }
    out.push_str(&format!("{} {}\n", file.keyword, file.version));
    out.push_str(&format!("project {}\n", file.project));

    let mut items: Vec<&Item> = file
        .items
        .iter()
        .filter(|i| i.sort_key().is_none())
        .collect();
    let mut records: Vec<&Item> = file
        .items
        .iter()
        .filter(|i| i.sort_key().is_some())
        .collect();
    records.sort_by_key(|i| i.sort_key().expect("filtered to records"));
    items.extend(records);

    let mut previous_was_namespace = false;
    for item in items {
        let is_namespace = matches!(item, Item::Namespace(_));
        // One blank line between top-level items, except between consecutive one-line
        // `namespace` declarations, which group.
        if !(is_namespace && previous_was_namespace) {
            out.push('\n');
        }
        emit_item(&mut out, item);
        previous_was_namespace = is_namespace;
    }
    if !file.trailing.is_empty() {
        out.push('\n');
        emit_comments(&mut out, &file.trailing, 0, false);
    }
    out
}

fn emit_item(out: &mut String, item: &Item) {
    // Not for a block: `Item::trivia()` returns the block's own trivia, and `emit_block`
    // emits it itself — it has to, because a nested block reaches it without passing
    // through here. Emitting in both places doubled a top-level block's leading comments
    // on every run (1, 2, 4, 8 ...), which is a formatter that is not idempotent.
    if !matches!(item, Item::Block(_)) {
        emit_comments(out, &item.trivia().leading, 0, false);
    }
    match item {
        Item::Namespace(n) => {
            out.push_str(&format!(
                "namespace {} \"{}\"",
                n.name,
                super::cst::escape(&n.description)
            ));
            emit_trailing(out, item.trivia().trailing.as_ref());
            out.push('\n');
        }
        Item::Record(record) => {
            out.push_str(&format!(
                "record {}/{} : {} {{\n",
                record.key, record.revision, record.kind
            ));
            let kind = Kind::from_name(&record.kind);
            emit_body(
                out,
                &record.body,
                &record.inner_trailing,
                kind,
                "record",
                INDENT,
            );
            out.push_str("}\n");
        }
        Item::Block(block) => emit_block(out, block, 0),
    }
}

fn emit_block(out: &mut String, block: &Block, indent: usize) {
    emit_comments(out, &block.trivia.leading, indent, false);
    let pad = " ".repeat(indent);
    match &block.head {
        Some(head) => {
            out.push_str(&format!(
                "{pad}{} {} {{\n",
                block.name,
                head.render_inline()
            ));
        }
        None => out.push_str(&format!("{pad}{} {{\n", block.name)),
    }
    emit_body(
        out,
        &block.body,
        &block.inner_trailing,
        None,
        &block.name,
        indent + INDENT,
    );
    out.push_str(&format!("{pad}}}\n"));
}

fn emit_body(
    out: &mut String,
    body: &[BodyItem],
    inner_trailing: &[Comment],
    kind: Option<Kind>,
    container: &str,
    indent: usize,
) {
    let mut ordered: Vec<(usize, &BodyItem)> = body.iter().enumerate().collect();
    ordered.sort_by(|(ai, a), (bi, b)| {
        order_key(a, kind, container, *ai).cmp(&order_key(b, kind, container, *bi))
    });
    for (_, item) in ordered {
        match item {
            BodyItem::Slot(slot) => emit_slot(out, slot, indent),
            BodyItem::Block(block) => emit_block(out, block, indent),
        }
    }
    emit_comments(out, inner_trailing, indent, false);
}

/// The canonical order of D-012, as a sort key.
///
/// `(group, position within group, tiebreak, source index)`. The source index keeps the
/// sort stable for items the vocabulary does not rank, so an unrecognised slot stays
/// where the author put it rather than moving unpredictably.
fn order_key(
    item: &BodyItem,
    kind: Option<Kind>,
    container: &str,
    source_index: usize,
) -> (u8, usize, String, usize) {
    let name = item.name();
    let head = item.head_text();

    // Slots inside a known block have their own fixed order.
    if let Some(position) = block_slot_order(container, name) {
        return (0, position, String::new(), source_index);
    }

    let group = match name {
        "title" => (0, 0),
        "state" => (1, 0),
        "scope" => (2, 0),
        "topic" => (3, 0),
        "claim" => (5, 0),
        "retired_claims" => (6, 0),
        "acceptance" => (7, 0),
        "disposition" => (8, 0),
        "acknowledged" => (10, 0),
        "author" => (11, 0),
        "created_at" => (12, 0),
        "source" => (13, 0),
        _ => {
            if let Some(index) =
                kind.and_then(|k| k.content_slots().iter().position(|s| s.slot.name() == name))
            {
                (4, index)
            } else if let Some(relation) = Relation::from_name(name) {
                let index = Relation::ALL
                    .iter()
                    .filter(|r| r.name() < relation.name())
                    .count();
                (9, index)
            } else if ContentSlot::from_name(name).is_some() {
                // A content slot that this kind does not define: V-008 reports it, and
                // the formatter keeps it where it was rather than guessing.
                (14, 0)
            } else {
                (14, 0)
            }
        }
    };
    // Claim, check, disposition and source blocks sort by their head or by content.
    let tiebreak = match name {
        "claim" | "check" | "disposition" => head,
        "source" => source_sort_key(item),
        _ => String::new(),
    };
    (group.0, group.1, tiebreak, source_index)
}

/// Sorts `source` blocks by kind, then by path or url (D-012 step 13).
fn source_sort_key(item: &BodyItem) -> String {
    let BodyItem::Block(block) = item else {
        return String::new();
    };
    let get = |name: &str| {
        block.body.iter().find_map(|i| match i {
            BodyItem::Slot(s) if s.name == name => Some(s.value.render_inline()),
            _ => None,
        })
    };
    format!(
        "{}\u{1}{}",
        get("kind").unwrap_or_default(),
        get("path").or_else(|| get("url")).unwrap_or_default()
    )
}

/// The fixed slot order inside a known block, or `None` for blocks the formatter does
/// not rank (`defaults`, and lock-file items, which are generated already ordered).
fn block_slot_order(container: &str, slot: &str) -> Option<usize> {
    let order: &[&str] = match container {
        "claim" => &["text", "supported_by"],
        "check" => &["statement", "method", "command", "verified_by"],
        "source" => &[
            "kind",
            "role",
            "document",
            "path",
            "url",
            "start_byte",
            "end_byte",
            "start_line",
            "end_line",
            "excerpt_hash",
            "excerpt",
            "use",
        ],
        "disposition" => &["outcome", "into", "note"],
        _ => return None,
    };
    order.iter().position(|s| *s == slot)
}

fn emit_slot(out: &mut String, slot: &Slot, indent: usize) {
    emit_comments(
        out,
        &slot.trivia.leading,
        indent,
        slot.trivia.blank_before(),
    );
    let pad = " ".repeat(indent);
    match &slot.value {
        Value::Prose(text, _) => {
            out.push_str(&format!("{pad}{} \"\"\"\n", slot.name));
            let inner = " ".repeat(indent + INDENT);
            for line in text.lines() {
                if line.is_empty() {
                    out.push('\n');
                } else {
                    out.push_str(&format!("{inner}{line}\n"));
                }
            }
            out.push_str(&format!("{inner}\"\"\""));
        }
        Value::Array(items, _) => {
            let sorted = sorted_array(items);
            if sorted.is_empty() {
                // An empty array means the same as omitting the slot (D-012); drop it.
                return;
            }
            let inline: Vec<String> = sorted.iter().map(|v| v.render_inline()).collect();
            let one_line = format!("{pad}{} [ {} ]", slot.name, inline.join(", "));
            if one_line.chars().count() <= WIDTH {
                out.push_str(&one_line);
            } else {
                out.push_str(&format!("{pad}{} [\n", slot.name));
                let inner = " ".repeat(indent + INDENT);
                for (i, rendered) in inline.iter().enumerate() {
                    let comma = if i + 1 == inline.len() { "" } else { "," };
                    out.push_str(&format!("{inner}{rendered}{comma}\n"));
                }
                out.push_str(&format!("{pad}]"));
            }
        }
        other => out.push_str(&format!("{pad}{} {}", slot.name, other.render_inline())),
    }
    emit_trailing(out, slot.trivia.trailing.as_ref());
    out.push('\n');
}

/// Sorts reference and scope arrays; leaves every other array alone.
///
/// `docs/03` §6.3 sorts references by key, revision and anchor, and scope terms by form.
/// String, glob and identifier arrays keep their authored order: they are lists a person
/// wrote in a deliberate order, and reordering them would churn diffs for no gain.
fn sorted_array(items: &[Value]) -> Vec<&Value> {
    let mut out: Vec<&Value> = items.iter().collect();
    let sortable = items
        .iter()
        .all(|v| matches!(v, Value::Ref(..)) || matches!(v, Value::Prefixed(..)))
        || items.iter().all(|v| {
            matches!(v, Value::Ref(..) | Value::Prefixed(..))
                || matches!(v, Value::Word(w, _) if w == "all")
        });
    if sortable && !items.is_empty() {
        out.sort_by_key(|v| v.sort_key());
    }
    out
}

fn emit_comments(out: &mut String, comments: &[Comment], indent: usize, blank_before: bool) {
    if comments.is_empty() {
        return;
    }
    if blank_before {
        out.push('\n');
    }
    let pad = " ".repeat(indent);
    for comment in comments {
        if comment.text.is_empty() {
            out.push_str(&format!("{pad}#\n"));
        } else {
            out.push_str(&format!("{pad}# {}\n", comment.text));
        }
    }
}

fn emit_trailing(out: &mut String, comment: Option<&Comment>) {
    if let Some(comment) = comment {
        out.push_str(&format!("  # {}", comment.text));
    }
}
