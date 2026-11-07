use std::path::PathBuf;

/// Parsed arguments of `aidp run exec`.
pub struct ExecRequest {
    pub template: String,
    pub target: Option<String>,
    pub input: Vec<(String, String)>,
    pub inputs_file: Option<PathBuf>,
    pub provider: Option<String>,
    pub approve_gate: Vec<String>,
    pub foreground: bool,
    pub url: Option<String>,
}
