use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::paths::{config_path, expand_tilde};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub global: Global,
    #[serde(default, rename = "scope", skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<Scope>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Global {
    #[serde(default)]
    pub tools: ToolVersions,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Scope {
    #[serde(rename = "path")]
    pub pattern: String,
    #[serde(default)]
    pub tools: ToolVersions,
}

pub type ToolVersions = BTreeMap<String, String>;

impl Config {
    pub fn load(path_override: Option<&Path>) -> Result<(Self, PathBuf), AppError> {
        let path = match path_override {
            Some(path) => path.to_path_buf(),
            None => config_path()?,
        };

        if !path.exists() {
            return Ok((Config::default(), path));
        }

        let contents = fs::read_to_string(&path)?;
        let config: Config = toml::from_str(&contents)?;
        Ok((config, path))
    }

    pub fn save(&self, path: &Path) -> Result<(), AppError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let contents = toml::to_string_pretty(self)?;
        fs::write(path, contents)?;
        Ok(())
    }

    pub fn normalized_scopes(&self) -> Result<Vec<Scope>, AppError> {
        let mut scopes = Vec::new();
        for scope in &self.scopes {
            let expanded = expand_tilde(&scope.pattern)?;
            scopes.push(Scope {
                pattern: expanded,
                tools: scope.tools.clone(),
            });
        }
        Ok(scopes)
    }

    pub fn all_tool_versions(&self) -> HashMap<String, HashSet<String>> {
        let mut map: HashMap<String, HashSet<String>> = HashMap::new();
        for (tool, version) in &self.global.tools {
            map.entry(tool.clone()).or_default().insert(version.clone());
        }
        for scope in &self.scopes {
            for (tool, version) in &scope.tools {
                map.entry(tool.clone()).or_default().insert(version.clone());
            }
        }
        map
    }

    pub fn is_tool_version_in_use(&self, tool: &str, version: &str) -> Vec<String> {
        let mut usages = Vec::new();

        // Check if used globally
        if let Some(global_version) = self.global.tools.get(tool) {
            if global_version == version {
                usages.push("global".to_string());
            }
        }

        // Check if used in any scope
        for scope in &self.scopes {
            if let Some(scope_version) = scope.tools.get(tool) {
                if scope_version == version {
                    usages.push(format!("scope: {}", scope.pattern));
                }
            }
        }

        usages
    }

    pub fn remove_tool_from_scope(
        &mut self,
        normalized_pattern: &str,
        tool: &str,
    ) -> Result<bool, AppError> {
        for index in 0..self.scopes.len() {
            let expanded = expand_tilde(&self.scopes[index].pattern)?;
            if expanded != normalized_pattern {
                continue;
            }
            if self.scopes[index].tools.remove(tool).is_some() {
                if self.scopes[index].tools.is_empty() {
                    self.scopes.remove(index);
                }
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn remove_tool_version_from_scope(
        &mut self,
        normalized_pattern: &str,
        tool: &str,
        version: &str,
    ) -> Result<bool, AppError> {
        for index in 0..self.scopes.len() {
            let expanded = expand_tilde(&self.scopes[index].pattern)?;
            if expanded != normalized_pattern {
                continue;
            }
            if self.scopes[index]
                .tools
                .get(tool)
                .is_some_and(|value| value == version)
            {
                self.scopes[index].tools.remove(tool);
                if self.scopes[index].tools.is_empty() {
                    self.scopes.remove(index);
                }
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(entries: &[(&str, &str)]) -> ToolVersions {
        entries
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn load_missing_file_returns_default() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("missing.toml");
        let (config, loaded_path) = Config::load(Some(&path)).expect("load config");
        assert!(config.global.tools.is_empty());
        assert_eq!(loaded_path, path);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("a/b/config.toml");
        let config = Config {
            global: Global {
                tools: map(&[("node", "22.0.0")]),
            },
            scopes: vec![Scope {
                pattern: "workspace/**".into(),
                tools: map(&[("bun", "1.2.0")]),
            }],
            ..Default::default()
        };
        config.save(&path).expect("save");
        let (loaded, loaded_path) = Config::load(Some(&path)).expect("load");
        assert_eq!(loaded_path, path);
        assert_eq!(loaded.global.tools.get("node"), Some(&"22.0.0".to_string()));
        assert_eq!(loaded.scopes.len(), 1);
    }

    #[test]
    fn save_omits_empty_scope_array() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.toml");
        let config = Config::default();
        config.save(&path).expect("save");
        let contents = fs::read_to_string(&path).expect("read config");
        assert!(!contents.contains("scope = []"));
    }

    #[test]
    fn save_orders_tools_deterministically() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.toml");
        let config = Config {
            global: Global {
                tools: map(&[("python", "3.12.1"), ("bun", "1.2.0"), ("node", "22.0.0")]),
            },
            scopes: vec![Scope {
                pattern: "/workspace/**".into(),
                tools: map(&[("pnpm", "9.0.0"), ("node", "22.0.0")]),
            }],
            ..Default::default()
        };

        config.save(&path).expect("save");
        let contents = fs::read_to_string(&path).expect("read config");

        let bun = contents.find("bun = \"1.2.0\"").expect("bun entry");
        let node = contents.find("node = \"22.0.0\"").expect("node entry");
        let python = contents.find("python = \"3.12.1\"").expect("python entry");
        assert!(bun < node && node < python);
    }

    #[test]
    fn normalized_scopes_and_all_versions_work() {
        let config = Config {
            global: Global {
                tools: map(&[("node", "20.0.0")]),
            },
            scopes: vec![
                Scope {
                    pattern: "~".into(),
                    tools: map(&[("node", "22.0.0"), ("bun", "1.0.0")]),
                },
                Scope {
                    pattern: "workspace/**".into(),
                    tools: map(&[("bun", "1.1.0")]),
                },
            ],
            ..Default::default()
        };

        let normalized = config.normalized_scopes().expect("normalized scopes");
        assert!(normalized[0].pattern.starts_with('/'));

        let versions = config.all_tool_versions();
        assert!(versions.get("node").expect("node set").contains("20.0.0"));
        assert!(versions.get("node").expect("node set").contains("22.0.0"));
        assert!(versions.get("bun").expect("bun set").contains("1.0.0"));
        assert!(versions.get("bun").expect("bun set").contains("1.1.0"));
    }

    #[test]
    fn is_tool_version_in_use_detects_global_usage() {
        let config = Config {
            global: Global {
                tools: map(&[("node", "20.0.0"), ("bun", "1.0.0")]),
            },
            scopes: vec![],
            ..Default::default()
        };

        let usages = config.is_tool_version_in_use("node", "20.0.0");
        assert!(!usages.is_empty());
        assert!(usages.iter().any(|u| u == "global"));

        let unused = config.is_tool_version_in_use("node", "22.0.0");
        assert!(unused.is_empty());
    }

    #[test]
    fn is_tool_version_in_use_detects_scoped_usage() {
        let config = Config {
            global: Global {
                tools: map(&[("node", "20.0.0")]),
            },
            scopes: vec![
                Scope {
                    pattern: "/workspace/**".into(),
                    tools: map(&[("node", "22.0.0")]),
                },
                Scope {
                    pattern: "/project/**".into(),
                    tools: map(&[("bun", "1.0.0")]),
                },
            ],
            ..Default::default()
        };

        let usages = config.is_tool_version_in_use("node", "22.0.0");
        assert!(!usages.is_empty());
        assert!(usages.iter().any(|u| u.contains("workspace")));

        let usages = config.is_tool_version_in_use("bun", "1.0.0");
        assert!(!usages.is_empty());
        assert!(usages.iter().any(|u| u.contains("project")));

        let unused = config.is_tool_version_in_use("deno", "1.0.0");
        assert!(unused.is_empty());
    }

    #[test]
    fn remove_tool_version_from_scope_removes_only_matching_entry() {
        let mut config = Config {
            scopes: vec![Scope {
                pattern: "/workspace/**".into(),
                tools: map(&[("node", "22.0.0"), ("bun", "1.0.0")]),
            }],
            ..Default::default()
        };

        let removed = config
            .remove_tool_version_from_scope("/workspace/**", "node", "22.0.0")
            .expect("remove");
        assert!(removed);
        assert!(!config.scopes[0].tools.contains_key("node"));
        assert_eq!(
            config.scopes[0].tools.get("bun"),
            Some(&"1.0.0".to_string())
        );
    }

    #[test]
    fn remove_tool_version_from_scope_removes_empty_scope() {
        let mut config = Config {
            scopes: vec![Scope {
                pattern: "/workspace/**".into(),
                tools: map(&[("node", "22.0.0")]),
            }],
            ..Default::default()
        };

        let removed = config
            .remove_tool_version_from_scope("/workspace/**", "node", "22.0.0")
            .expect("remove");
        assert!(removed);
        assert!(config.scopes.is_empty());
    }
}
