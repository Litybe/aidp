use std::collections::BTreeMap;

use aidp_agents::{AgentError, McpConfiguration, McpTransport};

use crate::models::constants::mcp::*;
use crate::models::mcp_provider_server::McpProviderServer;

pub(crate) fn claude_configuration(
    configuration: &McpConfiguration,
) -> Result<(String, BTreeMap<String, String>), AgentError> {
    let mut servers = servers(configuration)?;
    let mut environment = BTreeMap::new();
    let mut index = 0usize;
    let mut substitute = |value: &mut String| {
        let name = format!("{CLAUDE_VALUE_PREFIX}{index}");
        index += 1;
        environment.insert(name.clone(), std::mem::take(value));
        *value = format!("${{{name}}}");
    };
    for server in servers.values_mut() {
        for value in server
            .url
            .iter_mut()
            .chain(server.command.iter_mut())
            .chain(server.args.iter_mut())
            .chain(server.headers.values_mut())
            .chain(server.env.values_mut())
        {
            substitute(value);
        }
    }
    let json = serde_json::to_string(
        &crate::models::claude_mcp_configuration::ClaudeMcpConfiguration { servers },
    )
    .map_err(|_| AgentError::Config(CONFIGURATION_ERROR.to_string()))?;
    Ok((json, environment))
}

pub(crate) fn redact(configuration: Option<&McpConfiguration>, text: &str) -> String {
    let Some(configuration) = configuration else {
        return text.to_string();
    };
    let mut values: Vec<String> = configuration.environment.values().cloned().collect();
    if let Ok(servers) = servers(configuration) {
        for server in servers.into_values() {
            if let Some(url) = server.url {
                if let Some((_, query)) = url.split_once('?') {
                    let query = query.split('#').next().unwrap_or_default();
                    for parameter in query.split('&') {
                        if let Some((_, value)) = parameter.split_once('=') {
                            values.push(value.to_string());
                            values.push(decode_query(value));
                        }
                    }
                }
                values.push(url);
            }
            values.extend(server.command);
            values.extend(server.args);
            for header in server.headers.into_values() {
                if let Some((_, credential)) = header.split_once(' ') {
                    if !credential.trim().is_empty() {
                        values.push(credential.trim().to_string());
                    }
                }
                values.push(header);
            }
            values.extend(server.env.into_values());
        }
    }
    values.sort_by_key(|value| std::cmp::Reverse(value.len()));
    let mut redacted = text.to_string();
    for value in values.into_iter().filter(|value| !value.is_empty()) {
        redacted = redacted.replace(&value, REDACTED_VALUE);
    }
    redacted
}

pub(crate) fn redact_value(
    configuration: Option<&McpConfiguration>,
    value: &mut serde_json::Value,
) {
    match value {
        serde_json::Value::String(text) => *text = redact(configuration, text),
        serde_json::Value::Array(values) => values
            .iter_mut()
            .for_each(|value| redact_value(configuration, value)),
        serde_json::Value::Object(values) => values
            .values_mut()
            .for_each(|value| redact_value(configuration, value)),
        _ => {}
    }
}

pub(crate) fn redact_event(
    configuration: Option<&McpConfiguration>,
    mut event: aidp_agents::AgentEvent,
) -> aidp_agents::AgentEvent {
    use aidp_agents::AgentEvent;
    match &mut event {
        AgentEvent::Text { text } => *text = redact(configuration, text),
        AgentEvent::ToolUse { input, .. } | AgentEvent::PermissionRequest { input, .. } => {
            redact_value(configuration, input)
        }
        AgentEvent::ToolResult { output, .. } => redact_value(configuration, output),
        AgentEvent::Raw(value) => redact_protocol_value(configuration, value),
        _ => {}
    }
    event
}

fn decode_query(value: &str) -> String {
    let mut bytes = Vec::new();
    let mut index = 0;
    let source = value.as_bytes();
    while index < source.len() {
        if source[index] == b'%' && index + 2 < source.len() {
            if let Ok(hex) = std::str::from_utf8(&source[index + 1..index + 3]) {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    bytes.push(byte);
                    index += 3;
                    continue;
                }
            }
        }
        bytes.push(if source[index] == b'+' {
            b' '
        } else {
            source[index]
        });
        index += 1;
    }
    String::from_utf8(bytes).unwrap_or_else(|_| value.to_string())
}

fn redact_protocol_value(configuration: Option<&McpConfiguration>, value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(values) => {
            for (key, value) in values {
                if CONTROL_FIELDS.contains(&key.as_str()) {
                    continue;
                }
                if PAYLOAD_FIELDS.contains(&key.as_str()) {
                    redact_value(configuration, value);
                } else {
                    redact_protocol_value(configuration, value);
                }
            }
        }
        serde_json::Value::Array(values) => values
            .iter_mut()
            .for_each(|value| redact_protocol_value(configuration, value)),
        _ => redact_value(configuration, value),
    }
}

pub(crate) fn drain_stderr(
    child: &mut dyn aidp_core::process_runner::SpawnedChild,
    confidential: bool,
) {
    if let Some(mut stderr) = child.take_stderr() {
        std::thread::spawn(move || {
            if confidential {
                let _ = std::io::copy(&mut stderr, &mut std::io::sink());
            } else {
                use std::io::{BufRead, BufReader};
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if !line.trim().is_empty() {
                        tracing::info!(line = %line, "{PROVIDER_STDERR}");
                    }
                }
            }
        });
    }
}

pub(crate) fn servers(
    configuration: &McpConfiguration,
) -> Result<BTreeMap<String, McpProviderServer>, AgentError> {
    configuration.validate()?;
    configuration
        .servers
        .iter()
        .map(|server| {
            Ok((
                server.name.clone(),
                McpProviderServer {
                    transport: (server.transport == McpTransport::Http).then_some(HTTP),
                    url: server
                        .url
                        .as_deref()
                        .map(|value| configuration.expand(value))
                        .transpose()?,
                    command: server
                        .command
                        .as_deref()
                        .map(|value| configuration.expand(value))
                        .transpose()?,
                    args: server
                        .args
                        .iter()
                        .map(|value| configuration.expand(value))
                        .collect::<Result<_, _>>()?,
                    headers: expand_map(configuration, &server.headers)?,
                    env: expand_map(configuration, &server.env)?,
                },
            ))
        })
        .collect()
}

fn expand_map(
    configuration: &McpConfiguration,
    values: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, AgentError> {
    values
        .iter()
        .map(|(name, value)| Ok((name.clone(), configuration.expand(value)?)))
        .collect()
}

#[cfg(test)]
#[path = "../tests/unit/mcp_provider_tests.rs"]
mod tests;
