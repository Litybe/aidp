/// A settings file section the daemon exposes under `/api/settings/<name>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    Routing,
    Budget,
    Models,
}

impl SettingsSection {
    /// The path segment and event name of the section.
    pub const fn as_str(self) -> &'static str {
        match self {
            SettingsSection::Routing => "routing",
            SettingsSection::Budget => "budget",
            SettingsSection::Models => "models",
        }
    }

    /// Parses a path segment.
    pub fn parse(segment: &str) -> Option<SettingsSection> {
        [
            SettingsSection::Routing,
            SettingsSection::Budget,
            SettingsSection::Models,
        ]
        .into_iter()
        .find(|section| section.as_str() == segment)
    }
}
