use aidp_agents::{
    AgentError, AgentHandle, AgentProvider, AgentSpec, Capabilities, McpConfiguration,
};
use std::sync::{Arc, Mutex};

pub struct McpRecordingProvider {
    inner: Arc<dyn AgentProvider>,
    configurations: Mutex<Vec<Option<Arc<McpConfiguration>>>>,
}

impl McpRecordingProvider {
    pub fn new(inner: Arc<dyn AgentProvider>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            configurations: Mutex::new(Vec::new()),
        })
    }

    pub fn configurations(&self) -> Vec<Option<Arc<McpConfiguration>>> {
        self.configurations.lock().unwrap().clone()
    }
}

impl AgentProvider for McpRecordingProvider {
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    fn start(&self, spec: AgentSpec) -> Result<AgentHandle, AgentError> {
        self.configurations
            .lock()
            .unwrap()
            .push(spec.mcp_configuration.clone());
        self.inner.start(spec)
    }
}
