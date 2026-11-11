string_enum!(
    /// The kind of work a memory or a context block is for.
    Route, "route", {
        Code => "code",
        Art => "art",
        Map => "map",
        Doc => "doc",
        Tooling => "tooling",
    }
);

impl Route {
    /// Sources worth searching for this route.
    pub fn sources(self) -> &'static [crate::models::enums::Source] {
        use crate::models::enums::Source;
        match self {
            Route::Code => &[Source::Code, Source::Doc, Source::Knowledge, Source::Bible],
            Route::Art => &[Source::Bible, Source::Doc, Source::Knowledge],
            Route::Map => &[Source::Bible, Source::Doc, Source::Knowledge, Source::Code],
            Route::Doc => &[Source::Bible, Source::Doc, Source::Knowledge],
            Route::Tooling => &[Source::Knowledge, Source::Doc, Source::Run],
        }
    }

    /// Domains an agent without a specific role reads for this route.
    pub fn domains(self) -> &'static [crate::models::enums::Domain] {
        use crate::models::enums::Domain;
        match self {
            Route::Code => &[
                Domain::Unity,
                Domain::Csharp,
                Domain::Mechanics,
                Domain::GameDesign,
            ],
            Route::Art => &[
                Domain::PixelArt,
                Domain::PixelAnimation,
                Domain::Tooling,
                Domain::GameDesign,
            ],
            Route::Map => &[
                Domain::LevelDesign,
                Domain::Rooms,
                Domain::Mechanics,
                Domain::Tooling,
                Domain::GameDesign,
            ],
            Route::Doc => &[
                Domain::GameDesign,
                Domain::Lore,
                Domain::Mechanics,
                Domain::Rooms,
            ],
            Route::Tooling => &[Domain::Tooling],
        }
    }
}
