//! Message fragments of the unparseable-expression warning.

/// Text before the stage name.
pub const UNPARSED_PREFIX: &str = "warning: stage `";

/// Text between the stage name and the expression kind.
pub const UNPARSED_MID: &str = "` has an unparseable `";

/// Text between the expression kind and the shown expression.
pub const UNPARSED_KIND_SUFFIX: &str = ":` expression (";

/// Text after the shown expression.
pub const UNPARSED_SUFFIX: &str = "); defaulting (when→run, gate→ask)";
