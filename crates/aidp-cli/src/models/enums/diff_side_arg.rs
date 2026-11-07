use aidp_integrations::models::enums::diff_side::DiffSide;

/// Side of a diff accepted by `--side` and `--start-side`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum DiffSideArg {
    /// The removed lines.
    #[value(name = "LEFT")]
    Left,
    /// The added and unchanged lines.
    #[value(name = "RIGHT")]
    Right,
}

impl From<DiffSideArg> for DiffSide {
    fn from(arg: DiffSideArg) -> Self {
        match arg {
            DiffSideArg::Left => DiffSide::Left,
            DiffSideArg::Right => DiffSide::Right,
        }
    }
}
