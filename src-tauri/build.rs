fn main() {
    // Never put credentials in cargo:rustc-env (or any build log).
    let key_path = std::path::Path::new("../.secrets/deepseek-api-key.txt");
    println!("cargo:rerun-if-changed={}", key_path.display());
    let raw = match std::fs::read_to_string(key_path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => panic!("Cannot read the private DeepSeek key file"),
    };
    let key = raw.trim_start_matches('\u{feff}').trim();
    assert!(
        key.len() <= 8192 && key.bytes().all(|b| b.is_ascii_graphic()),
        "DeepSeek key must be a single bare ASCII token (no Bearer prefix)"
    );
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(
        output.join("deepseek_key.rs"),
        format!("const API_KEY: &str = {key:?};\n"),
    )
    .expect("Cannot write the embedded DeepSeek key");
    tauri_build::build()
}
