use aidp_core::Format;

/// Output format accepted by `--format`, mirrored from `aidp_core::Format`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum FormatArg {
    /// One compact JSON document on a single line.
    JsonCompact,
    /// One indented JSON document.
    #[value(alias = "json")]
    JsonPretty,
    /// One compact JSON value per line.
    Ndjson,
    /// Human-readable text.
    #[value(alias = "md")]
    Text,
    /// The one-line cost footer of a report, for `aidp cost ticket`; text elsewhere.
    Footer,
}

impl From<FormatArg> for Format {
    fn from(arg: FormatArg) -> Self {
        match arg {
            FormatArg::JsonCompact => Self::JsonCompact,
            FormatArg::JsonPretty => Self::JsonPretty,
            FormatArg::Ndjson => Self::Ndjson,
            FormatArg::Text | FormatArg::Footer => Self::Text,
        }
    }
}

impl From<Format> for FormatArg {
    fn from(format: Format) -> Self {
        match format {
            Format::JsonCompact => Self::JsonCompact,
            Format::JsonPretty => Self::JsonPretty,
            Format::Ndjson => Self::Ndjson,
            Format::Text => Self::Text,
        }
    }
}
