/// How a settings leaf is validated.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FieldKind {
    Bool,
    /// A number above 0 and at most 1.
    Fraction,
    /// A number between 0 and 1 included.
    UnitInterval,
    /// A number of 0 or more.
    NonNegative,
    /// A whole number between the two bounds included.
    Count(u64, u64),
    /// A token count: a number, or text such as `3M`.
    Tokens,
    /// One of the listed words.
    Choice(&'static [&'static str]),
    /// A local `HH:MM` time.
    ClockTime,
    /// A percentage above 0 and at most 100; `null` removes the key.
    OptionalPercent,
}
