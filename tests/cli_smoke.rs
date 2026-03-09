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
fn activate_outputs_fish_syntax() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config = temp.path().join("config.toml");
    let cache = temp.path().join("cache");
    let shims = temp.path().join("shim dir $PATH");
    fs::create_dir_all(&shims).expect("create shims dir");

    let output = Command::new(env!("CARGO_BIN_EXE_ampland"))
        .env("SHELL", "/usr/bin/fish")
        .arg("--config")
        .arg(&config)
        .arg("--cache-dir")
        .arg(&cache)
        .arg("--shims-dir")
        .arg(&shims)
        .arg("activate")
        .output()
        .expect("run ampland");

    assert!(output.status.success());

    let escaped_shims = shims
        .display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`");

    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("set -gx PATH \"{}\" $PATH\n", escaped_shims)
    );
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
fn uninstall_in_use_version_succeeds_for_current_scope() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_file = temp.path().join("config.toml");
    let cache = temp.path().join("cache");
    let shims = temp.path().join("shims");
    let cache_tool_dir = cache.join("node").join("22.0.0");
    let project_path = temp.path().join("workspace").join("project");

    fs::create_dir_all(&project_path).expect("create project dir");
    fs::write(
        &config_file,
        format!(
            "[global.tools]\n\n[[scope]]\npath = \"{}/**\"\n[scope.tools]\nnode = \"22.0.0\"\n",
            temp.path().join("workspace").display()
        ),
    )
    .expect("write config");

    fs::create_dir_all(&cache_tool_dir).expect("create tool cache dir");
    fs::write(cache_tool_dir.join("node"), "fake binary").expect("create fake binary");

    let output = Command::new(env!("CARGO_BIN_EXE_ampland"))
        .arg("--config")
        .arg(&config_file)
        .arg("--cache-dir")
        .arg(&cache)
        .arg("--shims-dir")
        .arg(&shims)
        .arg("--path")
        .arg(&project_path)
        .arg("uninstall")
        .arg("node")
        .arg("22.0.0")
        .output()
        .expect("run ampland");

    assert!(
        output.status.success(),
        "uninstall should succeed for current scope: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let updated = fs::read_to_string(&config_file).expect("read config");
    assert!(!updated.contains("node = \"22.0.0\""));
}

#[test]
fn uninstall_keeps_cache_when_other_scope_still_uses_version() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config_file = temp.path().join("config.toml");
    let cache = temp.path().join("cache");
    let shims = temp.path().join("shims");
    let cache_tool_dir = cache.join("node").join("22.0.0");
    let workspace = temp.path().join("workspace");
    let project_a = workspace.join("a");
    let project_b = workspace.join("b");

    fs::create_dir_all(&project_a).expect("create project a");
    fs::create_dir_all(&project_b).expect("create project b");
    fs::write(
        &config_file,
        format!(
            "[global.tools]\n\n[[scope]]\npath = \"{}/a/**\"\n[scope.tools]\nnode = \"22.0.0\"\n\n[[scope]]\npath = \"{}/b/**\"\n[scope.tools]\nnode = \"22.0.0\"\n",
            workspace.display(),
            workspace.display()
        ),
    )
    .expect("write config");

    fs::create_dir_all(&cache_tool_dir).expect("create cache version dir");
    fs::write(cache_tool_dir.join("node"), "fake binary").expect("create fake binary");

    let output = Command::new(env!("CARGO_BIN_EXE_ampland"))
        .arg("--config")
        .arg(&config_file)
        .arg("--cache-dir")
        .arg(&cache)
        .arg("--shims-dir")
        .arg(&shims)
        .arg("--path")
        .arg(&project_a)
        .arg("uninstall")
        .arg("node")
        .arg("22.0.0")
        .output()
        .expect("run ampland");

    assert!(
        output.status.success(),
        "uninstall should succeed without deleting cache: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(cache_tool_dir.exists(), "cache should be kept");
    let updated = fs::read_to_string(&config_file).expect("read config");
    assert!(!updated.contains(&format!("{}/a/**", workspace.display())));
    assert!(updated.contains(&format!("{}/b/**", workspace.display())));
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

#[test]
fn doctor_uses_overridden_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config = temp.path().join("config.toml");
    let cache = temp.path().join("custom-cache");
    let shims = temp.path().join("custom-shims");

    let output = Command::new(env!("CARGO_BIN_EXE_ampland"))
        .arg("--config")
        .arg(&config)
        .arg("--cache-dir")
        .arg(&cache)
        .arg("--shims-dir")
        .arg(&shims)
        .arg("--json")
        .arg("doctor")
        .output()
        .expect("run ampland");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("parse json");
    assert_eq!(
        json.get("config_path").and_then(|v| v.as_str()),
        config.to_str()
    );
    assert_eq!(
        json.get("cache_root").and_then(|v| v.as_str()),
        cache.to_str()
    );
    assert_eq!(
        json.get("shims_root").and_then(|v| v.as_str()),
        shims.to_str()
    );
}

#[test]
fn use_without_args_missing_tool_versions_fails() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config = temp.path().join("config.toml");
    let cache = temp.path().join("cache");
    let work_dir = temp.path().join("project");
    fs::create_dir_all(&work_dir).expect("create work dir");

    let output = Command::new(env!("CARGO_BIN_EXE_ampland"))
        .current_dir(&work_dir)
        .arg("--config")
        .arg(&config)
        .arg("--cache-dir")
        .arg(&cache)
        .arg("use")
        .output()
        .expect("run ampland");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(".tool-versions")
            && (stderr.contains("not found") || stderr.contains("found at")),
        "Error should mention missing .tool-versions file, got: {}",
        stderr
    );
}

#[test]
fn use_without_args_reads_tool_versions() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config = temp.path().join("config.toml");
    let cache = temp.path().join("cache");
    let work_dir = temp.path().join("project");
    fs::create_dir_all(&work_dir).expect("create work dir");

    // Create a .tool-versions file with fake tool versions
    let tool_versions = work_dir.join(".tool-versions");
    fs::write(&tool_versions, "node 20.10.0\npython 3.11.5\n").expect("write .tool-versions");

    let output = Command::new(env!("CARGO_BIN_EXE_ampland"))
        .current_dir(&work_dir)
        .arg("--config")
        .arg(&config)
        .arg("--cache-dir")
        .arg(&cache)
        .arg("use")
        .output()
        .expect("run ampland");

    // This will fail because the tools aren't in the manifest,
    // but we're testing that it reads the file
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should fail trying to resolve the tools from manifest, not complaining about missing file
    assert!(
        !stderr.contains(".tool-versions") || !stderr.contains("not found"),
        "Should not complain about missing .tool-versions, got: {}",
        stderr
    );
}

#[test]
fn use_without_args_with_comments_and_empty_lines() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config = temp.path().join("config.toml");
    let cache = temp.path().join("cache");
    let work_dir = temp.path().join("project");
    fs::create_dir_all(&work_dir).expect("create work dir");

    // Create a .tool-versions file with comments and empty lines
    let tool_versions = work_dir.join(".tool-versions");
    fs::write(
        &tool_versions,
        "# This is a comment\nnode 20.10.0\n\n# Another tool\npython 3.11.5\n",
    )
    .expect("write .tool-versions");

    let output = Command::new(env!("CARGO_BIN_EXE_ampland"))
        .current_dir(&work_dir)
        .arg("--config")
        .arg(&config)
        .arg("--cache-dir")
        .arg(&cache)
        .arg("use")
        .output()
        .expect("run ampland");

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should fail trying to resolve the tools, not parsing the file
    assert!(
        !stderr.contains(".tool-versions") || !stderr.contains("not found"),
        "Should parse file correctly, got: {}",
        stderr
    );
}

#[test]
fn use_without_args_invalid_format_fails() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config = temp.path().join("config.toml");
    let cache = temp.path().join("cache");
    let work_dir = temp.path().join("project");
    fs::create_dir_all(&work_dir).expect("create work dir");

    // Create a .tool-versions file with missing version
    let tool_versions = work_dir.join(".tool-versions");
    fs::write(&tool_versions, "node\n").expect("write .tool-versions");

    let output = Command::new(env!("CARGO_BIN_EXE_ampland"))
        .current_dir(&work_dir)
        .arg("--config")
        .arg(&config)
        .arg("--cache-dir")
        .arg(&cache)
        .arg("use")
        .output()
        .expect("run ampland");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("missing version") && stderr.contains("line 1"),
        "Error should mention missing version at line 1, got: {}",
        stderr
    );
}

#[test]
fn use_with_args_still_works() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config = temp.path().join("config.toml");
    let cache = temp.path().join("cache");
    let work_dir = temp.path().join("project");
    fs::create_dir_all(&work_dir).expect("create work dir");

    // Create a .tool-versions file, but pass explicit args
    let tool_versions = work_dir.join(".tool-versions");
    fs::write(&tool_versions, "python 3.11.5\n").expect("write .tool-versions");

    let output = Command::new(env!("CARGO_BIN_EXE_ampland"))
        .current_dir(&work_dir)
        .arg("--config")
        .arg(&config)
        .arg("--cache-dir")
        .arg(&cache)
        .arg("use")
        .arg("node")
        .arg("20.10.0")
        .output()
        .expect("run ampland");

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should try to use node, not read .tool-versions
    // Will fail because node isn't in manifest, but that's expected
    assert!(
        !stderr.contains("python"),
        "Should not try to use python from .tool-versions, got: {}",
        stderr
    );
}
