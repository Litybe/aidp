//! Budgeted packing of hits and their Markdown rendering with citations.

use crate::models::enums::Source;
use crate::models::hit::Hit;
use crate::models::packed::Packed;
use crate::text::{estimate_tokens, truncate_chars};

const FENCE: &str = "```";
const LONG_FENCE: &str = "````";
const CODE_LANGUAGE: &str = "cs";
const ELLIPSIS: &str = "\n...";
const HEADER_OVERHEAD_CHARS: usize = 32;
const CHARS_PER_TOKEN: usize = crate::models::constants::limits::CHARS_PER_TOKEN;

/// Renders one hit as a Markdown excerpt with its citation.
pub fn render_hit(hit: &Hit) -> String {
    let fence = if hit.text.contains(FENCE) {
        LONG_FENCE
    } else {
        FENCE
    };
    let language = if hit.source == Source::Code {
        CODE_LANGUAGE
    } else {
        ""
    };
    format!(
        "### {} - {}\n{fence}{language}\n{}\n{fence}\n",
        hit.citation(),
        hit.title,
        hit.text
    )
}

/// Keeps hits in order while their rendering fits `budget_tokens`.
///
/// A hit that does not fit is skipped so a smaller later one can still enter;
/// when nothing fits at all, the first hit is cut to the budget.
pub fn pack(hits: Vec<Hit>, budget_tokens: usize) -> Packed {
    let mut packed = Packed {
        budget_tokens,
        ..Packed::default()
    };
    for hit in hits {
        let cost = estimate_tokens(&render_hit(&hit));
        if packed.used_tokens + cost <= budget_tokens {
            packed.used_tokens += cost;
            packed.hits.push(hit);
        } else if packed.hits.is_empty() && budget_tokens > 0 {
            let room = (budget_tokens * CHARS_PER_TOKEN).saturating_sub(
                hit.citation().chars().count() + hit.title.chars().count() + HEADER_OVERHEAD_CHARS,
            );
            let mut cut = hit.clone();
            cut.text = format!("{}{ELLIPSIS}", truncate_chars(&hit.text, room));
            packed.used_tokens += estimate_tokens(&render_hit(&cut));
            packed.hits.push(cut);
        } else {
            packed.dropped += 1;
        }
    }
    packed
}

/// Markdown of every packed hit, in order.
pub fn render_markdown(packed: &Packed) -> String {
    packed
        .hits
        .iter()
        .map(render_hit)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
#[path = "../tests/unit/pack_tests.rs"]
mod tests;
