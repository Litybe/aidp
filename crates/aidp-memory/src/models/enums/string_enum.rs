/// Declares an enum serialized as a lowercase string with `as_str`, `parse` and `ALL`.
macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident, $label:literal, { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        pub enum $name {
            $(
                #[serde(rename = $text)]
                $variant,
            )+
        }

        impl $name {
            /// Every variant, in declaration order.
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            /// The stored and displayed spelling.
            pub fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $text,)+
                }
            }

            /// Parses the stored spelling, ignoring case.
            pub fn parse(raw: &str) -> crate::error::Result<Self> {
                let lowered = raw.trim().to_lowercase();
                match lowered.as_str() {
                    $($text => Ok($name::$variant),)+
                    _ => Err(crate::error::MemoryError::Invalid(format!(
                        "unknown {} `{}`; expected one of: {}",
                        $label,
                        raw,
                        [$($text),+].join(", ")
                    ))),
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}
