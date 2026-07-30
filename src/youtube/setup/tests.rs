use std::path::{Path, PathBuf};

use super::*;
use super::migration::tool_item_names;
use super::paths::{pick_tools_dir, BGUTIL_VERSION_FILE};
use super::verify::{parse_sums_file, sha256_hex, verify_sha256};

fn mig_tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ttspotify_toolmig_{}_{}",
        name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fake_legacy_install(legacy: &Path) {
    std::fs::create_dir_all(legacy).unwrap();
    for name in tool_item_names() {
        if name == "yt-dlp-plugins" {
            let plug = legacy.join(name).join("bgutil_ytdlp_pot_provider");
            std::fs::create_dir_all(&plug).unwrap();
            std::fs::write(plug.join("plugin.py"), "py").unwrap();
        } else {
            std::fs::write(legacy.join(name), name).unwrap();
        }
    }
}

#[test]
fn migrates_marked_lib_and_removes_empty_legacy() {
    let base = mig_tmp("full");
    let legacy = base.join("lib");
    fake_legacy_install(&legacy);
    let target = base.join("data").join("ttspotify").join("lib");

    assert!(migrate_tools_dir(&legacy, &target));
    for name in tool_item_names() {
        assert!(target.join(name).exists(), "missing {name} in target");
        assert!(!legacy.join(name).exists(), "{name} left in legacy");
    }
    assert!(target
        .join("yt-dlp-plugins")
        .join("bgutil_ytdlp_pot_provider")
        .join("plugin.py")
        .is_file());
    assert!(!legacy.exists());

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn refuses_lib_without_our_marker() {
    let base = mig_tmp("unmarked");
    let legacy = base.join("lib");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("yt-dlp"), "x").unwrap();
    let target = base.join("data").join("lib");

    assert!(!migrate_tools_dir(&legacy, &target));
    assert!(legacy.join("yt-dlp").is_file());
    assert!(!target.exists());

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn keeps_legacy_dir_when_it_holds_foreign_files() {
    let base = mig_tmp("foreign");
    let legacy = base.join("lib");
    fake_legacy_install(&legacy);
    std::fs::write(legacy.join("users-own-notes.txt"), "keep me").unwrap();
    let target = base.join("data").join("lib");

    assert!(migrate_tools_dir(&legacy, &target));
    assert!(legacy.join("users-own-notes.txt").is_file());
    assert!(!legacy.join(BGUTIL_VERSION_FILE).exists());
    assert!(target.join(BGUTIL_VERSION_FILE).is_file());

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn existing_exe_side_lib_with_tools_wins() {
    let legacy = PathBuf::from("/opt/bot/lib");
    let picked = pick_tools_dir(
        legacy.clone(),
        true,
        Some(PathBuf::from("/home/u/.local/share")),
    );
    assert_eq!(picked, legacy);
}

#[test]
fn fresh_install_uses_xdg_data_dir() {
    let picked = pick_tools_dir(
        PathBuf::from("/usr/local/bin/lib"),
        false,
        Some(PathBuf::from("/home/u/.local/share")),
    );
    assert_eq!(
        picked,
        PathBuf::from("/home/u/.local/share/ttspotify/lib")
    );
}

#[test]
fn missing_data_dir_falls_back_to_exe_side_lib() {
    let legacy = PathBuf::from("/opt/bot/lib");
    assert_eq!(pick_tools_dir(legacy.clone(), false, None), legacy);
}

#[test]
fn resolve_paths_lands_in_lib_subdir() {
    let paths = resolve_paths().expect("resolve_paths");
    assert!(paths.lib_dir.ends_with("lib"));
    assert!(paths.yt_dlp.starts_with(&paths.lib_dir));
    assert!(paths.bgutil_pot.starts_with(&paths.lib_dir));
    assert!(paths.plugin_dir.starts_with(&paths.lib_dir));
}

#[test]
fn yt_dlp_filename_matches_platform() {
    let paths = resolve_paths().unwrap();
    let name = paths.yt_dlp.file_name().unwrap().to_str().unwrap();
    if cfg!(windows) {
        assert_eq!(name, "yt-dlp.exe");
    } else {
        assert_eq!(name, "yt-dlp");
    }
}

#[test]
fn default_cookies_path_ends_in_cookies_txt() {
    let p = default_cookies_path("");
    assert_eq!(p.file_name().and_then(|s| s.to_str()), Some("cookies.txt"));
    let p2 = default_cookies_path("cesar");
    assert_eq!(
        p2.file_name().and_then(|s| s.to_str()),
        Some("cookies_cesar.txt")
    );
}

#[test]
fn sha256_of_known_input() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn verify_sha256_matches_case_insensitively() {
    let h = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD";
    assert!(verify_sha256(b"abc", h));
    assert!(!verify_sha256(b"abd", h));
}

#[test]
fn parse_sums_file_finds_asset() {
    let text = "\
aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111  yt-dlp.exe
bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222 *yt-dlp_linux
short  ignored.bin";
    assert_eq!(
        parse_sums_file(text, "yt-dlp.exe").as_deref(),
        Some("aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111")
    );
    assert_eq!(
        parse_sums_file(text, "yt-dlp_linux").as_deref(),
        Some("bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222")
    );
    assert_eq!(parse_sums_file(text, "nope.exe"), None);
    assert_eq!(parse_sums_file(text, "ignored.bin"), None);
}
