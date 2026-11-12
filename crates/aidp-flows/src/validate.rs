use crate::expr;
use crate::models::constants::resource;
use crate::models::enums::{DuplicateScope, StepKind};
use crate::models::{FlowDef, Step};
use std::collections::BTreeSet;

pub use crate::models::enums::IssueCode;
pub use crate::models::{Catalog, Issue, Report};

struct Validator<'a> {
    def: &'a FlowDef,
    catalog: &'a Catalog,
    report: Report,
    catalog_warning_given: bool,
}

/// Checks `def` against `catalog` and returns every error and warning.
pub fn validate(def: &FlowDef, catalog: &Catalog) -> Report {
    let mut validator = Validator {
        def,
        catalog,
        report: Report::default(),
        catalog_warning_given: false,
    };
    validator.run();
    validator.report
}

fn collect_ids(steps: &[Step], ids: &mut Vec<String>) {
    for step in steps {
        ids.push(step.id.clone());
        collect_ids(&step.body, ids);
    }
}

impl Validator<'_> {
    fn run(&mut self) {
        let def = self.def;
        let mut main_ids = Vec::new();
        collect_ids(&def.steps, &mut main_ids);
        self.report_duplicates(&main_ids, DuplicateScope::Flow);
        let main_known: BTreeSet<String> = main_ids.into_iter().collect();
        self.check_steps(&def.steps, &main_known);
        self.check_entries(&main_known);
        for (name, steps) in &def.blocks {
            let mut block_ids = Vec::new();
            collect_ids(steps, &mut block_ids);
            self.report_duplicates(&block_ids, DuplicateScope::Block(name));
            let mut known: BTreeSet<String> = block_ids.into_iter().collect();
            known.extend(main_known.iter().cloned());
            self.check_steps(steps, &known);
        }
    }

    fn check_entries(&mut self, known: &BTreeSet<String>) {
        let def = self.def;
        let top: Vec<&str> = def.steps.iter().map(|step| step.id.as_str()).collect();
        for (name, entry) in &def.entries {
            let Some(landing) = top.iter().position(|id| *id == entry.step) else {
                self.error(
                    IssueCode::BadEntry,
                    Some(&entry.step),
                    format!(
                        "entry `{name}` lands on `{}`, not a top-level step",
                        entry.step
                    ),
                );
                continue;
            };
            for kept in &entry.keep {
                if !top[..landing].contains(&kept.as_str()) {
                    self.error(
                        IssueCode::BadEntry,
                        Some(kept),
                        format!(
                            "entry `{name}` keeps `{kept}`, not a top-level step before `{}`",
                            entry.step
                        ),
                    );
                }
            }
            for assumed in entry.outputs.keys() {
                if !known.contains(assumed) {
                    self.error(
                        IssueCode::BadEntry,
                        Some(assumed),
                        format!("entry `{name}` assumes outputs of unknown step `{assumed}`"),
                    );
                }
            }
        }
    }

    fn error(&mut self, code: IssueCode, step: Option<&str>, message: String) {
        self.report.errors.push(Issue {
            code,
            step: step.map(str::to_string),
            message,
        });
    }

    fn report_duplicates(&mut self, ids: &[String], scope: DuplicateScope<'_>) {
        let mut seen = BTreeSet::new();
        let mut reported = BTreeSet::new();
        for id in ids {
            if !seen.insert(id.as_str()) && reported.insert(id.as_str()) {
                self.error(
                    IssueCode::DuplicateStepId,
                    Some(id),
                    format!("duplicate step id `{id}` in {}", scope.as_str()),
                );
            }
        }
    }

    fn check_steps(&mut self, steps: &[Step], known: &BTreeSet<String>) {
        for step in steps {
            self.check_condition(step, step.when.as_deref(), known);
            self.check_condition(step, step.until.as_deref(), known);
            self.check_kind(step);
            self.check_resource(step);
            self.check_steps(&step.body, known);
        }
    }

    fn check_condition(&mut self, step: &Step, source: Option<&str>, known: &BTreeSet<String>) {
        let Some(source) = source else { return };
        match expr::parse(source) {
            Err(error) => self.error(
                IssueCode::BadExpression,
                Some(&step.id),
                format!("step `{}`: {error}", step.id),
            ),
            Ok(parsed) => {
                for reference in parsed.step_refs() {
                    if !known.contains(&reference) {
                        self.error(
                            IssueCode::UnknownStepRef,
                            Some(&step.id),
                            format!("step `{}` references unknown step `{reference}`", step.id),
                        );
                    }
                }
            }
        }
    }

    fn check_kind(&mut self, step: &Step) {
        match step.kind {
            StepKind::Action => self.check_action(step),
            StepKind::Wait => self.check_wait(step),
            StepKind::Pipeline => self.check_pipeline(step),
            StepKind::Loop => self.check_loop(step),
            StepKind::Human => {}
        }
    }

    fn check_action(&mut self, step: &Step) {
        let Some(name) = step.use_.as_deref() else {
            return self.missing_use(step);
        };
        let known = self.catalog.actions.contains(name) || self.def.blocks.contains_key(name);
        if !known {
            self.unknown_use(step, name);
        }
    }

    fn check_wait(&mut self, step: &Step) {
        let Some(name) = step.use_.as_deref() else {
            return self.missing_use(step);
        };
        if !self.catalog.wait_probes.contains(name) {
            self.unknown_use(step, name);
        }
    }

    fn check_resource(&mut self, step: &Step) {
        let Some(resource) = step.resource.as_deref() else {
            return;
        };
        if step.kind != StepKind::Pipeline {
            self.error(
                IssueCode::BadResource,
                Some(&step.id),
                format!(
                    "step `{}`: `resource` only applies to pipeline steps",
                    step.id
                ),
            );
        } else if !resource::KNOWN.contains(&resource) {
            self.error(
                IssueCode::BadResource,
                Some(&step.id),
                format!(
                    "step `{}` names unknown resource `{resource}` (known: {})",
                    step.id,
                    resource::KNOWN.join(", ")
                ),
            );
        }
    }

    fn check_pipeline(&mut self, step: &Step) {
        let Some(template) = step.use_.as_deref() else {
            return self.missing_use(step);
        };
        let catalog = self.catalog;
        match &catalog.pipelines {
            Some(known) if !known.contains(template) => self.error(
                IssueCode::UnknownPipelineTemplate,
                Some(&step.id),
                format!(
                    "step `{}` uses unknown pipeline template `{template}`",
                    step.id
                ),
            ),
            Some(_) => {}
            None => self.warn_catalog_unreadable(),
        }
    }

    fn check_loop(&mut self, step: &Step) {
        let has_human = step.body.iter().any(Step::contains_human);
        if step.max.is_none() && !has_human {
            self.error(
                IssueCode::LoopUnbounded,
                Some(&step.id),
                format!(
                    "loop `{}` has no `max` and no human step in its body",
                    step.id
                ),
            );
        }
    }

    fn missing_use(&mut self, step: &Step) {
        self.error(
            IssueCode::MissingUse,
            Some(&step.id),
            format!("step `{}` has no `use`", step.id),
        );
    }

    fn unknown_use(&mut self, step: &Step, name: &str) {
        self.error(
            IssueCode::UnknownUse,
            Some(&step.id),
            format!("step `{}` uses unknown `{name}`", step.id),
        );
    }

    fn warn_catalog_unreadable(&mut self) {
        if self.catalog_warning_given {
            return;
        }
        self.catalog_warning_given = true;
        self.report.warnings.push(Issue {
            code: IssueCode::CatalogUnreadable,
            step: None,
            message: "pipeline template catalog unreadable: templates not checked".to_string(),
        });
    }
}

#[cfg(test)]
#[path = "../tests/unit/validate_tests.rs"]
mod tests;
