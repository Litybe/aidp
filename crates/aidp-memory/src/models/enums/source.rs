string_enum!(
    /// A family of indexed documents.
    Source, "source", {
        Bible => "bible",
        Doc => "doc",
        Code => "code",
        Run => "run",
        Ticket => "ticket",
        Knowledge => "knowledge",
        Memory => "memory",
    }
);

impl Source {
    /// Sources indexed from files, the default of `rag index`.
    pub const FILE_SOURCES: &'static [Source] = &[
        Source::Bible,
        Source::Doc,
        Source::Code,
        Source::Run,
        Source::Ticket,
        Source::Knowledge,
    ];

    /// Parses a comma-separated list such as `bible,doc`.
    pub fn parse_list(raw: &str) -> crate::error::Result<Vec<Source>> {
        raw.split(',')
            .filter(|part| !part.trim().is_empty())
            .map(Source::parse)
            .collect()
    }
}
