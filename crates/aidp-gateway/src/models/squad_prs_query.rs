use aidp_integrations::models::enums::bots_filter::BotsFilter;
use aidp_integrations::models::enums::lane::Lane;

const KEY_LANE: &str = "lane";
const KEY_REPO: &str = "repo";
const KEY_AUTHOR: &str = "author";
const KEY_BOTS: &str = "bots";
const KEY_TEXT: &str = "q";

/// Filters of `GET /api/prs`, applied to the cached rows of each request.
#[derive(Debug, Clone, Default)]
pub struct SquadPrsQuery {
    pub lane: Option<Lane>,
    pub repo: Option<String>,
    pub author: Option<String>,
    pub bots: BotsFilter,
    pub text: Option<String>,
}

fn parse_word<T: serde::de::DeserializeOwned>(key: &str, value: &str) -> Result<T, String> {
    serde_json::from_value(serde_json::Value::String(value.to_string()))
        .map_err(|_| format!("invalid {key}: {value}"))
}

impl SquadPrsQuery {
    /// Reads the filters through `get`; an unknown lane or bots value is an error message.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let lane = get(KEY_LANE)
            .map(|value| parse_word::<Lane>(KEY_LANE, &value))
            .transpose()?;
        let bots = get(KEY_BOTS)
            .map(|value| parse_word::<BotsFilter>(KEY_BOTS, &value))
            .transpose()?
            .unwrap_or_default();
        Ok(SquadPrsQuery {
            lane,
            repo: get(KEY_REPO),
            author: get(KEY_AUTHOR),
            bots,
            text: get(KEY_TEXT),
        })
    }
}
