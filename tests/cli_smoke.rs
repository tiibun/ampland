use std::fs;
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

#[test]
fn uninstall_missing_version_fails() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config = temp.path().join("config.toml");
    let cache = temp.path().join("cache");

    let status = Command::new(env!("CARGO_BIN_EXE_ampland"))
        .arg("--config")
        .arg(&config)
        .arg("--cache-dir")
        .arg(&cache)
        .arg("uninstall")
        .arg("node")
        .arg("22")
        .status()
        .expect("run ampland");

    assert!(!status.success());
}

#[test]
fn uninstall_in_use_version_fails() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_file = temp.path().join("config.toml");
    let cache = temp.path().join("cache");
    let cache_tool_dir = cache.join("node").join("22.0.0");

    // Create a config with node@22.0.0 configured globally
    fs::write(
        &config_file,
        "[global]\ntools = { node = \"22.0.0\" }\n",
    )
    .expect("write config");

    // Create fake tool version directory
    fs::create_dir_all(&cache_tool_dir).expect("create tool cache dir");
    fs::write(cache_tool_dir.join("node"), "fake binary").expect("create fake binary");

    // Try to uninstall the version that's in use
    let output = Command::new(env!("CARGO_BIN_EXE_ampland"))
        .arg("--config")
        .arg(&config_file)
        .arg("--cache-dir")
        .arg(&cache)
        .arg("uninstall")
        .arg("node")
        .arg("22.0.0")
        .output()
        .expect("run ampland");

    // Should fail because the tool is in use
    assert!(!output.status.success());
    // Error should mention that it's in use
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("in use") || stderr.contains("global"),
        "Error should mention tool is in use, got: {}",
        stderr
    );
}
