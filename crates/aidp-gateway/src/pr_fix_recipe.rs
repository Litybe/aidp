//! Renders the fix-pr-comments recipe from the selected threads and points.

use crate::models::constants::pr_fix::PR_FIX_SUMMARY_PREFIX;
use crate::models::constants::pr_fix_recipe::*;
use crate::models::pr_fix_recipe_input::PrFixRecipeInput;
use crate::models::pr_fix_thread_input::PrFixThreadInput;
use crate::models::selection_point::SelectionPoint;
use anyhow::Context;
use std::path::Path;

/// Reads the recipe template under the skill root.
pub fn load_recipe(skill_root: &Path) -> anyhow::Result<String> {
    let recipe_path = skill_root.join(RECIPE_RELATIVE_DIR).join(RECIPE_FILE_NAME);
    std::fs::read_to_string(&recipe_path)
        .with_context(|| format!("cannot read the recipe {}", recipe_path.display()))
}

/// Neutralizes code fences and placeholder openers in user-originated text.
fn escape(text: &str) -> String {
    text.replace(FENCE, FENCE_NEUTRALIZED)
        .replace(PLACEHOLDER_OPEN, OPEN_NEUTRALIZED)
}

fn fenced(body: &str) -> String {
    format!("{FENCE}\n{}\n{FENCE}", escape(body))
}

fn render_thread(thread: &PrFixThreadInput) -> String {
    let line = thread.line.map(|n| format!(":{n}")).unwrap_or_default();
    let mut block = format!(
        "{THREAD_HEADING} {}\n{FILE_LABEL} {}{line}\n\n",
        escape(&thread.id),
        escape(&thread.path)
    );
    block.push_str(&format!(
        "{DIFF_FENCE_OPEN}\n{}\n{FENCE}\n\n{CONVERSATION_LABEL}\n",
        escape(&thread.diff_hunk)
    ));
    for (author, body) in &thread.conversation {
        block.push_str(&format!(
            "\n{COMMENT_AUTHOR_LABEL} {}\n{}\n",
            escape(author),
            fenced(body)
        ));
    }
    block
}

fn render_point(point: &SelectionPoint) -> String {
    format!(
        "{POINT_HEADING} {}\n{}\n",
        escape(&point.id),
        fenced(&point.body)
    )
}

fn join_or_none(blocks: Vec<String>) -> String {
    if blocks.is_empty() {
        NO_ITEMS.to_string()
    } else {
        blocks.join("\n")
    }
}

fn render_note(note: Option<&str>) -> String {
    match note.filter(|text| !text.trim().is_empty()) {
        Some(text) => fenced(text),
        None => NO_NOTE.to_string(),
    }
}

fn placeholder_value(name: &str, input: &PrFixRecipeInput) -> Option<String> {
    match name {
        PLACEHOLDER_REPO => Some(escape(&input.repo)),
        PLACEHOLDER_PR_NUMBER => Some(input.pr_number.to_string()),
        PLACEHOLDER_HEAD_BRANCH => Some(escape(&input.head_branch)),
        PLACEHOLDER_THREADS => Some(join_or_none(
            input.threads.iter().map(render_thread).collect(),
        )),
        PLACEHOLDER_POINTS => Some(join_or_none(
            input.points.iter().map(render_point).collect(),
        )),
        PLACEHOLDER_NOTE => Some(render_note(input.note.as_deref())),
        PLACEHOLDER_SUMMARY_PREFIX => Some(PR_FIX_SUMMARY_PREFIX.to_string()),
        _ => None,
    }
}

/// Fills the template in a single pass, so substituted text is never re-scanned.
pub fn render(template: &str, input: &PrFixRecipeInput) -> String {
    let mut rendered = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open_at) = rest.find(PLACEHOLDER_OPEN) {
        rendered.push_str(&rest[..open_at]);
        let after_open = &rest[open_at + PLACEHOLDER_OPEN.len()..];
        let replacement = after_open.find(PLACEHOLDER_CLOSE).and_then(|close_at| {
            placeholder_value(&after_open[..close_at], input)
                .map(|value| (value, close_at + PLACEHOLDER_CLOSE.len()))
        });
        match replacement {
            Some((value, consumed)) => {
                rendered.push_str(&value);
                rest = &after_open[consumed..];
            }
            None => {
                rendered.push_str(PLACEHOLDER_OPEN);
                rest = after_open;
            }
        }
    }
    rendered.push_str(rest);
    rendered
}

/// Returns the JSON object of the last summary line, or `None` when absent or malformed.
pub fn parse_summary(transcript_tail: &str) -> Option<serde_json::Value> {
    let payload = transcript_tail
        .lines()
        .rev()
        .find_map(|line| line.trim().strip_prefix(PR_FIX_SUMMARY_PREFIX.trim_end()))?;
    serde_json::from_str::<serde_json::Value>(payload.trim())
        .ok()
        .filter(serde_json::Value::is_object)
}

#[cfg(test)]
#[path = "../tests/unit/pr_fix_recipe_tests.rs"]
mod tests;
