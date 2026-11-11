//! The embedder every process but the daemon uses: it asks the running daemon, which owns the one model.

use crate::embedder::Embedder;
use crate::error::{MemoryError, Result};
use crate::models::constants::remote::{
    BEARER_PREFIX, CONNECT_TIMEOUT_MS, DEFAULT_GATEWAY_URL, EMBED_MAX_TEXTS, EMBED_PATH,
    EMBED_READ_TIMEOUT_SECS, ENV_GATEWAY_URL, ENV_UI_TOKEN, MODEL_PATH, PROBE_TIMEOUT_MS,
};
use crate::models::embed_request::EmbedRequest;
use crate::models::embed_response::EmbedResponse;
use crate::models::enums::model_state::ModelState;
use crate::models::model_status_body::ModelStatusBody;
use std::time::Duration;

const AUTHORIZATION: &str = "Authorization";

/// Base URL of the daemon: `AIDP_GATEWAY_URL` or the local default.
pub fn gateway_url() -> String {
    std::env::var(ENV_GATEWAY_URL)
        .ok()
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| DEFAULT_GATEWAY_URL.to_string())
        .trim_end_matches('/')
        .to_string()
}

/// Embeds through `POST /api/embed` of the daemon; created only when the model is running there.
pub struct RemoteEmbedder {
    agent: ureq::Agent,
    base_url: String,
    token: Option<String>,
    model_id: String,
    dim: usize,
    embed_timeout: Duration,
}

impl RemoteEmbedder {
    /// Probes the daemon; fails when it is unreachable or its model is not running.
    pub fn connect(base_url: &str, embed_timeout: Duration) -> Result<Self> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_millis(CONNECT_TIMEOUT_MS))
            .build();
        let token = std::env::var(ENV_UI_TOKEN)
            .ok()
            .map(|token| token.trim().to_string())
            .filter(|token| !token.is_empty());
        let base_url = base_url.trim_end_matches('/').to_string();
        let status = probe(&agent, &base_url, token.as_deref())?;
        if status.state != ModelState::Running {
            return Err(MemoryError::Embedding(format!(
                "the embedding model of the daemon is {}",
                status.state.as_str()
            )));
        }
        let dim = status.dim.ok_or_else(|| {
            MemoryError::Embedding("the daemon did not report the vector size".to_string())
        })?;
        Ok(Self {
            agent,
            base_url,
            token,
            model_id: status.model,
            dim,
            embed_timeout,
        })
    }

    /// [`RemoteEmbedder::connect`] on the daemon named by the environment, with the batch read timeout.
    pub fn connect_default() -> Result<Self> {
        Self::connect(&gateway_url(), Duration::from_secs(EMBED_READ_TIMEOUT_SECS))
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let body = EmbedRequest {
            texts: texts.iter().map(|text| (*text).to_string()).collect(),
        };
        let mut request = self
            .agent
            .post(&format!("{}{EMBED_PATH}", self.base_url))
            .timeout(self.embed_timeout);
        if let Some(token) = &self.token {
            request = request.set(AUTHORIZATION, &format!("{BEARER_PREFIX}{token}"));
        }
        let response: EmbedResponse = request
            .send_json(&body)
            .map_err(|error| MemoryError::Embedding(format!("daemon embed failed: {error}")))?
            .into_json()
            .map_err(|error| MemoryError::Embedding(format!("daemon embed reply: {error}")))?;
        if response.model != self.model_id || response.dim != self.dim {
            return Err(MemoryError::Embedding(format!(
                "the daemon now serves {} ({} values), expected {} ({})",
                response.model, response.dim, self.model_id, self.dim
            )));
        }
        Ok(response.vectors)
    }
}

fn probe(agent: &ureq::Agent, base_url: &str, token: Option<&str>) -> Result<ModelStatusBody> {
    let mut request = agent
        .get(&format!("{base_url}{MODEL_PATH}"))
        .timeout(Duration::from_millis(PROBE_TIMEOUT_MS));
    if let Some(token) = token {
        request = request.set(AUTHORIZATION, &format!("{BEARER_PREFIX}{token}"));
    }
    request
        .call()
        .map_err(|error| MemoryError::Embedding(format!("daemon not reachable: {error}")))?
        .into_json()
        .map_err(|error| MemoryError::Embedding(format!("daemon model status: {error}")))
}

impl Embedder for RemoteEmbedder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let mut vectors = Vec::with_capacity(texts.len());
        for batch in texts.chunks(EMBED_MAX_TEXTS) {
            let embedded = self.embed_batch(batch)?;
            if embedded.len() != batch.len() {
                return Err(MemoryError::Embedding(format!(
                    "the daemon returned {} vectors for {} texts",
                    embedded.len(),
                    batch.len()
                )));
            }
            vectors.extend(embedded);
        }
        Ok(vectors)
    }
}

/// The embedder of the running daemon, or `None` after one warning: callers then search lexically.
pub fn remote_embedder() -> Option<Box<dyn Embedder>> {
    match RemoteEmbedder::connect_default() {
        Ok(embedder) => Some(Box::new(embedder)),
        Err(error) => {
            tracing::warn!(%error, "embedding model unavailable; search is lexical only (start it with `aidp memory model start`)");
            None
        }
    }
}

/// Like [`remote_embedder`] without a warning, with a short timeout: for hooks that must never wait.
pub fn quiet_remote_embedder(embed_timeout: Duration) -> Option<Box<dyn Embedder>> {
    RemoteEmbedder::connect(&gateway_url(), embed_timeout)
        .ok()
        .map(|embedder| Box::new(embedder) as Box<dyn Embedder>)
}
