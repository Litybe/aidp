use super::{classify_core, ExitStatus, TypedExit};

#[test]
fn classify_core_maps_not_found() {
    let error = anyhow::Error::new(crate::Error::NotFound("missing".into()));
    assert_eq!(classify_core(&error), Some(ExitStatus::NotFound));
}

#[test]
fn classify_core_maps_scope_violation_to_policy_refusal() {
    let error = anyhow::Error::new(crate::Error::ScopeViolation("blocked".into()));
    assert_eq!(classify_core(&error), Some(ExitStatus::PolicyRefusal));
}

#[test]
fn classify_core_maps_typed_exit() {
    let error = anyhow::Error::new(TypedExit {
        status: ExitStatus::NegativeVerdict,
        message: "gate failed".into(),
    });
    assert_eq!(classify_core(&error), Some(ExitStatus::NegativeVerdict));
}

#[test]
fn classify_core_returns_none_for_plain_anyhow() {
    let error = anyhow::anyhow!("x");
    assert_eq!(classify_core(&error), None);
}

#[test]
fn classify_core_finds_typed_cause_behind_context() {
    let base = anyhow::Error::new(crate::Error::NotFound("missing".into()));
    let wrapped = base.context("outer context");
    assert_eq!(classify_core(&wrapped), Some(ExitStatus::NotFound));
}
