/// A file of a pull request at one commit: its path and blob sha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileBlob {
    pub path: String,
    pub sha: String,
}
