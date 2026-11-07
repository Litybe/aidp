pub const REF_STAGES: &str = "stages";
pub const REF_OUTPUTS: &str = "outputs";
pub const REF_SIGNALS: &str = "signals";
/// Prefix that marks a `when:`/`gate:` value as reading a stage output.
pub const RUNTIME_REF_MARKER: &str = "stages.";
pub const HANDLEBARS_OPEN: &str = "{{";
pub const HANDLEBARS_CLOSE: &str = "}}";
pub const REF_SEPARATOR: char = '.';
/// Parts of `stages.<stage>.<segment>.<name>`.
pub const STAGE_REF_PARTS: usize = 4;
/// Parts of `stages.<stage>.outputs.<name>.<prop>`.
pub const LOOP_REF_MIN_PARTS: usize = 5;
