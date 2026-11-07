//! Shared, bounded catalogue access for Rust hosts and the gateway API.

use crate::models::catalog_document::CatalogDocument;
use crate::models::catalog_saved::CatalogSaved;
use crate::models::catalog_template::CatalogTemplate;
use crate::models::catalog_validation::CatalogValidation;
use crate::models::catalog_write_request::CatalogWriteRequest;
use crate::models::constants::catalog::*;
use crate::models::native_error_body::NativeErrorBody;
pub use crate::models::pipeline_catalog::PipelineCatalog;
use crate::models::{ApiError, FlowReply};
use crate::session_error::SessionError;
use aidp_core::pipeline::{self, Pipeline, MAX_PIPELINE_BYTES};
use aidp_flows::FlowDef;
use serde::Serialize;
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tiny_http::Method;

static WRITE_GUARD: Mutex<()> = Mutex::new(());

impl PipelineCatalog {
    pub fn new(skill_root: PathBuf) -> Self {
        Self { root: skill_root }
    }

    fn directory(&self, flows: bool) -> PathBuf {
        let templates = self.root.join(TEMPLATES);
        if flows {
            templates.join(FLOWS)
        } else {
            templates
        }
    }

    fn file(&self, id: &str, flows: bool) -> anyhow::Result<PathBuf> {
        aidp_core::safe_name::check(if flows { FLOW_LABEL } else { TEMPLATE_LABEL }, id)?;
        let directory = self.directory(flows);
        for extension in [YAML_EXTENSION, YML_EXTENSION] {
            let path = directory.join(format!("{id}.{extension}"));
            if path.exists() {
                self.contained(&path)?;
                return Ok(path);
            }
        }
        Err(SessionError::NotFound(id.to_string()).into())
    }

    fn contained(&self, path: &Path) -> anyhow::Result<()> {
        let root = self.root.canonicalize()?;
        if !path.canonicalize()?.starts_with(root) {
            anyhow::bail!(OUTSIDE_ROOT);
        }
        Ok(())
    }

    fn ids(&self, flows: bool) -> anyhow::Result<Vec<String>> {
        let directory = self.directory(flows);
        if !directory.exists() {
            return Ok(Vec::new());
        }
        self.contained(&directory)?;
        let mut ids = Vec::new();
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if !path.is_file()
                || !matches!(
                    path.extension().and_then(|value| value.to_str()),
                    Some(YAML_EXTENSION | YML_EXTENSION)
                )
            {
                continue;
            }
            if let Some(id) = path.file_stem().and_then(|value| value.to_str()) {
                ids.push(id.to_string());
                anyhow::ensure!(ids.len() <= MAX_COLLECTION_FILES, COLLECTION_TOO_LARGE);
            }
        }
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    pub fn templates(&self) -> anyhow::Result<Vec<CatalogTemplate>> {
        collect_bounded(self.ids(false)?, MAX_COLLECTION_BYTES, |id| {
            self.template(id)
        })
    }

    pub fn template(&self, id: &str) -> anyhow::Result<CatalogDocument<CatalogTemplate>> {
        let raw = read_bounded(&self.file(id, false)?)?;
        let pipeline = parse_pipeline(&raw)?;
        anyhow::ensure!(pipeline.id == id, ID_MISMATCH);
        Ok(CatalogDocument {
            parsed: describe(pipeline),
            raw,
        })
    }

    pub fn flows(&self) -> anyhow::Result<Vec<FlowDef>> {
        collect_bounded(self.ids(true)?, MAX_COLLECTION_BYTES, |id| self.flow(id))
    }

    pub fn flow(&self, id: &str) -> anyhow::Result<CatalogDocument<FlowDef>> {
        let raw = read_bounded(&self.file(id, true)?)?;
        let parsed = FlowDef::from_yaml(&raw)?;
        anyhow::ensure!(parsed.id == id, ID_MISMATCH);
        Ok(CatalogDocument { parsed, raw })
    }

    /// Validates before writing; create refuses collisions, update requires an existing template.
    pub fn save_template(
        &self,
        id: &str,
        content: &str,
        create: bool,
    ) -> anyhow::Result<CatalogSaved> {
        aidp_core::safe_name::check(TEMPLATE_LABEL, id)?;
        let pipeline = parse_pipeline(content)?;
        anyhow::ensure!(pipeline.id == id, ID_MISMATCH);
        let _guard = crate::sync::lock(&WRITE_GUARD);
        let directory = self.directory(false);
        fs::create_dir_all(&directory)?;
        self.contained(&directory)?;
        let existing = self.file(id, false);
        let path = if create {
            if existing.is_ok() {
                return Err(SessionError::Conflict(id.to_string()).into());
            }
            if let Err(error) = existing {
                if !error.is::<SessionError>() {
                    return Err(error);
                }
            }
            directory.join(format!("{id}.{YAML_EXTENSION}"))
        } else {
            existing?
        };
        if create {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?
                .write_all(content.as_bytes())?;
        } else {
            let temporary = directory.join(format!("{}{UPDATE_SUFFIX}", uuid::Uuid::new_v4()));
            fs::write(&temporary, content)?;
            if let Err(error) = fs::rename(&temporary, &path) {
                let _ = fs::remove_file(&temporary);
                return Err(error.into());
            }
        }
        Ok(CatalogSaved {
            success: true,
            id: id.to_string(),
        })
    }
}

fn describe(pipeline: Pipeline) -> CatalogTemplate {
    CatalogTemplate {
        input_count: pipeline.inputs.len(),
        stage_count: pipeline.stages.len(),
        pipeline,
        source: SOURCE_SKILL.to_string(),
    }
}

fn collect_bounded<T>(
    ids: Vec<String>,
    limit: usize,
    load: impl Fn(&str) -> anyhow::Result<CatalogDocument<T>>,
) -> anyhow::Result<Vec<T>> {
    let mut remaining = limit;
    let mut entries = Vec::new();
    for id in ids {
        let document = load(&id)?;
        remaining = remaining
            .checked_sub(document.raw.len())
            .ok_or_else(|| anyhow::anyhow!(COLLECTION_TOO_LARGE))?;
        entries.push(document.parsed);
    }
    Ok(entries)
}

fn read_bounded(path: &Path) -> anyhow::Result<String> {
    let mut raw = String::new();
    fs::File::open(path)?
        .take(MAX_PIPELINE_BYTES + 1)
        .read_to_string(&mut raw)?;
    anyhow::ensure!(raw.len() as u64 <= MAX_PIPELINE_BYTES, FILE_TOO_LARGE);
    Ok(raw)
}

fn parse_pipeline(content: &str) -> anyhow::Result<Pipeline> {
    let pipeline = pipeline::parse(content)?;
    pipeline::topo_sort(&pipeline)?;
    Ok(pipeline)
}

pub fn validate_template(content: &str) -> CatalogValidation {
    match parse_pipeline(content) {
        Ok(_) => CatalogValidation {
            valid: true,
            errors: Vec::new(),
        },
        Err(error) => CatalogValidation {
            valid: false,
            errors: vec![error.to_string()],
        },
    }
}

fn reply<T: Serialize>(status: u16, body: &T) -> anyhow::Result<FlowReply> {
    Ok(FlowReply {
        status,
        body: serde_json::to_value(body)?,
    })
}

fn dispatch(
    catalog: &PipelineCatalog,
    method: &Method,
    segments: &[String],
    body: &Value,
) -> anyhow::Result<Option<FlowReply>> {
    let parts: Vec<&str> = segments.iter().map(String::as_str).collect();
    let document = || {
        serde_json::from_value::<CatalogWriteRequest>(body.clone())
            .map_err(|_| anyhow::anyhow!(INVALID_BODY))
    };
    let result = match (method, parts.as_slice()) {
        (Method::Get, [TEMPLATES]) => reply(HTTP_OK, &catalog.templates()?),
        (Method::Get, [TEMPLATES, id]) => reply(HTTP_OK, &catalog.template(id)?),
        (Method::Get, [FLOWS]) => reply(HTTP_OK, &catalog.flows()?),
        (Method::Get, [FLOWS, id]) => reply(HTTP_OK, &catalog.flow(id)?),
        (Method::Post, [TEMPLATES, VALIDATE]) => {
            reply(HTTP_OK, &validate_template(&document()?.content))
        }
        (Method::Post, [TEMPLATES]) => {
            let request = document()?;
            let id = request
                .id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!(ID_REQUIRED))?;
            reply(
                HTTP_CREATED,
                &catalog.save_template(id, &request.content, true)?,
            )
        }
        (Method::Put, [TEMPLATES, id]) => {
            let request = document()?;
            if let Some(body_id) = request.id.as_deref() {
                anyhow::ensure!(body_id == *id, ID_MISMATCH);
            }
            reply(
                HTTP_OK,
                &catalog.save_template(id, &request.content, false)?,
            )
        }
        _ => return Ok(None),
    };
    result.map(Some)
}

/// Catalogue routes use the host HTTP access gate before calling this handler.
pub fn handle(
    skill_root: Option<PathBuf>,
    method: &Method,
    path: &str,
    body: &Value,
) -> Option<FlowReply> {
    let suffix = path.strip_prefix(API_PREFIX)?;
    if !suffix.starts_with('/') {
        return None;
    }
    let result = skill_root
        .ok_or_else(|| SessionError::Config(SKILL_UNAVAILABLE.to_string()).into())
        .and_then(|root| {
            dispatch(
                &PipelineCatalog::new(root),
                method,
                &crate::path_segments::split(suffix.trim_matches('/')),
                body,
            )
        });
    match result {
        Ok(reply) => reply,
        Err(error) => {
            let api = ApiError::from(&error);
            reply(
                api.status.status(),
                &NativeErrorBody {
                    error: api.message,
                    code: api.status.code(),
                },
            )
            .ok()
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/catalog_tests.rs"]
mod tests;
