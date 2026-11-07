/// Where duplicate step ids are looked for.
#[derive(Debug, Clone, Copy)]
pub(crate) enum DuplicateScope<'a> {
    Flow,
    Block(&'a str),
}

impl DuplicateScope<'_> {
    /// The scope name quoted in the issue message.
    pub(crate) fn as_str(&self) -> &str {
        match self {
            DuplicateScope::Flow => "flow",
            DuplicateScope::Block(name) => name,
        }
    }
}
