/// What a `pr.json` a pipeline stage wrote says about the pull request; every field is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrArtifact {
    pub number: Option<u64>,
    pub url: Option<String>,
    /// `owner/name`.
    pub repository: Option<String>,
    pub branch: Option<String>,
    pub commit_sha: Option<String>,
}
