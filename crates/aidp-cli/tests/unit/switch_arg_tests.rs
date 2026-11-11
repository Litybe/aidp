use super::*;
use clap::ValueEnum;

#[test]
fn on_and_off_parse_and_report_their_value() {
    assert_eq!(SwitchArg::from_str("on", false), Ok(SwitchArg::On));
    assert_eq!(SwitchArg::from_str("off", false), Ok(SwitchArg::Off));
    assert!(SwitchArg::On.enabled());
    assert!(!SwitchArg::Off.enabled());
}

#[test]
fn anything_else_is_refused() {
    assert!(SwitchArg::from_str("maybe", false).is_err());
}
