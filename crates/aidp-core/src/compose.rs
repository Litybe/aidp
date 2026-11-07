//! Pipeline composition: inline a `type: pipeline` stage by expanding the
//! referenced pipeline's stages into the parent, at init time.
//!
//! Mirrors how `pipeline::flat_stages` already inlines loop bodies — but the
//! stages come from another YAML file. The expansion is purely structural: the
//! result is a single flat `Pipeline` with no `type: pipeline` stages left, so
//! the orchestrator, prompt renderer, scope resolver, topo-sort and
//! `find_stage` all keep operating unchanged. One run, one run-state.
//!
//! Namespacing uses `__` (NOT `.`) because `prompt::eval_expr` splits template
//! expressions on `.` and joins stage names into filesystem paths — a `.` in a
//! stage name would be both misparsed and turned into a nested directory.

use crate::models::enums::ModelTier;
use crate::pipeline::{self, InputSpec, Pipeline, Stage};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

const MAX_DEPTH: usize = 8;
/// Default ceiling on the total flattened stage count (top-level + loop bodies)
/// after composition. Bounds composition *width*, complementing `MAX_DEPTH`
/// (which bounds nesting). Overridable per pipeline via
/// `settings.max_total_stages`.
pub const DEFAULT_MAX_TOTAL_STAGES: usize = 256;
/// LOC-48: `pub(crate)` (not private) so `workflow_emit::composite_span_len`
/// can recognize a composite's namespaced stages by prefix without
/// hardcoding the separator a second time.
pub(crate) const SEP: &str = "__";

/// A rewrite function applied to the inner text of a `{{ ... }}` span. Returns
/// the full replacement for the entire span (it decides whether to keep the
/// `{{ }}` wrapping), or `None` to leave the span untouched.
type Rewriter<'a> = dyn Fn(&str) -> Option<String> + 'a;

/// Resolve a composite stage's `uses:` reference to a pipeline YAML file.
/// Tries, in order: a direct file path, `<root>/<ref>` and `<root>/<ref>.yaml`
/// under each search root, then a match by pipeline `id` via discovery.
pub fn resolve_ref(reference: &str, roots: &[&Path]) -> anyhow::Result<PathBuf> {
    // LOC-27 (7a): a direct path is honoured only when it canonicalizes to a
    // file under one of the composition roots, so an absolute or `../..`
    // reference cannot pull in an arbitrary YAML from elsewhere on disk.
    let direct = Path::new(reference);
    let mut refused_direct = false;
    if direct.is_file() {
        if under_roots(direct, roots) {
            return Ok(direct.to_path_buf());
        }
        refused_direct = true;
    }
    for root in roots {
        let cand = root.join(reference);
        if cand.is_file() && under_roots(&cand, roots) {
            return Ok(cand);
        }
        let cand_yaml = root.join(format!("{reference}.yaml"));
        if cand_yaml.is_file() && under_roots(&cand_yaml, roots) {
            return Ok(cand_yaml);
        }
    }
    let mut id_match_outside_roots = false;
    for (id, path) in pipeline::discover(roots)? {
        if id == reference {
            if under_roots(&path, roots) {
                return Ok(path);
            }
            id_match_outside_roots = true;
        }
    }
    if refused_direct || id_match_outside_roots {
        anyhow::bail!(
            "pipeline reference '{reference}' refused: the file lies outside every composition root"
        );
    }
    anyhow::bail!(
        "unknown pipeline reference '{reference}' (not a file, and no discovered pipeline has that id)"
    )
}

/// True when `path` canonicalizes to a location under one of `roots`
/// (roots canonicalized too; an uncanonicalizable root is skipped).
fn under_roots(path: &Path, roots: &[&Path]) -> bool {
    let Ok(canon) = std::fs::canonicalize(path) else {
        return false;
    };
    roots.iter().any(|root| {
        std::fs::canonicalize(root)
            .map(|r| canon.starts_with(&r))
            .unwrap_or(false)
    })
}

/// Expand all `type: pipeline` stages in `pipe`, returning a flat pipeline with
/// every referenced pipeline inlined and namespaced.
pub fn expand(pipe: &Pipeline, roots: &[&Path]) -> anyhow::Result<Pipeline> {
    let mut visited = vec![pipe.id.clone()];
    let (stages, _entries) = expand_inner(pipe, roots, &mut visited, 0)?;
    let mut out = pipe.clone();
    out.stages = stages;
    check_duplicate_names(&out)?;

    // Width guard: a shallow-but-wide composite tree can still explode the flat
    // pipeline even within MAX_DEPTH. Count exactly as `flat_stages` materializes
    // (top-level stages + each loop body's stages).
    let limit = out
        .settings
        .max_total_stages
        .unwrap_or(DEFAULT_MAX_TOTAL_STAGES);
    let total = out.stages.len() + out.stages.iter().map(|s| s.body.len()).sum::<usize>();
    if total > limit {
        anyhow::bail!(
            "pipeline '{}': composition expands to {} stages (top-level + loop bodies), exceeding the limit of {} — raise `settings.max_total_stages` if this is intentional",
            out.id,
            total,
            limit
        );
    }
    Ok(out)
}

/// Returns the flattened top-level stage list for `pipe`, with composites
/// inlined and self-healed (needs remapped, output aliases rewritten) at this
/// level, together with the set of names within that list which are `pipe`'s
/// own entry points (its top-level stages with empty `needs`, recursively
/// resolved through any composite entry down to its real, further-nested
/// stage names). A caller that itself embeds `pipe` as a `uses:` child uses
/// this set — NOT `pipe.stages` filtered by empty `needs` — to know which
/// stages in the returned list should inherit the wrapping composite stage's
/// own `needs`/`gate`/`when`, since a raw entry stage that is itself
/// composite never survives expansion under its own name. Loop containers
/// pass through with their bodies intact.
fn expand_inner(
    pipe: &Pipeline,
    roots: &[&Path],
    visited: &mut Vec<String>,
    depth: usize,
) -> anyhow::Result<(Vec<Stage>, HashSet<String>)> {
    // Reject composites AND nested loops inside loop bodies (unsupported in v1).
    // A nested loop would be silently flattened by the emitter into one-shot
    // calls that never iterate, so fail loud at compose time instead.
    for s in &pipe.stages {
        if s.is_loop() {
            for b in &s.body {
                if b.is_composite() {
                    anyhow::bail!(
                        "pipeline '{}': `type: pipeline` inside a loop body is not supported (stage '{}')",
                        pipe.id,
                        b.name
                    );
                }
                if b.is_loop() {
                    anyhow::bail!(
                        "pipeline '{}': a `type: loop` nested inside a loop body is not supported (stage '{}') — it would never iterate",
                        pipe.id,
                        b.name
                    );
                }
            }
        }
    }

    let mut out: Vec<Stage> = Vec::new();
    // `pipe`'s own entry points, in the namespace of `out` — see the doc
    // comment above for why this can't be recomputed from `pipe.stages` alone
    // once a composite entry's expansion is inlined.
    let mut entries: HashSet<String> = HashSet::new();
    // composite original name -> namespaced terminal stage names
    let mut composite_terminals: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // (composite name, declared output name) -> (namespaced inner stage, inner output)
    let mut alias_map: BTreeMap<(String, String), (String, String)> = BTreeMap::new();

    for s in &pipe.stages {
        if !s.is_composite() {
            if s.needs.is_empty() {
                entries.insert(s.name.clone());
            }
            out.push(s.clone());
            continue;
        }

        // --- composite stage ---
        if s.agent.is_some() {
            anyhow::bail!("composite stage '{}' must not set 'agent'", s.name);
        }
        if !s.skills.is_empty() {
            anyhow::bail!("composite stage '{}' must not set 'skills'", s.name);
        }
        if !s.body.is_empty() {
            anyhow::bail!("composite stage '{}' must not set 'body'", s.name);
        }
        if depth + 1 > MAX_DEPTH {
            let mut chain = visited.clone();
            chain.push(format!("(uses {})", s.uses.clone().unwrap_or_default()));
            anyhow::bail!(
                "composition exceeds max depth {MAX_DEPTH}: {}",
                chain.join(" -> ")
            );
        }

        let reference = s
            .uses
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("composite stage '{}' missing 'uses'", s.name))?;
        let child_path = resolve_ref(reference, roots)
            .map_err(|e| anyhow::anyhow!("composite stage '{}': {e}", s.name))?;
        let child = {
            use anyhow::Context;
            pipeline::load(&child_path).with_context(|| {
                format!(
                    "composite stage '{}': uses '{reference}' -> {}",
                    s.name,
                    child_path.display()
                )
            })?
        };

        if visited.contains(&child.id) {
            let mut chain = visited.clone();
            chain.push(child.id.clone());
            anyhow::bail!("composition cycle detected: {}", chain.join(" -> "));
        }

        // Child search roots: the child file's own directory first, then inherited.
        let mut child_roots: Vec<PathBuf> = Vec::new();
        if let Some(p) = child_path.parent() {
            child_roots.push(p.to_path_buf());
        }
        child_roots.extend(roots.iter().map(|p| p.to_path_buf()));
        let child_root_refs: Vec<&Path> = child_roots.iter().map(|p| p.as_path()).collect();

        visited.push(child.id.clone());
        let (child_flat, child_entries) =
            expand_inner(&child, &child_root_refs, visited, depth + 1)?;
        visited.pop();

        // Validate required child inputs are satisfied by the composite mapping.
        let input_map = build_input_map(s);
        for (k, spec) in &child.inputs {
            if spec.required && !input_map.contains_key(k) && spec.default.is_none() {
                anyhow::bail!(
                    "composite stage '{}' (uses '{}'): required input '{}' not provided via with:/inputs:",
                    s.name,
                    child.id,
                    k
                );
            }
        }
        // Reject inputs the child does NOT declare — these are silently dropped
        // today (the root cause of the dev-composed `axes` drift), so fail loud.
        for k in input_map.keys() {
            if !child.inputs.contains_key(k) {
                anyhow::bail!(
                    "composite stage '{}' (uses '{}'): input '{}' is not declared by the referenced pipeline",
                    s.name,
                    child.id,
                    k
                );
            }
        }

        // The set of stage names defined inside the child (top-level + bodies),
        // used to decide which `stages.X` refs to namespace.
        let child_names: HashSet<String> = child_flat
            .iter()
            .flat_map(|st| {
                let mut v = vec![st.name.clone()];
                v.extend(st.body.iter().map(|b| b.name.clone()));
                v
            })
            .collect();

        let ns = s.name.clone();
        let child_inputs = child.inputs.clone();
        // Carry the child's own settings so a reused sub-pipeline's
        // `{{ settings.X }}` (e.g. a loop's max_correction_iterations) binds to
        // the child's value, not the parent's (WI-P0-4).
        let child_settings = child.settings.clone();
        let rewriter: Box<Rewriter> = Box::new(move |expr: &str| {
            child_expr_rewrite(
                expr,
                &ns,
                &child_names,
                &input_map,
                &child_inputs,
                &child_settings,
            )
        });

        // Namespace + rewrite every child stage.
        let mut inlined: Vec<Stage> = child_flat
            .iter()
            .map(|cs| rewrite_child_stage(cs, &s.name, s.model, rewriter.as_ref()))
            .collect();

        // Entry stages (the child's own real entry points, resolved through
        // any further nesting via `child_entries` — NOT `child.stages`
        // filtered by empty `needs`, which only holds if the child's entry is
        // not itself composite) inherit the composite stage's needs / gate /
        // when.
        let entry_set: HashSet<String> = child_entries
            .iter()
            .map(|e| format!("{}{SEP}{}", s.name, e))
            .collect();
        let gate_truthy = gate_is_truthy(&s.gate);
        for st in &mut inlined {
            // LOC-51: the composite's `when:` reaches EVERY inlined stage, not
            // just the entries — a skipped `needs` does not skip its successor,
            // so a non-entry stage (e.g. `review__review`) would otherwise run.
            // Loop bodies follow their container.
            if let Some(w) = &s.when {
                st.composite_when.push(w.clone());
            }
            if entry_set.contains(&st.name) {
                st.needs = s.needs.clone();
                // Unconditional overwrite (not `if st.composite_entry.is_none()`):
                // a doubly-composite entry was already stamped by the recursive
                // call that produced `child_flat`, and this outer level's stamp
                // must win — one Workflow-pass boundary per top-level `uses:`,
                // not per nesting level.
                st.composite_entry = Some(s.name.clone());
                if gate_truthy && st.gate.is_none() {
                    st.gate = s.gate.clone();
                }
                if st.when.is_none() {
                    st.when = s.when.clone();
                }
            }
        }
        // If `s` itself has no `needs` declared in `pipe`, it is one of
        // `pipe`'s own entry points — and since `s` is composite, its
        // position in `out` is really held by `entry_set` (already
        // namespaced), which the caller one level up must treat as entries.
        if s.needs.is_empty() {
            entries.extend(entry_set.iter().cloned());
        }

        // Record terminals (for downstream `needs: [composite]` remapping).
        // Computed over `child_flat` — the child's fully expanded, internally
        // namespaced stage list — NOT `child.stages`: a raw terminal that is
        // itself composite never survives expansion under its own name, so
        // walking the raw declaration yields a stage that does not exist
        // (mirror of the `entry_set` / `child_entries` treatment on the
        // inbound side).
        composite_terminals.insert(s.name.clone(), namespaced_terminals(&child_flat, &s.name));

        // Record output aliases declared on the composite stage.
        record_output_aliases(s, &child.stages, &inlined, &mut alias_map)?;

        out.extend(inlined);
    }

    // --- self-heal pass for this level ---
    if !composite_terminals.is_empty() {
        for st in &mut out {
            remap_needs(&mut st.needs, &composite_terminals);
            for b in &mut st.body {
                remap_needs(&mut b.needs, &composite_terminals);
            }
        }
    }
    if !alias_map.is_empty() {
        let alias_rewriter: Box<Rewriter> =
            Box::new(move |expr: &str| alias_expr_rewrite(expr, &alias_map));
        for st in &mut out {
            apply_templates(st, alias_rewriter.as_ref());
            for b in &mut st.body {
                apply_templates(b, alias_rewriter.as_ref());
            }
        }
    }

    Ok((out, entries))
}

/// Build the composite stage's input mapping: `inputs` first, then `with`
/// overriding. Values are stringified parent-context expressions/literals.
fn build_input_map(s: &Stage) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for (k, v) in &s.inputs {
        m.insert(k.clone(), yaml_scalar_to_string(v));
    }
    for (k, v) in &s.with {
        m.insert(k.clone(), yaml_scalar_to_string(v));
    }
    m
}

/// Rewrite a child template expression: substitute `inputs.K` with the parent
/// mapping (or the child default), and namespace `stages.X` refs to inner stages.
fn child_expr_rewrite(
    expr: &str,
    ns: &str,
    child_names: &HashSet<String>,
    input_map: &BTreeMap<String, String>,
    child_inputs: &BTreeMap<String, InputSpec>,
    child_settings: &pipeline::Settings,
) -> Option<String> {
    let parts: Vec<&str> = expr.split('.').map(str::trim).collect();
    match parts.as_slice() {
        ["inputs", k] => {
            if let Some(val) = input_map.get(*k) {
                Some(val.clone())
            } else {
                child_inputs
                    .get(*k)
                    .and_then(|spec| spec.default.as_ref())
                    .map(yaml_scalar_to_string)
            }
        }
        // Bind the child's own settings for its inlined stages; an unknown key
        // falls through (None) to be resolved against the parent at runtime.
        ["settings", k] => child_setting_literal(child_settings, k),
        ["stages", st, rest @ ..] if child_names.contains(*st) => {
            let mut segs = vec![format!("{st}")];
            segs.extend(rest.iter().map(|r| r.to_string()));
            // namespace only the stage segment
            segs[0] = format!("{ns}{SEP}{st}");
            Some(format!("{{{{ stages.{} }}}}", segs.join(".")))
        }
        _ => None,
    }
}

/// Resolve `settings.<k>` against the child pipeline's own settings to a scalar
/// literal. Known fields always resolve (they carry serde defaults); unknown
/// keys fall through to `extra`, then to `None` (→ resolved against the parent).
fn child_setting_literal(settings: &pipeline::Settings, k: &str) -> Option<String> {
    match k {
        "max_correction_iterations" => Some(settings.max_correction_iterations.to_string()),
        "max_total_stages" => settings.max_total_stages.map(|n| n.to_string()),
        other => settings.extra.get(other).map(yaml_scalar_to_string),
    }
}

/// Rewrite a sibling reference to a composite stage's aliased output, e.g.
/// `stages.qa.outputs.report` -> `stages.qa__scan.outputs.report`.
fn alias_expr_rewrite(
    expr: &str,
    alias_map: &BTreeMap<(String, String), (String, String)>,
) -> Option<String> {
    let parts: Vec<&str> = expr.split('.').map(str::trim).collect();
    match parts.as_slice() {
        ["stages", comp, "outputs", out, rest @ ..] => {
            let key = (comp.to_string(), out.to_string());
            alias_map.get(&key).map(|(stage, inner_out)| {
                let mut tail = vec![stage.clone(), "outputs".to_string(), inner_out.clone()];
                tail.extend(rest.iter().map(|r| r.to_string()));
                format!("{{{{ stages.{} }}}}", tail.join("."))
            })
        }
        _ => None,
    }
}

/// F-5 (Lot 3, Capitalisation): checklist of every `Stage` field that
/// references a stage name — split across the two functions below because
/// they are two DIFFERENT encodings of the same idea (a bare stage-name list
/// vs. an embedded handlebars span), not two arbitrarily duplicated
/// functions, so they must NOT be merged. Any new field of either kind must
/// be added to both this list and its matching function:
///
///   encoding "bare name"        (namespaced here, in `rewrite_child_stage`)
///     - `needs`
///     - `merge_findings.from`
///   encoding "handlebars span"  (`{{ stages.<name>.* }}`, namespaced in
///                                `apply_templates` via `rewrite_str`)
///     - `fan_out.from`
///     - `plan_tasks.from`
///
/// Namespace a child stage: rename it and its needs, inherit the composite
/// model as a default, rewrite its template fields, and recurse into its body.
fn rewrite_child_stage(
    cs: &Stage,
    ns: &str,
    composite_model: Option<ModelTier>,
    f: &Rewriter,
) -> Stage {
    let mut out = cs.clone();
    out.name = format!("{ns}{SEP}{}", cs.name);
    out.needs = cs.needs.iter().map(|n| format!("{ns}{SEP}{n}")).collect();
    // C6 (Lot 3, Capitalisation): `merge_findings.from` names producer stages
    // the same way `needs` does (bare stage names, not a handlebars ref), so it
    // must be namespaced the same way or composition leaves it pointing at a
    // pre-composition stage name that no longer exists in the composed graph.
    if let Some(mf) = &mut out.merge_findings {
        mf.from = mf.from.iter().map(|n| format!("{ns}{SEP}{n}")).collect();
    }
    if out.model.is_none() {
        out.model = composite_model;
    }
    apply_templates(&mut out, f);
    out.body = cs
        .body
        .iter()
        .map(|b| rewrite_child_stage(b, ns, composite_model, f))
        .collect();
    out
}

/// Apply the rewriter to every templated field of a stage (not its body — the
/// caller recurses into bodies via `rewrite_child_stage`).
fn apply_templates(s: &mut Stage, f: &Rewriter) {
    for v in s.inputs.values_mut() {
        *v = rewrite_value(v, f);
    }
    for v in s.with.values_mut() {
        *v = rewrite_value(v, f);
    }
    if let Some(w) = s.when.take() {
        s.when = Some(rewrite_value(&w, f));
    }
    s.composite_when = s
        .composite_when
        .iter()
        .map(|w| rewrite_value(w, f))
        .collect();
    if let Some(g) = s.gate.take() {
        s.gate = Some(rewrite_value(&g, f));
    }
    if let Some(mi) = s.max_iterations.take() {
        s.max_iterations = Some(rewrite_value(&mi, f));
    }
    s.outputs = s.outputs.iter().map(|o| rewrite_value(o, f)).collect();
    // A dynamic fan-out's `from: {{ stages.<producer>.signals.<prop> }}` must be
    // namespaced along with the producer stage, or the emitter can't resolve it.
    if let Some(fo) = &mut s.fan_out {
        if let Some(from) = fo.from.take() {
            fo.from = Some(rewrite_str(&from, f));
        }
    }
    // Same for a plan-driven stage's `from: {{ stages.<planner>.outputs.<name> }}`:
    // the producer is namespaced by composition, so the ref must follow or the
    // emitter cannot locate plan.json.
    if let Some(pt) = &mut s.plan_tasks {
        pt.from = rewrite_str(&pt.from, f);
    }
    if let Some(sc) = &mut s.scope {
        sc.allowed = rewrite_str_vec(&sc.allowed, f);
        sc.denied = rewrite_str_vec(&sc.denied, f);
        sc.writes = rewrite_str_vec(&sc.writes, f);
        sc.reads = rewrite_str_vec(&sc.reads, f);
    }
}

fn rewrite_str_vec(v: &[String], f: &Rewriter) -> Vec<String> {
    v.iter().map(|s| rewrite_str(s, f)).collect()
}

/// Recursively rewrite template spans inside a YAML value.
fn rewrite_value(v: &serde_yaml::Value, f: &Rewriter) -> serde_yaml::Value {
    match v {
        serde_yaml::Value::String(s) => serde_yaml::Value::String(rewrite_str(s, f)),
        serde_yaml::Value::Sequence(seq) => {
            serde_yaml::Value::Sequence(seq.iter().map(|e| rewrite_value(e, f)).collect())
        }
        serde_yaml::Value::Mapping(m) => {
            let mut out = serde_yaml::Mapping::new();
            for (k, val) in m {
                out.insert(k.clone(), rewrite_value(val, f));
            }
            serde_yaml::Value::Mapping(out)
        }
        other => other.clone(),
    }
}

/// Walk `{{ ... }}` spans in a string (same grammar as `prompt::substitute`),
/// replacing each via `f` when it returns `Some`.
fn rewrite_str(s: &str, f: &Rewriter) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("{{") {
        let Some(end) = rest[start + 2..].find("}}") else {
            break;
        };
        out.push_str(&rest[..start]);
        let raw = &rest[start + 2..start + 2 + end];
        let expr = raw.trim();
        match f(expr) {
            Some(replacement) => out.push_str(&replacement),
            None => {
                out.push_str("{{");
                out.push_str(raw);
                out.push_str("}}");
            }
        }
        rest = &rest[start + 2 + end + 2..];
    }
    out.push_str(rest);
    out
}

/// Terminal top-level stages of a child pipeline (no other top-level stage
/// needs them), namespaced. Loop containers never reach `success`, so a
/// terminal loop is replaced by its body terminals.
///
/// `stages` must be the child's ALREADY-EXPANDED stage list (`child_flat`),
/// never its raw declarations: only there is the needs graph complete and are
/// nested composites resolved down to the real stage names that survive
/// expansion.
fn namespaced_terminals(stages: &[Stage], ns: &str) -> Vec<String> {
    let needed: HashSet<&str> = stages
        .iter()
        .flat_map(|s| s.needs.iter().map(|n| n.as_str()))
        .collect();
    let mut terms = Vec::new();
    for s in stages {
        if needed.contains(s.name.as_str()) {
            continue;
        }
        if s.is_loop() {
            for bt in body_terminals(&s.body) {
                terms.push(format!("{ns}{SEP}{bt}"));
            }
        } else {
            terms.push(format!("{ns}{SEP}{}", s.name));
        }
    }
    terms
}

fn body_terminals(body: &[Stage]) -> Vec<String> {
    let needed: HashSet<&str> = body
        .iter()
        .flat_map(|s| s.needs.iter().map(|n| n.as_str()))
        .collect();
    body.iter()
        .filter(|s| !needed.contains(s.name.as_str()) && !s.is_loop())
        .map(|s| s.name.clone())
        .collect()
}

fn remap_needs(needs: &mut Vec<String>, map: &BTreeMap<String, Vec<String>>) {
    let mut new: Vec<String> = Vec::new();
    for n in needs.iter() {
        match map.get(n) {
            Some(terms) => {
                for t in terms {
                    if !new.contains(t) {
                        new.push(t.clone());
                    }
                }
            }
            None => {
                if !new.contains(n) {
                    new.push(n.clone());
                }
            }
        }
    }
    *needs = new;
}

/// Record output aliases declared on a composite stage so sibling references
/// `{{ stages.<composite>.outputs.<name> }}` can be rewritten to the inner
/// namespaced stage output.
fn record_output_aliases(
    s: &Stage,
    child_stages: &[Stage],
    inlined: &[Stage],
    alias_map: &mut BTreeMap<(String, String), (String, String)>,
) -> anyhow::Result<()> {
    for o in &s.outputs {
        match o {
            serde_yaml::Value::String(name) => {
                // Alias to the inner stage that declares an output with this name.
                let inner = find_declaring_stage(name, inlined, &s.name).map_err(|e| {
                    anyhow::anyhow!("composite stage '{}': output '{}': {e}", s.name, name)
                })?;
                if let Some((stage, out)) = inner {
                    alias_map.insert((s.name.clone(), strip_ext(name)), (stage, out));
                }
            }
            serde_yaml::Value::Mapping(m) => {
                let from = m
                    .get(serde_yaml::Value::String("from".into()))
                    .and_then(|v| v.as_str());
                let declared = m
                    .get(serde_yaml::Value::String("name".into()))
                    .and_then(|v| v.as_str());
                if let Some(from) = from {
                    let (inner_stage, inner_out) = from.split_once('.').ok_or_else(|| {
                        anyhow::anyhow!(
                            "composite stage '{}': output `from: {}` must be `<stage>.<output>`",
                            s.name,
                            from
                        )
                    })?;
                    // Verify the inner stage exists in the child.
                    if !child_stages.iter().any(|cs| cs.name == inner_stage)
                        && !child_stages
                            .iter()
                            .flat_map(|cs| cs.body.iter())
                            .any(|b| b.name == inner_stage)
                    {
                        anyhow::bail!(
                            "composite stage '{}': output `from: {}` references unknown inner stage '{}'",
                            s.name,
                            from,
                            inner_stage
                        );
                    }
                    let exposed = declared
                        .map(strip_ext)
                        .unwrap_or_else(|| strip_ext(inner_out));
                    alias_map.insert(
                        (s.name.clone(), exposed),
                        (
                            format!("{}{SEP}{}", s.name, inner_stage),
                            strip_ext(inner_out),
                        ),
                    );
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Find the (already namespaced) inner stage declaring `output_name`. Errors if
/// more than one inner stage declares it (ambiguous).
fn find_declaring_stage(
    output_name: &str,
    inlined: &[Stage],
    _ns: &str,
) -> anyhow::Result<Option<(String, String)>> {
    let target = strip_ext(output_name);
    let mut hits: Vec<String> = Vec::new();
    for st in inlined {
        if st.output_names().contains(&target) {
            hits.push(st.name.clone());
        }
        for b in &st.body {
            if b.output_names().contains(&target) {
                hits.push(b.name.clone());
            }
        }
    }
    match hits.len() {
        0 => Ok(None),
        1 => Ok(Some((hits.remove(0), target))),
        _ => anyhow::bail!(
            "ambiguous output '{output_name}' declared by multiple inner stages: {}",
            hits.join(", ")
        ),
    }
}

fn gate_is_truthy(gate: &Option<serde_yaml::Value>) -> bool {
    match gate {
        None => false,
        Some(serde_yaml::Value::Bool(b)) => *b,
        Some(serde_yaml::Value::Null) => false,
        Some(_) => true,
    }
}

fn yaml_scalar_to_string(v: &serde_yaml::Value) -> String {
    match v {
        serde_yaml::Value::String(s) => s.clone(),
        other => serde_yaml::to_string(other)
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| format!("{other:?}")),
    }
}

fn strip_ext(s: &str) -> String {
    match s.rsplit_once('.') {
        Some((n, _)) => n.to_string(),
        None => s.to_string(),
    }
}

/// Bail if any stage name (top-level or body) appears more than once.
fn check_duplicate_names(pipe: &Pipeline) -> anyhow::Result<()> {
    let mut seen: HashSet<&str> = HashSet::new();
    for s in &pipe.stages {
        if !seen.insert(s.name.as_str()) {
            anyhow::bail!("duplicate stage name after expansion: '{}'", s.name);
        }
        for b in &s.body {
            if !seen.insert(b.name.as_str()) {
                anyhow::bail!("duplicate stage name after expansion: '{}'", b.name);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {

    #[test]
    fn it_resolve_ref_refuses_absolute_path_outside_roots() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let yaml = outside.path().join("evil.yaml");
        std::fs::write(
            &yaml,
            "version: 5
id: evil
stages: []
",
        )
        .unwrap();
        let reference = yaml.to_string_lossy().to_string();
        let err = resolve_ref(&reference, &[root.path()]).unwrap_err();
        assert!(err.to_string().contains(&reference), "{err}");
    }

    #[test]
    fn it_resolve_ref_accepts_relative_ref_inside_root() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("child.yaml"),
            "version: 5
id: child
stages: []
",
        )
        .unwrap();
        let got = resolve_ref("child", &[root.path()]).unwrap();
        assert!(got.ends_with("child.yaml"));
        let abs = root.path().join("child.yaml");
        let got = resolve_ref(&abs.to_string_lossy(), &[root.path()]).unwrap();
        assert_eq!(got, abs);
    }

    #[test]
    fn it_resolve_ref_refuses_id_match_via_symlink_outside_roots() {
        // A symlinked YAML *inside* a composition root, whose `id:` matches the
        // reference, must not resolve via the pipeline::discover ID-match path
        // when it canonicalizes to a location outside every root.
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let real = outside.path().join("evil.yaml");
        std::fs::write(
            &real,
            "version: 5
id: evil-id
stages: []
",
        )
        .unwrap();
        let link = root.path().join("evil-link.yaml");
        #[cfg(unix)]
        let symlinked = std::os::unix::fs::symlink(&real, &link).is_ok();
        #[cfg(windows)]
        let symlinked = std::os::windows::fs::symlink_file(&real, &link).is_ok();
        #[cfg(not(any(unix, windows)))]
        let symlinked = false;
        if !symlinked {
            // Unprivileged environment (common on Windows without dev mode /
            // admin rights) — symlink creation itself failed, nothing to test.
            return;
        }
        let err = resolve_ref("evil-id", &[root.path()]).unwrap_err();
        assert!(err.to_string().contains("refused"), "{err}");
    }
    use super::*;
    use crate::pipeline;

    fn templates_dir() -> PathBuf {
        aidp_testkit::fixtures::templates_dir()
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aidp-compose-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn stage<'a>(p: &'a Pipeline, name: &str) -> &'a Stage {
        p.stages
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("missing stage {name}"))
    }

    fn input_str(s: &Stage, key: &str) -> String {
        match s.inputs.get(key) {
            Some(serde_yaml::Value::String(v)) => v.clone(),
            other => panic!("input {key} not a string: {other:?}"),
        }
    }

    #[test]
    fn it_expand_dev_composed_inlines_audit() {
        let tpl = templates_dir();
        let pipe = pipeline::load(&tpl.join("dev-composed.yaml")).unwrap();
        let roots = [tpl.as_path()];
        let eff = expand(&pipe, &roots).unwrap();

        let names: Vec<&str> = eff.stages.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                // `dev-composed` is plan-driven now: its implementation stage is
                // `implement` (compiled from plan.json), not the old `code` fan-out.
                "prepare_branch",
                // Prompt mode's demand stage: it turns the free text into the
                // requirements note `plan` reads. Guarded on `inputs.prompt`, so a
                // Jira run drops it at compile time — it is listed here because
                // `expand` runs before any `when:` is evaluated.
                "discover_prompt",
                "plan",
                "implement",
                "qa__explore",
                "qa__dim_correctness",
                "qa__dim_security",
                "qa__dim_perf",
                "qa__dim_maintainability",
                "qa__verify",
                "qa__report",
                "pr"
            ]
        );

        // No composites remain.
        assert!(eff.stages.iter().all(|s| !s.is_composite()));

        // Entry of the child inherits the composite's needs.
        assert_eq!(stage(&eff, "qa__explore").needs, vec!["implement"]);
        // Internal child needs are namespaced.
        assert_eq!(
            stage(&eff, "qa__dim_correctness").needs,
            vec!["qa__explore"]
        );
        assert_eq!(
            stage(&eff, "qa__verify").needs,
            vec![
                "qa__dim_correctness",
                "qa__dim_security",
                "qa__dim_perf",
                "qa__dim_maintainability"
            ]
        );
        assert_eq!(stage(&eff, "qa__report").needs, vec!["qa__verify"]);
        // Downstream `needs: [qa]` remapped to the child terminal.
        assert_eq!(stage(&eff, "pr").needs, vec!["qa__report"]);

        // Child input mapped to a parent expression stays a parent-context ref.
        assert_eq!(
            input_str(stage(&eff, "qa__explore"), "path"),
            "{{ inputs.target_path }}"
        );
        // Sibling reference to the composite output is rewritten via the alias.
        assert_eq!(
            input_str(stage(&eff, "pr"), "audit_report"),
            "{{ stages.qa__report.outputs.audit-report }}"
        );

        // find_stage + topo_sort operate on namespaced names.
        assert!(pipeline::find_stage(&eff, "qa__report").is_some());
        let layers = pipeline::topo_sort(&eff).unwrap();
        assert!(layers.iter().flatten().any(|n| n == "qa__report"));
    }

    // A composite entry stage that is itself composite (`type: pipeline`) must
    // not swallow an outer composite's `needs`: the real, doubly-namespaced
    // entry stage has to inherit it transitively. `templates/dev.yaml`
    // composes `discover` (`needs: [prepare_branch]`, uses `dev-discovery`),
    // whose own first stage `knowledge` is itself composite (`uses:
    // knowledge-read`, no `needs:` declared — an entry of `dev-discovery`),
    // which composes the single stage `knowledge_read`. Before the fix,
    // `entry_set` was computed from `child.stages` (the child's raw,
    // unexpanded declarations) using each raw stage's own name — but a raw
    // entry that is itself composite never survives expansion under that
    // name, so the match silently failed and `discover__knowledge__knowledge_read`
    // ended up with `needs: []`, free to dispatch before `prepare_branch`.
    #[test]
    fn it_triple_nested_composite_propagates_needs_to_the_real_entry() {
        let tpl = templates_dir();
        let pipe = pipeline::load(&tpl.join("dev.yaml")).unwrap();
        let eff = expand(&pipe, &[tpl.as_path()]).unwrap();

        assert_eq!(
            stage(&eff, "discover__knowledge__knowledge_read").needs,
            vec!["prepare_branch"]
        );
    }

    // Mirror of `triple_nested_composite_propagates_needs_to_the_real_entry`
    // for the TERMINAL / outbound side. A downstream `needs: [<composite>]`
    // is remapped to that composite's terminal stages — but when the child's
    // own sink stage is ITSELF composite, the raw declaration names a stage
    // that never survives expansion. `namespaced_terminals` has no branch for
    // `is_composite()` (only for `is_loop()`), so walking `child.stages`
    // emitted `<ns>__<raw sink name>` — a phantom. Walking the expanded
    // `child_flat` yields the real, doubly-namespaced sink instead.
    #[test]
    fn it_nested_composite_terminal_remaps_downstream_needs_to_the_real_sink() {
        let dir = tmp_dir("nestedterm");
        // 3 levels: outer -> mid (uses inner as its own sink) -> inner.
        std::fs::write(
            dir.join("inner.yaml"),
            "version: 5
id: inner
stages:
  - { name: i_first, agent: executor }
  - { name: i_last, agent: executor, needs: [i_first] }
",
        )
        .unwrap();
        std::fs::write(
            dir.join("mid.yaml"),
            "version: 5
id: mid
stages:
  - { name: m_first, agent: executor }
  - { name: m_sink, type: pipeline, uses: inner, needs: [m_first] }
",
        )
        .unwrap();
        std::fs::write(
            dir.join("outer.yaml"),
            "version: 5
id: outer
stages:
  - { name: c, type: pipeline, uses: mid }
  - { name: after, agent: executor, needs: [c] }
",
        )
        .unwrap();

        let pipe = pipeline::load(&dir.join("outer.yaml")).unwrap();
        let eff = expand(&pipe, &[dir.as_path()]).unwrap();

        // The real sink is doubly namespaced; `c__m_sink` never exists.
        let names: Vec<&str> = eff.stages.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"c__m_sink__i_last"),
            "expected the doubly-namespaced sink, got: {names:?}"
        );
        assert!(!names.contains(&"c__m_sink"), "got: {names:?}");

        assert_eq!(stage(&eff, "after").needs, vec!["c__m_sink__i_last"]);
        // And the whole thing must actually topo-sort.
        pipeline::topo_sort(&eff).expect("expanded pipeline must topo-sort");
    }

    // The 7 real templates that compose `dev-discovery` — whose own sink
    // stage `knowledge_sync` is composite (`uses: knowledge-update`, itself a
    // 2-stage pipeline sinking on `knowledge_feature_update`) — used to have
    // their downstream `needs: [discover]` remapped to the phantom
    // `discover__knowledge_sync`, so `topo_sort` failed and the downstream
    // stage silently lost its "## Upstream outputs" prompt section.
    #[test]
    fn it_templates_composing_dev_discovery_topo_sort() {
        let tpl = templates_dir();
        for id in [
            "dev",
            "dev-angular",
            "dev-csharp",
            "dev-csharp-cheat-stack",
            "dev-lambda-ts",
            "dev-lambda-tsgit",
            "test-csharp",
        ] {
            let pipe = pipeline::load(&tpl.join(format!("{id}.yaml"))).unwrap();
            let eff = expand(&pipe, &[tpl.as_path()]).unwrap();

            let names: Vec<&str> = eff.stages.iter().map(|s| s.name.as_str()).collect();
            assert!(
                names.contains(&"discover__knowledge_sync__knowledge_feature_update"),
                "{id}: expected the real dev-discovery sink, got: {names:?}"
            );
            assert!(
                !names.contains(&"discover__knowledge_sync"),
                "{id}: phantom composite stage survived expansion: {names:?}"
            );
            // Every `needs` entry must name a stage that actually exists.
            for st in &eff.stages {
                for n in &st.needs {
                    assert!(
                        names.contains(&n.as_str()),
                        "{id}: stage '{}' needs unknown stage '{n}'",
                        st.name
                    );
                }
            }
            pipeline::topo_sort(&eff)
                .unwrap_or_else(|e| panic!("{id}: topo_sort must succeed, got: {e}"));

            // The real symptom was not the topo error (run.rs degrades to an
            // empty plan on it) but the downstream stage silently losing its
            // "## Upstream outputs" prompt section: workflow_emit emits
            // `up({needs})` verbatim from this list, and `up()` no-ops on a
            // name absent from `results`. So assert the remap target itself,
            // not merely that the sort succeeds.
            assert_eq!(
                stage(&eff, "plan").needs,
                vec!["discover__knowledge_sync__knowledge_feature_update"],
                "{id}: stage 'plan' must depend on the real dev-discovery sink"
            );
        }
    }

    #[test]
    fn it_effective_pipeline_revalidates() {
        let tpl = templates_dir();
        let pipe = pipeline::load(&tpl.join("dev-composed.yaml")).unwrap();
        let eff = expand(&pipe, &[tpl.as_path()]).unwrap();

        let dir = tmp_dir("revalidate");
        let out = dir.join("effective.yaml");
        std::fs::write(&out, serde_yaml::to_string(&eff).unwrap()).unwrap();
        // load() runs schema validation; must pass with namespaced names.
        pipeline::load(&out).expect("effective pipeline must re-validate against schema");
    }

    #[test]
    fn it_cycle_is_detected() {
        let dir = tmp_dir("cycle");
        std::fs::write(
            dir.join("a.yaml"),
            "version: 4\nid: a\nstages:\n  - { name: s, type: pipeline, uses: b }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("b.yaml"),
            "version: 4\nid: b\nstages:\n  - { name: s, type: pipeline, uses: a }\n",
        )
        .unwrap();
        let pipe = pipeline::load(&dir.join("a.yaml")).unwrap();
        let err = expand(&pipe, &[dir.as_path()]).unwrap_err().to_string();
        assert!(err.contains("cycle"), "expected cycle error, got: {err}");
    }

    // WI-P3-2: the total flattened-stage count is bounded; a pipeline that
    // exceeds its `settings.max_total_stages` bails, while the default (256)
    // leaves normal pipelines untouched.
    // WI-P0-4: a reused child's `{{ settings.X }}` binds to the CHILD's value,
    // not the parent's, for its inlined stages.
    #[test]
    fn it_child_settings_bind_for_inlined_child() {
        let dir = tmp_dir("childsettings");
        std::fs::write(
            dir.join("child.yaml"),
            "version: 5\nid: childs\nsettings: { max_correction_iterations: 5 }\nstages:\n  - { name: work, agent: executor, inputs: { cap: \"{{ settings.max_correction_iterations }}\" } }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("parent.yaml"),
            "version: 5\nid: parents\nsettings: { max_correction_iterations: 2 }\nstages:\n  - { name: c, type: pipeline, uses: childs }\n",
        )
        .unwrap();
        let pipe = pipeline::load(&dir.join("parent.yaml")).unwrap();
        let eff = expand(&pipe, &[dir.as_path()]).unwrap();
        // The inlined child stage must see the CHILD's 5, not the parent's 2.
        assert_eq!(input_str(stage(&eff, "c__work"), "cap"), "5");
    }

    #[test]
    fn it_total_stage_budget_enforced() {
        let dir = tmp_dir("stagebudget");
        std::fs::write(
            dir.join("wide.yaml"),
            "version: 5\nid: wide\nsettings: { max_total_stages: 2 }\nstages:\n  - { name: a, agent: analyzer }\n  - { name: b, agent: analyzer }\n  - { name: c, agent: analyzer }\n",
        )
        .unwrap();
        let pipe = pipeline::load(&dir.join("wide.yaml")).unwrap();
        let err = expand(&pipe, &[dir.as_path()]).unwrap_err().to_string();
        assert!(err.contains("exceeding the limit of 2"), "got: {err}");

        std::fs::write(
            dir.join("ok.yaml"),
            "version: 5\nid: okwide\nstages:\n  - { name: a, agent: analyzer }\n  - { name: b, agent: analyzer }\n  - { name: c, agent: analyzer }\n",
        )
        .unwrap();
        let pipe2 = pipeline::load(&dir.join("ok.yaml")).unwrap();
        assert!(
            expand(&pipe2, &[dir.as_path()]).is_ok(),
            "default limit must not reject a small pipeline"
        );
    }

    // WI-P2-7: a `type: loop` nested in a loop body is rejected at compose time.
    #[test]
    fn it_nested_loop_in_loop_body_rejected() {
        let dir = tmp_dir("nestloop");
        std::fs::write(
            dir.join("p.yaml"),
            "version: 5\nid: nest\nstages:\n  - name: outer\n    type: loop\n    body:\n      - name: inner\n        type: loop\n        body:\n          - { name: step, agent: executor }\n",
        )
        .unwrap();
        let pipe = pipeline::load(&dir.join("p.yaml")).unwrap();
        let err = expand(&pipe, &[dir.as_path()]).unwrap_err().to_string();
        assert!(err.contains("nested inside a loop body"), "got: {err}");
    }

    // WI-P2-7: a composite passing an input the child does not declare is rejected.
    #[test]
    fn it_unknown_child_input_rejected() {
        let dir = tmp_dir("unknowninput");
        std::fs::write(
            dir.join("child.yaml"),
            "version: 5\nid: ch\ninputs:\n  known: { kind: string }\nstages:\n  - { name: work, agent: executor, inputs: { x: \"{{ inputs.known }}\" } }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("parent.yaml"),
            "version: 5\nid: par\nstages:\n  - { name: c, type: pipeline, uses: ch, with: { known: \"a\", bogus: \"b\" } }\n",
        )
        .unwrap();
        let pipe = pipeline::load(&dir.join("parent.yaml")).unwrap();
        let err = expand(&pipe, &[dir.as_path()]).unwrap_err().to_string();
        assert!(
            err.contains("not declared by the referenced pipeline"),
            "got: {err}"
        );
    }

    // LOC-51: a composite's `when:` must reach its non-entry stages too — a
    // skipped `needs` does not skip its successor, so without it the child's
    // synthesis stage ran on a skipped review. A stage's own `when:` is kept.
    #[test]
    fn it_composite_when_reaches_non_entry_stages() {
        let dir = tmp_dir("compwhen");
        std::fs::write(
            dir.join("child.yaml"),
            "version: 5
id: chw
stages:
  - { name: axis, agent: reviewer }
  - { name: synth, agent: reviewer, needs: [axis], when: \"{{ inputs.x == 'y' }}\" }
  - { name: tail, agent: reviewer, needs: [synth] }
",
        )
        .unwrap();
        std::fs::write(
            dir.join("parent.yaml"),
            "version: 5
id: parw
inputs:
  track: { kind: string, default: standard }
  x: { kind: string, default: y }
stages:
  - { name: c, type: pipeline, uses: chw, when: \"{{ inputs.track == 'deep' }}\" }
  - { name: after, agent: executor, needs: [c] }
",
        )
        .unwrap();
        let pipe = pipeline::load(&dir.join("parent.yaml")).unwrap();
        let eff = expand(&pipe, &[dir.as_path()]).unwrap();
        let cond = serde_yaml::Value::String("{{ inputs.track == 'deep' }}".into());
        for name in ["c__axis", "c__synth", "c__tail"] {
            assert_eq!(
                stage(&eff, name).composite_when,
                vec![cond.clone()],
                "{name}"
            );
        }
        assert!(stage(&eff, "c__synth").when.is_some());
        assert!(stage(&eff, "after").composite_when.is_empty());
    }

    // A dynamic fan-out's `from: {{ stages.<producer>.signals.<prop> }}` must be
    // namespaced when the child pipeline is inlined, else the emitter can't
    // resolve the producer stage.
    #[test]
    fn it_fanout_from_is_namespaced_on_inline() {
        let dir = tmp_dir("fanoutns");
        std::fs::write(
            dir.join("child.yaml"),
            "version: 5\nid: chfo\nstages:\n  - { name: plan, agent: planner }\n  - name: work\n    agent: executor\n    needs: [plan]\n    fan_out: { from: \"{{ stages.plan.signals.shards }}\", max: 4 }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("parent.yaml"),
            "version: 5\nid: parfo\nstages:\n  - { name: c, type: pipeline, uses: chfo }\n",
        )
        .unwrap();
        let pipe = pipeline::load(&dir.join("parent.yaml")).unwrap();
        let eff = expand(&pipe, &[dir.as_path()]).unwrap();
        let work = stage(&eff, "c__work");
        let from = work.fan_out.as_ref().unwrap().from.as_deref().unwrap();
        assert_eq!(from, "{{ stages.c__plan.signals.shards }}");
    }

    // A doubly-composite entry (an entry stage reached through two nested
    // `uses:` levels) is stamped by the OUTERMOST composite only — one
    // Workflow-pass boundary per top-level `uses:`, not one per nesting
    // level. The inner stamp (`m`) must be overwritten by the outer one
    // (`c`), never merged or left as a second, inner-scoped marker.
    #[test]
    fn it_nested_composite_entry_is_stamped_by_the_outermost_uses() {
        let dir = tmp_dir("nestedentry");
        std::fs::write(
            dir.join("inner.yaml"),
            "version: 5\nid: innerdbl\nstages:\n  - { name: x, agent: executor }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("mid.yaml"),
            "version: 5\nid: middbl\nstages:\n  - { name: m, type: pipeline, uses: innerdbl }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("parent.yaml"),
            "version: 5\nid: outerdbl\nstages:\n  - { name: c, type: pipeline, uses: middbl }\n",
        )
        .unwrap();
        let pipe = pipeline::load(&dir.join("parent.yaml")).unwrap();
        let eff = expand(&pipe, &[dir.as_path()]).unwrap();
        let entry = stage(&eff, "c__m__x");
        assert_eq!(
            entry.composite_entry.as_deref(),
            Some("c"),
            "the outermost composite (`c`) must own the pass boundary, not the inner `m`"
        );
    }

    // F-5: `merge_findings.from` names producer stages the "bare name" way
    // (like `needs`, not a handlebars span), so it must be namespaced the
    // same way `needs` already is on inline composition — mirror of
    // `fanout_from_is_namespaced_on_inline` for the other encoding.
    #[test]
    fn it_merge_findings_from_is_namespaced_on_inline() {
        let dir = tmp_dir("mergefindns");
        std::fs::write(
            dir.join("child.yaml"),
            "version: 5\nid: chmf\nstages:\n  - { name: a, agent: reviewer, outputs: [findings.json] }\n  - { name: b, agent: reviewer, outputs: [findings.json] }\n  - name: synth\n    agent: reviewer\n    needs: [a, b]\n    merge_findings: { from: [a, b], out: findings.json }\n    outputs: [findings.json]\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("parent.yaml"),
            "version: 5\nid: parmf\nstages:\n  - { name: c, type: pipeline, uses: chmf }\n",
        )
        .unwrap();
        let pipe = pipeline::load(&dir.join("parent.yaml")).unwrap();
        let eff = expand(&pipe, &[dir.as_path()]).unwrap();
        let synth = stage(&eff, "c__synth");
        let from = &synth.merge_findings.as_ref().unwrap().from;
        assert_eq!(from, &vec!["c__a".to_string(), "c__b".to_string()]);
    }

    #[test]
    fn it_missing_reference_errors() {
        let dir = tmp_dir("missing");
        std::fs::write(
            dir.join("p.yaml"),
            "version: 4\nid: p\nstages:\n  - { name: s, type: pipeline, uses: nope }\n",
        )
        .unwrap();
        let pipe = pipeline::load(&dir.join("p.yaml")).unwrap();
        let err = expand(&pipe, &[dir.as_path()]).unwrap_err().to_string();
        assert!(err.contains("unknown pipeline reference"), "got: {err}");
    }

    #[test]
    fn it_required_child_input_must_be_provided() {
        let dir = tmp_dir("reqinput");
        std::fs::write(
            dir.join("child.yaml"),
            "version: 4\nid: child\ninputs:\n  must: { required: true, kind: string }\nstages:\n  - { name: work, agent: executor, inputs: { x: \"{{ inputs.must }}\" } }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("parent.yaml"),
            "version: 4\nid: parent\nstages:\n  - { name: c, type: pipeline, uses: child }\n",
        )
        .unwrap();
        let pipe = pipeline::load(&dir.join("parent.yaml")).unwrap();
        let err = expand(&pipe, &[dir.as_path()]).unwrap_err().to_string();
        assert!(err.contains("required input 'must'"), "got: {err}");
    }

    // ======================================================================
    // Lot T ratchet: one cargo test per migration rule, locking down the
    // park's post-migration state so it cannot regress. Every test below
    // PARSES the YAML (via `pipeline::load`, same as the production code
    // path) and walks the resulting structure — never a raw line-oriented
    // grep of the file text. That distinction is load-bearing: a raw count
    // of the substring "model:" over these 39 templates finds 260 lines,
    // but only 259 are real pins (the 260th is prose inside a
    // `description:`), and 47 of the real pins live inside single-line
    // inline flow mappings (`{ name: x, model: sonnet }`) that a
    // line-oriented scan would misparse as one unit instead of two fields.
    // Measured on ae20d7a / 2026-09-11 — verify these counts on the tree
    // you are running against, do not force them.
    // ======================================================================

    /// Recursively collect every string scalar reachable from a `serde_yaml`
    /// value (sequence/mapping contents), preserving nothing about its
    /// position — callers only care about substring content.
    fn collect_yaml_strings<'a>(v: &'a serde_yaml::Value, out: &mut Vec<&'a str>) {
        match v {
            serde_yaml::Value::String(s) => out.push(s.as_str()),
            serde_yaml::Value::Sequence(seq) => {
                for item in seq {
                    collect_yaml_strings(item, out);
                }
            }
            serde_yaml::Value::Mapping(m) => {
                for val in m.values() {
                    collect_yaml_strings(val, out);
                }
            }
            _ => {}
        }
    }

    /// Every string in `pipe` that could plausibly embed a `{{ inputs.X }}`
    /// handlebars reference: pipeline/stage `description`/`policy`, a
    /// stage's `inputs:`/`with:` call arguments, `when:`/`gate:`/
    /// `max_iterations:`, its `scope:` path lists, and the fan-out /
    /// plan-tasks merge hints. Deliberately excludes `Pipeline.inputs`
    /// (`InputSpec.description` is schema documentation for a human filling
    /// in `--inputs`, never evaluated as an expression). Recurses into loop
    /// bodies via `pipeline::flat_stages`; a composite (`type: pipeline`)
    /// stage's own referenced pipeline is a separate file, walked on its own
    /// turn by the same loop in each caller.
    fn all_template_strings(pipe: &Pipeline) -> Vec<&str> {
        let mut out = vec![pipe.description.as_str()];
        out.extend(pipe.policy.iter().map(String::as_str));
        for st in pipeline::flat_stages(pipe) {
            out.push(st.description.as_str());
            out.extend(st.policy.iter().map(String::as_str));
            for v in st.inputs.values() {
                collect_yaml_strings(v, &mut out);
            }
            for v in st.with.values() {
                collect_yaml_strings(v, &mut out);
            }
            if let Some(w) = &st.when {
                collect_yaml_strings(w, &mut out);
            }
            if let Some(g) = &st.gate {
                collect_yaml_strings(g, &mut out);
            }
            if let Some(mi) = &st.max_iterations {
                collect_yaml_strings(mi, &mut out);
            }
            if let Some(sc) = &st.scope {
                out.extend(sc.allowed.iter().map(String::as_str));
                out.extend(sc.denied.iter().map(String::as_str));
                out.extend(sc.writes.iter().map(String::as_str));
                out.extend(sc.reads.iter().map(String::as_str));
            }
            if let Some(fo) = &st.fan_out {
                out.extend(fo.items.iter().map(String::as_str));
                if let Some(f) = &fo.from {
                    out.push(f.as_str());
                }
                if let Some(h) = &fo.merge_hint {
                    out.push(h.as_str());
                }
            }
            if let Some(pt) = &st.plan_tasks {
                out.push(pt.from.as_str());
                if let Some(h) = &pt.merge_hint {
                    out.push(h.as_str());
                }
            }
        }
        out
    }

    /// Every `inputs.<NAME>` identifier found strictly inside a `{{ ... }}`
    /// handlebars span of `text` — never a bare `inputs.X` substring, which
    /// can legitimately appear in prose (a description explaining "lands in
    /// inputs.target" is not itself a reference the schema must satisfy).
    /// The identifier is matched in full (`[A-Za-z0-9_]+`), so
    /// `inputs.target_path` is never mistaken for a truncated `inputs.target`.
    fn input_refs_in(text: &str) -> Vec<String> {
        use once_cell::sync::Lazy;
        static SPAN: Lazy<regex::Regex> =
            Lazy::new(|| regex::Regex::new(r"\{\{([^}]*)\}\}").unwrap());
        static NAME: Lazy<regex::Regex> =
            Lazy::new(|| regex::Regex::new(r"inputs\.([A-Za-z0-9_]+)").unwrap());
        let mut out = Vec::new();
        for span in SPAN.captures_iter(text) {
            for name in NAME.captures_iter(&span[1]) {
                out.push(name[1].to_string());
            }
        }
        out
    }

    // 1) R-*: the base sanity check every other test below depends on — every
    // template in the park must still load, expand (composites inlined) and
    // topo-sort. Catches a template that fails to even parse or compose (bad
    // YAML, a broken `uses:`, a cycle, an unresolved `needs:`) that a
    // narrower rule-specific test below would never touch. The fail-closed
    // count guards against the whole test silently vacuously passing if
    // `templates_dir()` ever pointed at an empty or missing directory.
    #[test]
    fn it_every_template_in_the_park_loads_and_expands() {
        let tpl = templates_dir();
        let roots = [tpl.as_path()];
        let discovered = pipeline::discover(&roots).expect("discover templates/");
        let n = discovered.len();
        assert!(
            n >= 39,
            "aucun template decouvert - la disposition du depot a change"
        );
        for (id, path) in discovered {
            let pipe = pipeline::load(&path)
                .unwrap_or_else(|e| panic!("load {id} ({}): {e}", path.display()));
            let eff = expand(&pipe, &roots).unwrap_or_else(|e| panic!("expand {id}: {e}"));
            // The `KNOWN_TERMINAL_REMAP_REGRESSION` named-exception list that
            // stood here (7 templates asserted to FAIL `topo_sort`, because a
            // composite terminal that was itself composite got remapped to a
            // name that never survives expansion) is gone: the bug it tracked
            // is fixed. `composite_terminals` is now built from the flattened
            // `child_flat` rather than the raw `child.stages`, so there is no
            // exception left — every template must topo-sort. The dedicated
            // regression tests are, below in this module,
            // `nested_composite_terminal_remaps_downstream_needs_to_the_real_sink`
            // and `templates_composing_dev_discovery_topo_sort`, plus
            // `workflow_emit::nested_composite_sink_is_emitted_as_a_resolvable_upstream`
            // for the emitted `up(...)` call site.
            pipeline::topo_sort(&eff).unwrap_or_else(|e| panic!("topo_sort {id}: {e}"));
        }
    }

    // 2) R-MOD-1: a `model:` pin that exactly repeats the emitter's own
    // per-role default (`AgentRole::default_model`) is prose masquerading
    // as configuration — `agent_opts` omits `model:` entirely when the field
    // is absent, at which point the harness applies that SAME default anyway,
    // so the pin changes nothing today. Left in place, a real future change
    // to `default_model()` would silently stop reaching every stage that
    // still carries one of these stale pins, while looking deliberately
    // configured. Measured 2026-09-11: 0 redundant pins remain across the
    // park after the Lot T mechanical migration (72 + 77 + 63 removed).
    #[test]
    fn it_no_template_pins_its_default_model() {
        let tpl = templates_dir();
        let roots = [tpl.as_path()];
        for (id, path) in pipeline::discover(&roots).expect("discover templates/") {
            let pipe = pipeline::load(&path).unwrap_or_else(|e| panic!("load {id}: {e}"));
            for st in pipeline::flat_stages(&pipe) {
                if st.is_composite() {
                    continue;
                }
                let agent = st.agent_role();
                assert!(
                    st.model != Some(agent.default_model()),
                    "template {id}, stage {}: model: {:?} exactly repeats the default model of {agent} - drop the redundant pin",
                    st.name,
                    st.model
                );
            }
        }
    }

    // 3) R-KIND-1: a template marked `kind: fragment` is a composition brick,
    // never meant to run standalone, so if nothing in the park actually
    // `uses:` it, it is a dead file nobody discovers by browsing
    // `templates/` (marking it `fragment` hides it from a human's mental map
    // of runnable pipelines, but grants no benefit if truly orphaned). This
    // is the test that would have caught the `dev-tests-csharp` orphan
    // before Lot T archived it (T8) instead of leaving it to a human's
    // periodic audit.
    #[test]
    fn it_every_fragment_is_referenced_by_some_uses() {
        let tpl = templates_dir();
        let roots = [tpl.as_path()];
        let discovered = pipeline::discover(&roots).expect("discover templates/");

        let mut fragments: Vec<String> = Vec::new();
        let mut used: HashSet<String> = HashSet::new();
        for (id, path) in &discovered {
            let pipe = pipeline::load(path).unwrap_or_else(|e| panic!("load {id}: {e}"));
            if pipe.kind == "fragment" {
                fragments.push(id.clone());
            }
            for st in pipeline::flat_stages(&pipe) {
                if let Some(u) = &st.uses {
                    used.insert(u.clone());
                    // `uses:` can also name a path (`dir/child.yaml`); a
                    // fragment referenced that way still counts, matched on
                    // the referencing file's basename.
                    if let Some(stem) = Path::new(u).file_stem().and_then(|s| s.to_str()) {
                        used.insert(stem.to_string());
                    }
                }
            }
        }

        for id in &fragments {
            assert!(
                used.contains(id),
                "template {id} is kind: fragment but no `uses:` anywhere in the park references it - orphaned fragment"
            );
        }
    }

    // 4) R-INP-1: a `{{ inputs.X }}` reference to an input the pipeline never
    // declares in its own `inputs:` block silently resolves to nothing at
    // render time (the literal `{{ inputs.X }}` text is left in the emitted
    // prompt instead of failing) — this is the class of bug where an input
    // gets renamed or removed but a stray reference to the old name survives
    // in some other stage. `target` and `jira_key` are exempt by name: the
    // CLI driver injects both unconditionally regardless of what a pipeline
    // declares (see `pipeline::validate_inputs`'s own doc comment), so
    // several templates read them without ever declaring them.
    #[test]
    fn it_every_input_reference_is_declared() {
        let tpl = templates_dir();
        let roots = [tpl.as_path()];
        for (id, path) in pipeline::discover(&roots).expect("discover templates/") {
            let pipe = pipeline::load(&path).unwrap_or_else(|e| panic!("load {id}: {e}"));
            for text in all_template_strings(&pipe) {
                for name in input_refs_in(text) {
                    if name == "target" || name == "jira_key" {
                        continue;
                    }
                    assert!(
                        pipe.inputs.contains_key(&name),
                        "template {id}: `{{{{ inputs.{name} }}}}` is referenced but not declared in `inputs:` (found in {text:?})"
                    );
                }
            }
        }
    }

    // 5) R-INP-2: `InputSpec::description` is a `String`, not an
    // `Option<String>` — the type lets it silently default to `""`, so an
    // empty description on a `required: true` input slips through schema
    // validation and stays invisible until a human reads the rendered prompt
    // and finds no explanation of what to actually pass. An optional input
    // is exempt: it can reasonably lean on its `default:` to communicate
    // intent instead.
    #[test]
    fn it_every_required_input_has_a_description() {
        let tpl = templates_dir();
        let roots = [tpl.as_path()];
        for (id, path) in pipeline::discover(&roots).expect("discover templates/") {
            let pipe = pipeline::load(&path).unwrap_or_else(|e| panic!("load {id}: {e}"));
            for (name, spec) in &pipe.inputs {
                if spec.required {
                    assert!(
                        !spec.description.trim().is_empty(),
                        "template {id}: required input '{name}' has no description"
                    );
                }
            }
        }
    }

    // 6) R-INP-3: a declared input nobody reads anywhere is either dead
    // weight (safe to delete) or wired through a channel this scan cannot
    // see — every survivor in today's park is the latter, so each gets a
    // NAMED, commented exception here instead of a silent pass that could
    // just as easily hide a real dead input tomorrow. `skills_by.input`
    // counts as a reference even though it is not a `{{ inputs.X }}`
    // handlebars span: `SkillsBy::resolve` reads that input's runtime value
    // directly, exactly like a handlebars ref would — without this,
    // `dev.yaml`'s `stack` input (read only via `skills_by: { input: stack,
    // ... }`) would wrongly read as unreferenced and fail this test.
    #[test]
    fn it_every_declared_input_is_referenced() {
        // `feature` feeds `run::feature_key` (git branch name AND vault
        // folder) entirely outside prompt rendering, so it is never a
        // `{{ inputs.feature }}` handlebars span in these 21 templates
        // specifically. `gha-fix` / `sonar-fix` / `pr-comments` set it from
        // the CI autopilot's dispatch plan (the PR's ticket key) rather than
        // from a `{{ feature }}` on the command line, but the same rule
        // applies: nothing in their stage bodies renders it. `func-doc.yaml`
        // also declares `feature` but DOES reference `{{ inputs.feature }}`
        // directly (twice) — it is therefore correctly absent from this
        // list; adding it back here would hide a real regression if that
        // reference were ever deleted.
        const FEATURE_EXEMPT: &[&str] = &[
            "cartography-cheat",
            "correction-after-test-dev",
            "dev",
            "dev-aidp",
            "dev-aidp-viewer",
            "dev-angular",
            "dev-composed",
            "dev-csharp",
            "dev-csharp-cheat-stack",
            "dev-csharp-short",
            "dev-flow",
            "dev-from-audit",
            "dev-from-audit-lambda-ts",
            "dev-lambda-ts",
            "dev-lambda-tsgit",
            "dev-model-mix",
            "cheat-local-dev",
            "gha-fix",
            "knowledge-init",
            "pr-comments",
            "sonar-fix",
            "test-csharp",
        ];

        let tpl = templates_dir();
        let roots = [tpl.as_path()];
        for (id, path) in pipeline::discover(&roots).expect("discover templates/") {
            let pipe = pipeline::load(&path).unwrap_or_else(|e| panic!("load {id}: {e}"));

            let mut referenced: HashSet<String> = HashSet::new();
            for text in all_template_strings(&pipe) {
                referenced.extend(input_refs_in(text));
            }
            for st in pipeline::flat_stages(&pipe) {
                if let Some(sb) = &st.skills_by {
                    referenced.insert(sb.input.clone());
                }
            }

            for name in pipe.inputs.keys() {
                if referenced.contains(name) {
                    continue;
                }
                let exempt = (name == "feature" && FEATURE_EXEMPT.contains(&id.as_str()))
                    // ticket_id: mapped by test-us-e2e.yaml's `with:` into its
                    // composed `design` stage (`uses: test-scenarios`) — not
                    // read inside test-scenarios.yaml itself, but
                    // `compose::expand` fails hard on a child input the
                    // parent maps that the child does not declare, so
                    // removing it here breaks composition even though this
                    // scan sees no reference.
                    || (name == "ticket_id" && id == "test-scenarios")
                    // pr: declared on these 3, but every stage in them reads
                    // `inputs.target` instead — a documented detour (see
                    // each file's own comment), not dead weight.
                    || (name == "pr" && matches!(id.as_str(), "gha-fix" | "pr-comments" | "sonar-fix"))
                    // prompt: cheat-local-dev declares it as an alternative to
                    // jira_keys (documented at the top of the file), used by the
                    // planner stage to frame the discovery, but not referenced in
                    // any stage input binding or handlebars template.
                    || (name == "prompt" && id == "cheat-local-dev");

                assert!(
                    exempt,
                    "template {id}: declared input '{name}' is never referenced (no handlebars span, no skills_by.input) and is not in a named exception list"
                );
            }
        }
    }

    // 7) R-POL-1: the ONE lexical/heuristic test in this ratchet — every
    // other test here parses structure, but "imperative/prohibitive doctrine
    // left in prose" has no structural shape to test for, so this greps on
    // purpose. Doctrine belongs in `policy:` (rendered under its own "##
    // Règles non négociables" heading by `prompt.rs`), never smuggled back
    // into `description:`, where nothing stops a future edit from drifting
    // it out of sync with the real rule. Scoped to `Pipeline.description`
    // and `Stage.description` only (recursing into loop bodies) —
    // `InputSpec.description` is schema documentation for a human filling in
    // `--inputs`, a different audience, and scanning it would catch
    // unrelated false positives (e.g. an enum input documenting its own
    // `always`/`never` VALUES in prose).
    #[test]
    fn it_no_doctrine_left_in_descriptions() {
        // Restrict the markers to words that are genuinely prohibitive
        // ("never"/"jamais"/"must not") — NOT "always"/"toujours", which
        // mostly narrate a factual constant (e.g. dev-aidp.yaml: "DEMAND is
        // always plain text") and would drag in false positives beyond the
        // named list below. If a future rewrite trips a NEW false positive,
        // the fix is to narrow this regex further, never to widen the
        // exception list to paper over it.
        let markers = regex::Regex::new(
            r"(?i)\b(never|jamais|must\s+not|must\s+never|ne\s+doit\s+jamais|ne\s+fait\s+jamais)\b",
        )
        .unwrap();

        // Named, commented exceptions: known false positives where the
        // matched word describes a mechanism or an architecture boundary,
        // not anti-gaming doctrine directed at a correction loop.
        const EXEMPT: &[&str] = &[
            // "Ce pipeline N'ÉCRIT JAMAIS dans le dépôt du service" states
            // what this pipeline touches (read-only cartography), not a
            // correction-loop rule.
            "cartography-cheat",
            // "which this pipeline reads but never edits" states the
            // viewer/daemon architecture boundary, not doctrine.
            "dev-aidp-viewer",
            // "the unused one is never dispatched" describes the dual-mode
            // `when:` compile-time skip mechanism, not a rule for the model.
            "dev-discovery",
            // "must not both touch" / "never production code" describe the
            // two-stage split's file ownership, not anti-gaming doctrine.
            "dev-tests-csharp-split",
        ];

        let tpl = templates_dir();
        let roots = [tpl.as_path()];
        for (id, path) in pipeline::discover(&roots).expect("discover templates/") {
            if EXEMPT.contains(&id.as_str()) {
                continue;
            }
            let pipe = pipeline::load(&path).unwrap_or_else(|e| panic!("load {id}: {e}"));
            assert!(
                !markers.is_match(&pipe.description),
                "template {id}: pipeline description still carries doctrine-sounding prose - move it to `policy:` ({:?})",
                pipe.description
            );
            for st in pipeline::flat_stages(&pipe) {
                assert!(
                    !markers.is_match(&st.description),
                    "template {id}, stage {}: stage description still carries doctrine-sounding prose - move it to `policy:` ({:?})",
                    st.name,
                    st.description
                );
            }
        }
    }

    // 8) R-ID-1: `uses: <name>` and `--pipeline <id>` both resolve a bare
    // name against either a filename or a discovered `id:` (see
    // `compose::resolve_ref` and `pipeline::discover`) — when a file's `id:`
    // disagrees with its own filename, `uses: <filename-without-extension>`
    // silently loads whatever OTHER pipeline happens to declare that id,
    // instead of failing loud. T8 fixed the one known mismatch
    // (`devPlan.yaml` -> `dev-model-mix.yaml`); this pins it down so it
    // cannot come back unnoticed.
    #[test]
    fn it_template_id_matches_its_filename() {
        let tpl = templates_dir();
        let roots = [tpl.as_path()];
        for (id, path) in pipeline::discover(&roots).expect("discover templates/") {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            assert_eq!(
                stem,
                id,
                "template at {}: id '{}' does not match its filename stem '{}'",
                path.display(),
                id,
                stem
            );
        }
    }

    // 9) A stray `.bak` / editor-swap file left in `templates/` by a bad
    // merge or an interrupted edit sits invisible forever: `pipeline::
    // discover` already filters to `.yaml`, so nothing else in the CLI path
    // would ever surface it. `templates/legacy/README.md` is the one
    // deliberate non-yaml file (see `pipeline::ARCHIVE_DIR`), covered by
    // allowing `.md` everywhere rather than special-casing that directory.
    #[test]
    fn it_no_backup_files_in_templates() {
        let tpl = templates_dir();
        for entry in walkdir::WalkDir::new(&tpl)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let ext = entry.path().extension().and_then(|e| e.to_str());
            assert!(
                matches!(ext, Some("yaml") | Some("md")),
                "unexpected non-yaml/md file under templates/: {}",
                entry.path().display()
            );
        }
    }

    // 10) `rolling_context_window` was removed as a typed `Settings` field
    // (see `pipeline::tests::removed_rolling_context_window_lands_in_extra_
    // not_a_typed_field`), but `Settings` carries `#[serde(flatten)] extra`,
    // so a template that still sets it — or misspells a real setting —
    // deserializes fine and is silently swallowed forever, never surfacing
    // as an error anywhere else. This is the only test that would ever
    // catch that.
    #[test]
    fn it_no_template_sets_a_dead_setting() {
        let tpl = templates_dir();
        let roots = [tpl.as_path()];
        for (id, path) in pipeline::discover(&roots).expect("discover templates/") {
            let pipe = pipeline::load(&path).unwrap_or_else(|e| panic!("load {id}: {e}"));
            assert!(
                !pipe.settings.extra.contains_key("rolling_context_window"),
                "template {id} still sets the removed setting `rolling_context_window:` (silently swallowed by Settings::extra)"
            );
        }
    }

    // 11) R-DUP-1: the decisive proof of the dev.yaml/dev-lambda-ts.yaml
    // consolidation — `implement`'s skill list now routes through
    // `skills_by` keyed on `stack` instead of dev.yaml and dev-lambda-ts.yaml
    // each hard-coding a near-duplicate list. Needs neither a compiled
    // binary nor a fixture: `SkillsBy::resolve` must reproduce
    // dev-lambda-ts.yaml's `implement.skills` EXACTLY (same content, same
    // ORDER — `prompt.rs` renders one bullet per skill in list order, so a
    // reordering is a different prompt). The day dev-lambda-ts.yaml's skill
    // list diverges from what this resolves to, this test goes red — that IS
    // the signal: either dev.yaml's `skills_by.map` is stale, or
    // dev-lambda-ts.yaml earned a genuine difference and should stop being
    // treated as a collapsed duplicate.
    #[test]
    fn it_collapsed_variants_reproduce_their_predecessor() {
        let tpl = templates_dir();
        let dev = pipeline::load(&tpl.join("dev.yaml")).unwrap();
        let dev_lambda_ts = pipeline::load(&tpl.join("dev-lambda-ts.yaml")).unwrap();

        let dev_implement = stage(&dev, "implement");
        let lambda_ts_implement = stage(&dev_lambda_ts, "implement");

        let skills_by = dev_implement
            .skills_by
            .as_ref()
            .expect("dev.yaml's implement stage must declare skills_by");

        let resolved_lambda_ts = skills_by.resolve(&dev_implement.skills, Some("lambda-ts"));
        assert_eq!(
            resolved_lambda_ts, lambda_ts_implement.skills,
            "dev.yaml's skills_by.resolve(_, \"lambda-ts\") must reproduce dev-lambda-ts.yaml's implement.skills exactly"
        );

        // `None` and the unmapped-but-named "generic" both fall back to the
        // SAME generic bucket — the multi-stack list dev.yaml hard-coded
        // before Lot T.
        let expected_generic = vec![
            "graphify-context".to_string(),
            "csharp-dev-aidp".to_string(),
            "lambda-ts-dev-aidp".to_string(),
            "feature-testing-aidp".to_string(),
            "reqnroll-testing-aidp".to_string(),
        ];
        assert_eq!(
            skills_by.resolve(&dev_implement.skills, Some("generic")),
            expected_generic
        );
        assert_eq!(
            skills_by.resolve(&dev_implement.skills, None),
            expected_generic
        );
    }
}
