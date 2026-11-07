use crate::models::static_file::StaticFile;

/// What the UI directory answers for one request.
#[derive(Debug, PartialEq, Eq)]
pub enum StaticLookup {
    /// Not a UI request: no UI directory, not a read, or an `/api` path.
    NotUi,
    /// The file to send, `index.html` for a client-side route.
    File(StaticFile),
    /// A missing file with a known extension, a refused path, or no `index.html`.
    NotFound,
}
