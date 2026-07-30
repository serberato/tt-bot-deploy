use crate::config::AdminMode;
use super::validation::{parse_admin_mode_input, parse_lang_code, parse_yes_no_input};

#[test]
fn parse_yes_no_input_matches_yes_variants() {
    assert!(parse_yes_no_input("y", false));
    assert!(parse_yes_no_input("Y", false));
    assert!(parse_yes_no_input("yes", false));
    assert!(parse_yes_no_input("YES", false));
}

#[test]
fn parse_yes_no_input_defaults_on_empty() {
    assert!(parse_yes_no_input("", true));
    assert!(!parse_yes_no_input("", false));
    assert!(!parse_yes_no_input("   ", false));
}

#[test]
fn parse_yes_no_input_rejects_no_variants() {
    assert!(!parse_yes_no_input("n", true));
    assert!(!parse_yes_no_input("NO", true));
    assert!(!parse_yes_no_input("other", true));
}

#[test]
fn parse_admin_mode_input_matches_modes() {
    assert_eq!(parse_admin_mode_input("everyone"), AdminMode::Everyone);
    assert_eq!(parse_admin_mode_input("ttrights"), AdminMode::TtRights);
    assert_eq!(parse_admin_mode_input("list"), AdminMode::List);
    assert_eq!(parse_admin_mode_input("both"), AdminMode::Both);
    assert_eq!(parse_admin_mode_input("unknown"), AdminMode::Both);
}

#[test]
fn parse_lang_code_defaults_to_en() {
    assert_eq!(parse_lang_code(""), "en");
    assert_eq!(parse_lang_code("   "), "en");
    assert_eq!(parse_lang_code("ES"), "es");
    assert_eq!(parse_lang_code("pt-BR"), "pt-br");
}
