use aidp_integrations::models::enums::bots_filter::BotsFilter;

/// Bot handling accepted by `--bots`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum BotsArg {
    /// Leave bot-authored pull requests out.
    Hide,
    /// List bot-authored pull requests with the others.
    Show,
    /// List only bot-authored pull requests.
    Only,
}

impl From<BotsArg> for BotsFilter {
    fn from(arg: BotsArg) -> Self {
        match arg {
            BotsArg::Hide => BotsFilter::Hide,
            BotsArg::Show => BotsFilter::Show,
            BotsArg::Only => BotsFilter::Only,
        }
    }
}
