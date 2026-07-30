fn main() {
    for old_file in &[
        "src/config.rs",
        "src/config/model.rs",
        "src/config/storage.rs",
        "src/services.rs",
        "src/i18n.rs",
        "src/bot/state.rs",
        "src/bot/controller.rs",
        "src/bot/commands.rs",
        "src/bot/runner.rs",
    ] {
        if std::path::Path::new(old_file).exists() {
            let _ = std::fs::remove_file(old_file);
        }
    }
}
