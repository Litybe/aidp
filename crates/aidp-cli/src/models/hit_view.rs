use aidp_memory::Hit;
use serde::Serialize;

/// A search hit as printed by `aidp rag search --format json`.
#[derive(Serialize)]
pub struct HitView {
    pub id: String,
    pub source: String,
    pub path: String,
    pub lines: String,
    pub citation: String,
    pub title: String,
    pub score: f64,
    pub via: String,
    pub text: String,
}

impl From<&Hit> for HitView {
    fn from(hit: &Hit) -> Self {
        Self {
            id: hit.id.clone(),
            source: hit.source.as_str().to_string(),
            path: hit.path.clone(),
            lines: format!("L{}-L{}", hit.line_start, hit.line_end),
            citation: hit.citation(),
            title: hit.title.clone(),
            score: (hit.score * 1000.0).round() / 1000.0,
            via: hit.via.clone(),
            text: hit.text.clone(),
        }
    }
}
