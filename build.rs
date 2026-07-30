fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::path::Path::new("src/config.rs").exists() {
        let _ = std::fs::remove_file("src/config.rs");
    }
}
