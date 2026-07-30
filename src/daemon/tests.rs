use super::ipc::systemd_escape_instance;
use super::pid::parse_running_units;
use super::service::{unit_file_contents, unit_version_from_contents, UNIT_FILE_VERSION};

#[test]
fn unit_file_does_not_restart_on_config_error() {
    let unit = unit_file_contents(
        "\"/opt/bot\" --config \"/home/u/.config/ttspotify/%i.json\"",
        std::path::Path::new("/home/u/.config/ttspotify"),
        Some(std::path::Path::new("/home/u/.local/share/ttspotify/lib")),
    );
    assert!(unit.contains(&format!(
        "RestartPreventExitStatus={}",
        crate::config::EXIT_CONFIG_ERROR
    )));
    assert!(!unit.contains("RestartForceExitStatus"));
    assert!(unit.contains("Restart=on-failure"));
    assert!(unit.contains("ExecStart=\"/opt/bot\""));
}

#[test]
fn unit_file_sandboxes_with_writable_bot_dirs() {
    let unit = unit_file_contents(
        "\"/opt/bot\" --config \"/home/u/.config/ttspotify/%i.json\"",
        std::path::Path::new("/home/u/.config/ttspotify"),
        Some(std::path::Path::new("/home/u/.local/share/ttspotify/lib")),
    );
    assert!(unit.contains("ProtectSystem=strict"));
    assert!(unit.contains("PrivateTmp=true"));
    assert!(unit.contains("NoNewPrivileges=true"));
    assert!(unit.contains("ReadWritePaths=-/home/u/.config/ttspotify"));
    assert!(unit.contains("ReadWritePaths=-/home/u/.local/share/ttspotify/lib"));
    assert!(unit.contains("ReadWritePaths=-%h/.cache"));
    assert!(unit.contains("WorkingDirectory=/home/u/.config/ttspotify"));
}

#[test]
fn unit_file_carries_current_version_stamp() {
    let unit = unit_file_contents(
        "\"/opt/bot\" --config \"/x/%i.json\"",
        std::path::Path::new("/x"),
        None,
    );
    assert_eq!(unit_version_from_contents(&unit), UNIT_FILE_VERSION);
}

#[test]
fn unit_version_parses_stamp_and_defaults_to_zero() {
    assert_eq!(
        unit_version_from_contents("[Unit]\n# ttspotify-unit-version: 7\n[Service]\n"),
        7
    );
    assert_eq!(unit_version_from_contents("[Unit]\nExecStart=x\n"), 0);
    assert_eq!(unit_version_from_contents("# ttspotify-unit-version: banana\n"), 0);
}

#[test]
fn unit_file_without_tools_dir_omits_its_rw_line() {
    let unit = unit_file_contents(
        "\"/opt/bot\" --config \"/x/%i.json\"",
        std::path::Path::new("/x"),
        None,
    );
    assert!(unit.contains("ReadWritePaths=-/x"));
    assert!(unit.contains("ReadWritePaths=-%h/.local/share/ttspotify"));
    assert!(!unit.contains("ReadWritePaths=-/home"));
}

#[test]
fn escape_instance_passes_plain_names_through() {
    assert_eq!(systemd_escape_instance("myserver"), "myserver");
    assert_eq!(systemd_escape_instance("srv_2.home:x"), "srv_2.home:x");
}

#[test]
fn escape_instance_encodes_specials_like_systemd_escape() {
    assert_eq!(systemd_escape_instance("my server"), r"my\x20server");
    assert_eq!(systemd_escape_instance("a/b"), "a-b");
    assert_eq!(systemd_escape_instance(".hidden"), r"\x2ehidden");
}

#[test]
fn parses_unit_names_from_first_column() {
    let out = "ttspotify@home.service loaded active running TTSpotify bot (home)\n\
               ttspotify@work.service loaded active running TTSpotify bot (work)\n";
    assert_eq!(
        parse_running_units(out),
        vec!["ttspotify@home.service", "ttspotify@work.service"]
    );
}

#[test]
fn ignores_foreign_units_and_blank_lines() {
    let out = "\nother@x.service loaded active running Something else\n\
               ttspotify@home.service loaded active running TTSpotify bot\n\n";
    assert_eq!(parse_running_units(out), vec!["ttspotify@home.service"]);
}

#[test]
fn empty_output_is_empty() {
    assert!(parse_running_units("").is_empty());
}
