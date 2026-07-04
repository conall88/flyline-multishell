mod common;

fn docker_available() -> bool {
    std::process::Command::new("docker")
        .args(["info"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn run_zsh_integration_test() {
    if !docker_available() {
        eprintln!("skipping zsh integration test: docker not available");
        return;
    }

    common::run_bake_target("zsh-integration-test").expect("zsh integration test failed");
    println!("Successfully tested zsh integration with flyline");
}

#[test]
fn zsh_integration_test() {
    run_zsh_integration_test();
}
