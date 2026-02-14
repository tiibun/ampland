use std::process::Command;

#[test]
fn list_command_runs_successfully() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config = temp.path().join("config.toml");
    let cache = temp.path().join("cache");

    let status = Command::new(env!("CARGO_BIN_EXE_ampland"))
        .arg("--config")
        .arg(&config)
        .arg("--cache-dir")
        .arg(&cache)
        .arg("--quiet")
        .arg("list")
        .status()
        .expect("run ampland");

    assert!(status.success());
}
