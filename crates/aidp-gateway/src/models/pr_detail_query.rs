const KEY_REPO: &str = "repo";
const KEY_NUMBER: &str = "number";
const KEY_PATH: &str = "path";
const KEY_PAGE: &str = "page";
const MISSING_PARAMS_MESSAGE: &str = "repo and a numeric number are required";
const BAD_PAGE_MESSAGE: &str = "page must be a positive number";

/// Query string of the PR page reads and of the comment deletes.
#[derive(Debug, Clone)]
pub struct PrDetailQuery {
    pub repo: String,
    pub number: u64,
    pub path: Option<String>,
    pub page: Option<u32>,
}

impl PrDetailQuery {
    /// Reads the parameters through `get`; a missing repo, number or a bad page is a message.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let number = get(KEY_NUMBER).and_then(|raw| raw.parse::<u64>().ok());
        let (Some(repo), Some(number)) = (get(KEY_REPO), number) else {
            return Err(MISSING_PARAMS_MESSAGE.to_string());
        };
        let page = get(KEY_PAGE)
            .map(|raw| raw.parse::<u32>().map_err(|_| BAD_PAGE_MESSAGE.to_string()))
            .transpose()?;
        Ok(PrDetailQuery {
            repo,
            number,
            path: get(KEY_PATH),
            page,
        })
    }
}
