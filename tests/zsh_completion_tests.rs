use std::process::Command;

fn manifest_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Parser unit tests live in `shell::zsh` (pure Rust). Run them here so
/// `cargo test --test zsh_completion_tests` covers the parser without linking
/// the full flyline rlib (cdylib + standalone stubs overflow at link/init).
#[test]
fn shell_zsh_parser_unit_tests() {
    let status = Command::new("cargo")
        .current_dir(manifest_dir())
        .args(["test", "--lib", "parse_capture", "--"])
        .status()
        .expect("spawn cargo test for shell::zsh parser");
    assert!(
        status.success(),
        "shell::zsh parser unit tests failed (run: cargo test --lib parse_capture)"
    );
}
