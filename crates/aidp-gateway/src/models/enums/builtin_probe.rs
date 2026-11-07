/// Wait probes every registry holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinProbe {
    Timer,
    PrReviews,
    PrThreads,
}

impl BuiltinProbe {
    /// Every probe, in registration order.
    pub const ALL: [BuiltinProbe; 3] = [
        BuiltinProbe::Timer,
        BuiltinProbe::PrReviews,
        BuiltinProbe::PrThreads,
    ];

    /// The registry name of the probe.
    pub const fn as_str(self) -> &'static str {
        match self {
            BuiltinProbe::Timer => "timer",
            BuiltinProbe::PrReviews => "pr.reviews",
            BuiltinProbe::PrThreads => "pr.threads",
        }
    }
}
