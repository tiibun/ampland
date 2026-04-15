use std::path::Path;

use crate::error::AppError;

pub fn parse_tool_versions_file(path: &Path) -> Result<Vec<(String, String)>, AppError> {
    let contents = std::fs::read_to_string(path).map_err(|err| AppError::Config {
        message: format!("failed to read {}: {}", path.display(), err),
    })?;

    let mut result = Vec::new();
    for (line_num, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut parts = line.split_whitespace();
        let tool = parts.next().ok_or_else(|| AppError::Config {
            message: format!(
                "invalid format at line {} in {}",
                line_num + 1,
                path.display()
            ),
        })?;
        let version = parts.next().ok_or_else(|| AppError::Config {
            message: format!(
                "missing version at line {} in {}",
                line_num + 1,
                path.display()
            ),
        })?;

        result.push((tool.to_string(), version.to_string()));
    }

    Ok(result)
}

pub fn parse_mise_toml_file(path: &Path) -> Result<Vec<(String, String)>, AppError> {
    let contents = std::fs::read_to_string(path).map_err(|err| AppError::Config {
        message: format!("failed to read {}: {}", path.display(), err),
    })?;

    let table: toml::Value = toml::from_str(&contents).map_err(|err| AppError::Config {
        message: format!("failed to parse {}: {}", path.display(), err),
    })?;

    let Some(tools) = table.get("tools").and_then(|v| v.as_table()) else {
        return Ok(vec![]);
    };

    let mut result = Vec::new();
    for (tool, value) in tools {
        let version = match value {
            toml::Value::String(s) => s.clone(),
            toml::Value::Table(t) => match t.get("version").and_then(|v| v.as_str()) {
                Some(v) => v.to_string(),
                None => {
                    return Err(AppError::Config {
                        message: format!(
                            "missing version for tool '{}' in {}",
                            tool,
                            path.display()
                        ),
                    })
                }
            },
            toml::Value::Array(arr) => match arr.first().and_then(|v| v.as_str()) {
                Some(v) => v.to_string(),
                None => {
                    return Err(AppError::Config {
                        message: format!(
                            "empty version for tool '{}' in {}",
                            tool,
                            path.display()
                        ),
                    })
                }
            },
            _ => {
                return Err(AppError::Config {
                    message: format!(
                        "invalid version format for tool '{}' in {}",
                        tool,
                        path.display()
                    ),
                })
            }
        };
        result.push((tool.clone(), version));
    }

    Ok(result)
}

pub fn parse_volta_from_package_json(path: &Path) -> Result<Vec<(String, String)>, AppError> {
    let contents = std::fs::read_to_string(path).map_err(|err| AppError::Config {
        message: format!("failed to read {}: {}", path.display(), err),
    })?;

    let json: serde_json::Value =
        serde_json::from_str(&contents).map_err(|err| AppError::Config {
            message: format!("failed to parse {}: {}", path.display(), err),
        })?;

    let Some(volta) = json.get("volta").and_then(|v| v.as_object()) else {
        return Ok(vec![]);
    };

    let mut result = Vec::new();
    for (tool, value) in volta {
        let version = match value.as_str() {
            Some(s) => s.to_string(),
            None => {
                return Err(AppError::Config {
                    message: format!(
                        "invalid version for tool '{}' in {}",
                        tool,
                        path.display()
                    ),
                })
            }
        };
        result.push((tool.clone(), version));
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tool_versions_file_valid() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let tool_versions_path = temp_dir.path().join(".tool-versions");

        std::fs::write(
            &tool_versions_path,
            "node 20.10.0\npython 3.11.5\ngo 1.21.0\n",
        )
        .expect("write file");

        let result = parse_tool_versions_file(&tool_versions_path).expect("parse");
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], ("node".to_string(), "20.10.0".to_string()));
        assert_eq!(result[1], ("python".to_string(), "3.11.5".to_string()));
        assert_eq!(result[2], ("go".to_string(), "1.21.0".to_string()));
    }

    #[test]
    fn parse_tool_versions_file_with_comments_and_empty_lines() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let tool_versions_path = temp_dir.path().join(".tool-versions");

        std::fs::write(
            &tool_versions_path,
            "# This is a comment\nnode 20.10.0\n\n# Another comment\npython 3.11.5\n\n",
        )
        .expect("write file");

        let result = parse_tool_versions_file(&tool_versions_path).expect("parse");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], ("node".to_string(), "20.10.0".to_string()));
        assert_eq!(result[1], ("python".to_string(), "3.11.5".to_string()));
    }

    #[test]
    fn parse_tool_versions_file_with_extra_whitespace() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let tool_versions_path = temp_dir.path().join(".tool-versions");

        std::fs::write(
            &tool_versions_path,
            "  node   20.10.0  \n  python   3.11.5  \n",
        )
        .expect("write file");

        let result = parse_tool_versions_file(&tool_versions_path).expect("parse");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], ("node".to_string(), "20.10.0".to_string()));
        assert_eq!(result[1], ("python".to_string(), "3.11.5".to_string()));
    }

    #[test]
    fn parse_tool_versions_file_missing_version() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let tool_versions_path = temp_dir.path().join(".tool-versions");

        std::fs::write(&tool_versions_path, "node\n").expect("write file");

        let result = parse_tool_versions_file(&tool_versions_path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("missing version"));
        assert!(err_msg.contains("line 1"));
    }

    #[test]
    fn parse_tool_versions_file_not_found() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let tool_versions_path = temp_dir.path().join("nonexistent.tool-versions");

        let result = parse_tool_versions_file(&tool_versions_path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("failed to read"));
    }

    #[test]
    fn parse_tool_versions_file_empty() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let tool_versions_path = temp_dir.path().join(".tool-versions");

        std::fs::write(&tool_versions_path, "").expect("write file");

        let result = parse_tool_versions_file(&tool_versions_path).expect("parse");
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn parse_tool_versions_file_only_comments() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let tool_versions_path = temp_dir.path().join(".tool-versions");

        std::fs::write(&tool_versions_path, "# comment 1\n# comment 2\n").expect("write file");

        let result = parse_tool_versions_file(&tool_versions_path).expect("parse");
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn parse_mise_toml_file_valid() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let path = temp_dir.path().join("mise.toml");

        std::fs::write(
            &path,
            "[tools]\nnode = \"20.10.0\"\npython = \"3.11.5\"\n",
        )
        .expect("write file");

        let mut result = parse_mise_toml_file(&path).expect("parse");
        result.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], ("node".to_string(), "20.10.0".to_string()));
        assert_eq!(result[1], ("python".to_string(), "3.11.5".to_string()));
    }

    #[test]
    fn parse_mise_toml_file_with_table_version() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let path = temp_dir.path().join("mise.toml");

        std::fs::write(&path, "[tools]\nnode = { version = \"20.10.0\" }\n").expect("write file");

        let result = parse_mise_toml_file(&path).expect("parse");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], ("node".to_string(), "20.10.0".to_string()));
    }

    #[test]
    fn parse_mise_toml_file_with_array_version() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let path = temp_dir.path().join("mise.toml");

        std::fs::write(&path, "[tools]\nnode = [\"20.10.0\", \"22.0.0\"]\n").expect("write file");

        let result = parse_mise_toml_file(&path).expect("parse");
        assert_eq!(result.len(), 1);
        // First element of the array is used
        assert_eq!(result[0], ("node".to_string(), "20.10.0".to_string()));
    }

    #[test]
    fn parse_mise_toml_file_no_tools_section() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let path = temp_dir.path().join("mise.toml");

        std::fs::write(&path, "[env]\nFOO = \"bar\"\n").expect("write file");

        let result = parse_mise_toml_file(&path).expect("parse");
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn parse_mise_toml_file_not_found() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let path = temp_dir.path().join("nonexistent.toml");

        let result = parse_mise_toml_file(&path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("failed to read"));
    }

    #[test]
    fn parse_volta_from_package_json_valid() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let path = temp_dir.path().join("package.json");

        std::fs::write(
            &path,
            r#"{"name":"my-app","volta":{"node":"20.10.0","npm":"10.2.0"}}"#,
        )
        .expect("write file");

        let mut result = parse_volta_from_package_json(&path).expect("parse");
        result.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], ("node".to_string(), "20.10.0".to_string()));
        assert_eq!(result[1], ("npm".to_string(), "10.2.0".to_string()));
    }

    #[test]
    fn parse_volta_from_package_json_no_volta_section() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let path = temp_dir.path().join("package.json");

        std::fs::write(&path, r#"{"name":"my-app","version":"1.0.0"}"#).expect("write file");

        let result = parse_volta_from_package_json(&path).expect("parse");
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn parse_volta_from_package_json_not_found() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let path = temp_dir.path().join("nonexistent.json");

        let result = parse_volta_from_package_json(&path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("failed to read"));
    }
}
