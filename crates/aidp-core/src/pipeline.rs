//! Pipeline YAML loader, schema validation, topo-sort.

use once_cell::sync::Lazy;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::models::enums::AgentRole;

const STAGE_TYPE_LOOP: &str = "loop";
const STAGE_TYPE_PIPELINE: &str = "pipeline";
const OUTPUT_NAME_KEY: &str = "name";
const INPUT_JIRA_KEYS: &str = "jira_keys";
const INPUT_JIRA_KEY: &str = "jira_key";
const INPUT_TARGET: &str = "target";
const RESEARCH_NAME_MARKERS: &[&str] = &["discover", "study", "explore", "research"];

pub use crate::models::pipeline::{
    FanOutSpec, InputSpec, KnowledgeLayer, KnowledgeSpec, MergeFindingsSpec, Pipeline, Placement,
    PlanTasksSpec, ScopeSpec, Settings, SkillsBy, Stage, ToolsSpec,
};

impl Stage {
    pub fn is_loop(&self) -> bool {
        self.r#type.as_deref() == Some(STAGE_TYPE_LOOP)
    }

    /// The role this stage runs as, `Executor` when it names none.
    pub fn agent_role(&self) -> AgentRole {
        self.agent
            .as_deref()
            .map_or(AgentRole::Executor, AgentRole::parse)
    }

    /// True when this stage references another pipeline (`type: pipeline`).
    pub fn is_composite(&self) -> bool {
        self.r#type.as_deref() == Some(STAGE_TYPE_PIPELINE)
    }

    /// A read-only research stage, detected by agent `analyzer` or a
    /// discover/study/explore/research name. Containers never qualify.
    pub fn is_research(&self) -> bool {
        if self.is_loop() || self.is_composite() {
            return false;
        }
        if self.agent_role() == AgentRole::Analyzer {
            return true;
        }
        let name = self.name.to_ascii_lowercase();
        RESEARCH_NAME_MARKERS
            .iter()
            .any(|marker| name.contains(marker))
    }

    /// The knowledge layer this stage curates, or `None` when it documents no
    /// project knowledge at all. Containers never qualify.
    pub fn knowledge_layer(&self) -> Option<KnowledgeLayer> {
        if self.is_loop() || self.is_composite() {
            return None;
        }
        self.knowledge.as_ref().and_then(|k| k.layer())
    }

    /// True when this stage documents the project itself (see [`Stage::knowledge`]).
    /// Containers never qualify.
    pub fn is_knowledge(&self) -> bool {
        self.knowledge_layer().is_some()
    }

    /// Where this stage's artefacts live: project knowledge, ticket research
    /// or this run's own folder, in that precedence order.
    pub fn placement(&self) -> Placement {
        match self.knowledge_layer() {
            Some(KnowledgeLayer::Feature) => Placement::KnowledgeFeature,
            Some(_) => Placement::Knowledge,
            None if self.is_research() => Placement::Ticket,
            None => Placement::Run,
        }
    }

    /// Extension-stripped output names, the key `{{ stages.S.outputs.NAME }}` uses.
    /// Use [`Stage::declared_outputs`] when the filename matters.
    pub fn output_names(&self) -> Vec<String> {
        self.outputs
            .iter()
            .filter_map(|v| match v {
                serde_yaml::Value::String(s) => Some(strip_ext(s)),
                serde_yaml::Value::Mapping(m) => m
                    .get(serde_yaml::Value::String(OUTPUT_NAME_KEY.into()))
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string()),
                _ => None,
            })
            .collect()
    }

    /// The declared output artifacts as written, with their extension
    /// (`changed-files.json`, `plan.md`).
    pub fn declared_outputs(&self) -> Vec<String> {
        self.outputs
            .iter()
            .filter_map(|v| match v {
                serde_yaml::Value::String(s) => Some(s.clone()),
                serde_yaml::Value::Mapping(m) => m
                    .get(serde_yaml::Value::String(OUTPUT_NAME_KEY.into()))
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string()),
                _ => None,
            })
            .collect()
    }

    /// The declared filename whose extension-stripped name is `name`.
    /// First match wins.
    pub fn declared_filename(&self, name: &str) -> Option<String> {
        self.declared_outputs()
            .into_iter()
            .find(|raw| strip_ext(raw) == name)
    }
}

fn strip_ext(s: &str) -> String {
    match s.rsplit_once('.') {
        Some((n, _)) => n.to_string(),
        None => s.to_string(),
    }
}

const SCHEMA: &str = include_str!("../../../schema/pipeline.schema.json");

/// Largest pipeline YAML `load` accepts, refused before any YAML parsing.
pub const MAX_PIPELINE_BYTES: u64 = 1024 * 1024;

/// Read at most `cap` bytes of `path`; a larger file is an error naming the cap.
fn read_capped(path: &Path, cap: u64) -> anyhow::Result<String> {
    use std::io::Read;
    let mut buf = String::new();
    std::fs::File::open(path)?
        .take(cap + 1)
        .read_to_string(&mut buf)?;
    if buf.len() as u64 > cap {
        anyhow::bail!("pipeline {} exceeds the {cap}-byte limit", path.display());
    }
    Ok(buf)
}

pub fn load(path: &Path) -> anyhow::Result<Pipeline> {
    use anyhow::Context;
    let s = read_capped(path, MAX_PIPELINE_BYTES)
        .with_context(|| format!("failed to read pipeline {}", path.display()))?;
    parse(&s).with_context(|| format!("failed to parse pipeline {}", path.display()))
}

/// Parses bounded YAML with the same schema checks used for files.
pub fn parse(text: &str) -> anyhow::Result<Pipeline> {
    if text.len() as u64 > MAX_PIPELINE_BYTES {
        anyhow::bail!("pipeline exceeds the {MAX_PIPELINE_BYTES}-byte limit");
    }
    let raw: serde_yaml::Value = serde_yaml::from_str(text)?;
    let json: serde_json::Value = serde_json::to_value(&raw)?;
    validate_json(&json)?;
    let p: Pipeline = serde_yaml::from_value(raw)?;
    Ok(p)
}

pub fn validate_path(path: &Path) -> anyhow::Result<()> {
    let _ = load(path)?;
    Ok(())
}

/// Compiled schema, built at most once per process (`jsonschema::JSONSchema`
/// compilation is the expensive part of every [`load`] call).
static COMPILED_SCHEMA: Lazy<jsonschema::JSONSchema> = Lazy::new(|| {
    let schema: serde_json::Value =
        serde_json::from_str(SCHEMA).expect("pipeline.schema.json must parse as JSON");
    jsonschema::JSONSchema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .compile(&schema)
        .unwrap_or_else(|e| panic!("pipeline schema compile: {e}"))
});

fn validate_json(instance: &serde_json::Value) -> anyhow::Result<()> {
    if let Err(errors) = COMPILED_SCHEMA.validate(instance) {
        let msgs: Vec<String> = errors
            .map(|e| format!("{}: {}", e.instance_path, e))
            .collect();
        anyhow::bail!("schema violations:\n  {}", msgs.join("\n  "));
    }
    Ok(())
}

/// Flatten loop bodies so topo-sort treats nested stages as members of the
/// loop's parent layer (the loop name itself stays as the visible node).
pub fn flat_stages(p: &Pipeline) -> Vec<&Stage> {
    let mut out = Vec::new();
    for s in &p.stages {
        out.push(s);
        if s.is_loop() {
            for b in &s.body {
                out.push(b);
            }
        }
    }
    out
}

/// Topo-sort returning parallel layers, each runnable concurrently.
/// Also checks every loop stage's `body` graph for cycles and unresolved deps.
pub fn topo_sort(p: &Pipeline) -> anyhow::Result<Vec<Vec<String>>> {
    let stages: Vec<&Stage> = p.stages.iter().collect();
    let mut layers = vec![];
    let mut placed = std::collections::HashSet::new();
    while placed.len() < stages.len() {
        let layer: Vec<String> = stages
            .iter()
            .filter(|s| !placed.contains(&s.name) && s.needs.iter().all(|n| placed.contains(n)))
            .map(|s| s.name.clone())
            .collect();
        if layer.is_empty() {
            anyhow::bail!(
                "cycle or unresolved dep among: {:?}",
                stages
                    .iter()
                    .filter(|s| !placed.contains(&s.name))
                    .map(|s| &s.name)
                    .collect::<Vec<_>>()
            );
        }
        for n in &layer {
            placed.insert(n.clone());
        }
        layers.push(layer);
    }
    for s in &stages {
        if s.is_loop() {
            topo_sort_loop_body(s, &placed)?;
        }
    }
    Ok(layers)
}

/// Topo-sorts one loop stage's `body`, rejecting a cycle or an unresolved
/// `needs`. Errors are prefixed with the loop name.
fn topo_sort_loop_body(
    loop_stage: &Stage,
    top_level: &std::collections::HashSet<String>,
) -> anyhow::Result<()> {
    let body_names: std::collections::HashSet<&str> =
        loop_stage.body.iter().map(|b| b.name.as_str()).collect();
    let mut placed: std::collections::HashSet<String> = std::collections::HashSet::new();
    while placed.len() < loop_stage.body.len() {
        let layer: Vec<&str> = loop_stage
            .body
            .iter()
            .filter(|b| {
                !placed.contains(&b.name)
                    && b.needs.iter().all(|n| {
                        placed.contains(n)
                            || (top_level.contains(n) && !body_names.contains(n.as_str()))
                    })
            })
            .map(|b| b.name.as_str())
            .collect();
        if layer.is_empty() {
            let remaining: Vec<&str> = loop_stage
                .body
                .iter()
                .filter(|b| !placed.contains(&b.name))
                .map(|b| b.name.as_str())
                .collect();
            anyhow::bail!(
                "loop {}: cycle or unresolved dep among: {:?}",
                loop_stage.name,
                remaining
            );
        }
        for n in &layer {
            placed.insert((*n).to_string());
        }
    }
    Ok(())
}

pub fn find_stage<'a>(p: &'a Pipeline, name: &str) -> Option<&'a Stage> {
    for s in &p.stages {
        if s.name == name {
            return Some(s);
        }
        for b in &s.body {
            if b.name == name {
                return Some(b);
            }
        }
    }
    None
}

/// Search roots used to resolve a composite stage's `uses:` reference,
/// in order: sibling directory, skill `templates/`, project-local flow.
pub fn discovery_roots(workspace_root: &Path, pipeline_path: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(parent) = pipeline_path.parent() {
        if !parent.as_os_str().is_empty() {
            roots.push(parent.to_path_buf());
        }
    }
    if let Ok(r) = std::env::var("AIDP_SKILL_ROOT") {
        roots.push(PathBuf::from(r).join("templates"));
    }
    roots.push(workspace_root.join(".aidp/pipeline/flow"));
    roots
}

/// A directory whose pipelines are archived: kept by explicit path only,
/// excluded from id discovery.
pub const ARCHIVE_DIR: &str = "legacy";

/// A directory of ticket flow definitions: they carry an `id` but no stages,
/// so they are never pipelines.
pub const FLOWS_DIR: &str = "flows";

/// Whether `path` sits inside an [`ARCHIVE_DIR`] or [`FLOWS_DIR`] directory.
pub fn is_archived(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str().eq_ignore_ascii_case(ARCHIVE_DIR)
            || c.as_os_str().eq_ignore_ascii_case(FLOWS_DIR)
    })
}

/// Discovers pipelines in `templates/` and `<repo>/.aidp/pipeline/flow/`,
/// skipping anything under a `legacy/` or `flows/` directory.
pub fn discover(roots: &[&Path]) -> anyhow::Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    for root in roots {
        if !root.exists() {
            continue;
        }
        for e in walkdir::WalkDir::new(root)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if e.path()
                .strip_prefix(root)
                .map(is_archived)
                .unwrap_or(false)
            {
                continue;
            }
            if e.path().extension().map(|x| x == "yaml").unwrap_or(false) {
                match e.path().metadata() {
                    Ok(meta) if meta.len() > MAX_PIPELINE_BYTES => {
                        continue;
                    }
                    Err(_) => continue,
                    Ok(_) => {}
                }
                if let Ok(s) = read_capped(e.path(), MAX_PIPELINE_BYTES) {
                    if let Ok(v) = serde_yaml::from_str::<serde_yaml::Value>(&s) {
                        if let Some(id) = v.get("id").and_then(|x| x.as_str()) {
                            out.push((id.to_string(), e.path().to_path_buf()));
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Validates and normalizes the inputs a driver passes to `run init` against
/// the pipeline's `inputs:` contract: defaults, aliases, required, allowed values.
pub fn validate_inputs(
    pipe: &Pipeline,
    provided: &BTreeMap<String, serde_json::Value>,
) -> anyhow::Result<BTreeMap<String, serde_json::Value>> {
    let mut warnings = Vec::new();
    validate_inputs_collect(pipe, provided, &mut warnings)
}

/// Same contract as [`validate_inputs`], but pushes a warning message (rather
/// than printing it) for each provided input the pipeline does not declare.
pub fn validate_inputs_collect(
    pipe: &Pipeline,
    provided: &BTreeMap<String, serde_json::Value>,
    warnings: &mut Vec<String>,
) -> anyhow::Result<BTreeMap<String, serde_json::Value>> {
    let mut out = provided.clone();

    for (k, spec) in &pipe.inputs {
        if !out.contains_key(k) {
            if let Some(def) = &spec.default {
                let json = serde_json::to_value(def)
                    .map_err(|e| anyhow::anyhow!("input '{}': invalid default: {}", k, e))?;
                out.insert(k.clone(), json);
            }
        }
    }

    let prompt_driven = out.get("prompt").is_some_and(crate::expr::truthy);
    for (k, spec) in &pipe.inputs {
        if spec.kind != "list" {
            continue;
        }
        let empty = match out.get(k) {
            None => true,
            Some(serde_json::Value::Array(a)) => a.is_empty(),
            Some(_) => false,
        };
        if !empty {
            continue;
        }
        let explicit = if k == INPUT_JIRA_KEYS {
            Some(INPUT_JIRA_KEY)
        } else {
            None
        };
        let generic = if prompt_driven {
            None
        } else {
            Some(INPUT_TARGET)
        };
        for candidate in explicit.into_iter().chain(generic) {
            if let Some(v) = out.get(candidate).cloned() {
                out.insert(k.clone(), serde_json::Value::Array(vec![v]));
                break;
            }
        }
    }

    for (k, spec) in &pipe.inputs {
        if spec.required && !out.contains_key(k) && spec.default.is_none() {
            anyhow::bail!(
                "pipeline '{}': required input '{}' not provided ({})",
                pipe.id,
                k,
                spec.description
            );
        }
    }

    for (k, spec) in &pipe.inputs {
        if spec.values.is_empty() {
            continue;
        }
        if let Some(v) = out.get(k) {
            if let Some(s) = v.as_str() {
                if !spec.values.iter().any(|allowed| allowed == s) {
                    anyhow::bail!(
                        "pipeline '{}': input '{}' value '{}' is not one of the allowed values: {}",
                        pipe.id,
                        k,
                        s,
                        spec.values.join(", ")
                    );
                }
            }
        }
    }

    for k in out.keys() {
        if k == INPUT_TARGET || k == INPUT_JIRA_KEY {
            continue;
        }
        if !pipe.inputs.contains_key(k) {
            warnings.push(format!(
                "warning: pipeline '{}': input '{}' is not declared by this pipeline",
                pipe.id, k
            ));
        }
    }

    Ok(out)
}

#[cfg(test)]
#[path = "../tests/unit/pipeline_inline_tests.rs"]
mod pipeline_inline_tests;

#[cfg(test)]
#[path = "../tests/unit/pipeline_tests.rs"]
mod pipeline_tests;
