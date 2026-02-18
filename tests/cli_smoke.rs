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
    fs::write(&config_file, "[global]\ntools = { node = \"22.0.0\" }\n").expect("write config");

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

#[test]
fn config_show_nonexistent_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config = temp.path().join("config.toml");
    let cache = temp.path().join("cache");

    let output = Command::new(env!("CARGO_BIN_EXE_ampland"))
        .arg("--config")
        .arg(&config)
        .arg("--cache-dir")
        .arg(&cache)
        .arg("config")
        .arg("show")
        .output()
        .expect("run ampland");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("does not exist"));
}

#[test]
fn config_show_existing_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_file = temp.path().join("config.toml");
    let cache = temp.path().join("cache");

    // Create a config file
    fs::write(&config_file, "[global.tools]\nnode = \"22.0.0\"\n").expect("write config");

    let output = Command::new(env!("CARGO_BIN_EXE_ampland"))
        .arg("--config")
        .arg(&config_file)
        .arg("--cache-dir")
        .arg(&cache)
        .arg("config")
        .arg("show")
        .output()
        .expect("run ampland");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[global.tools]"));
    assert!(stdout.contains("node = \"22.0.0\""));
}

#[test]
fn config_show_json_format() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_file = temp.path().join("config.toml");
    let cache = temp.path().join("cache");

    // Create a config file
    fs::write(&config_file, "[global.tools]\nnode = \"22.0.0\"\n").expect("write config");

    let output = Command::new(env!("CARGO_BIN_EXE_ampland"))
        .arg("--config")
        .arg(&config_file)
        .arg("--cache-dir")
        .arg(&cache)
        .arg("--json")
        .arg("config")
        .arg("show")
        .output()
        .expect("run ampland");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"path\""));
    assert!(stdout.contains("\"contents\""));

    // Parse as JSON to ensure it's valid
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("parse JSON");
    assert!(json.get("path").is_some());
    assert!(json.get("contents").is_some());
}

#[test]
fn config_edit_creates_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_file = temp.path().join("config.toml");
    let cache = temp.path().join("cache");

    // Use `true` as editor which just exits successfully
    let output = Command::new(env!("CARGO_BIN_EXE_ampland"))
        .env("EDITOR", "true")
        .arg("--config")
        .arg(&config_file)
        .arg("--cache-dir")
        .arg(&cache)
        .arg("config")
        .arg("edit")
        .output()
        .expect("run ampland");

    assert!(output.status.success());
    assert!(config_file.exists());
}
