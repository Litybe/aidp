use crate::models::enums::fake_step::FakeStep;

const DEFAULT_EXIT_CODE: i32 = 0;

/// Ordered steps a fake agent plays, then its exit code.
#[derive(Debug, Clone)]
pub struct FakeScript {
    pub(crate) steps: Vec<FakeStep>,
    pub(crate) exit_code: i32,
}

impl Default for FakeScript {
    fn default() -> Self {
        Self {
            steps: Vec::new(),
            exit_code: DEFAULT_EXIT_CODE,
        }
    }
}
