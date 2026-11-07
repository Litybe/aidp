use super::GoldenCase;

/// Collects the cases of one template and drops any identical to one already kept.
#[derive(Default)]
pub(crate) struct CaseCollector {
    pub(crate) cases: Vec<GoldenCase>,
}

impl CaseCollector {
    /// Keeps `rendered` unless a kept case has the same script and metadata.
    pub(crate) fn push(&mut self, rendered: GoldenCase) {
        let duplicate = self
            .cases
            .iter()
            .any(|kept| kept.script == rendered.script && kept.meta == rendered.meta);
        if !duplicate {
            self.cases.push(rendered);
        }
    }
}
