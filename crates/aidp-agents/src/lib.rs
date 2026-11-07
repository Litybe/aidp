//! Provider-neutral agent runtime contract: trait, events, config and resolution.

pub mod config;
pub mod handle;
pub mod models;
pub mod provider;
pub mod resolve;

pub use config::{ProvidersConfig, PROVIDERS_KEY, PROVIDER_VERSION_ARG};
pub use handle::{AgentControl, AgentHandle};
pub use models::agent_spec::AgentSpec;
pub use models::capabilities::Capabilities;
pub use models::enums::agent_error::AgentError;
pub use models::enums::agent_event::AgentEvent;
pub use models::enums::exit_reason::ExitReason;
pub use models::enums::mcp_transport::McpTransport;
pub use models::enums::permission_policy::PermissionPolicy;
pub use models::enums::provider_origin::ProviderOrigin;
pub use models::mcp_configuration::McpConfiguration;
pub use models::mcp_server_config::McpServerConfig;
pub use models::provider_entry::ProviderEntry;
pub use models::resolve_request::ResolveRequest;
pub use models::token_usage::TokenUsage;
pub use provider::AgentProvider;
pub use resolve::{resolve_provider, Resolved};
