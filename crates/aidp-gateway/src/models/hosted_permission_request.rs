use serde_json::Value;

#[derive(Clone)]
pub(crate) struct HostedPermissionRequest {
    pub(crate) id: String,
    pub(crate) tool: String,
    pub(crate) input: Value,
}
