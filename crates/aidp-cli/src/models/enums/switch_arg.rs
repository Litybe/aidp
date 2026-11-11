/// An `on` or `off` argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum SwitchArg {
    On,
    Off,
}

impl SwitchArg {
    /// True for `on`.
    pub fn enabled(self) -> bool {
        self == SwitchArg::On
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/switch_arg_tests.rs"]
mod tests;
