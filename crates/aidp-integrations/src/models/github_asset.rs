/// An image GitHub hosts for a pull request or a comment, read on the viewer's behalf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubAsset {
    pub bytes: Vec<u8>,
    pub content_type: String,
}
