use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::paths::{config_path, expand_tilde};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub global: Global,
    #[serde(default, rename = "scope")]
    pub scopes: Vec<Scope>,
    #[serde(default)]
    pub manifest: ManifestConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Global {
    #[serde(default)]
    pub tools: HashMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Scope {
    #[serde(rename = "path")]
    pub pattern: String,
    #[serde(default)]
    pub tools: HashMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestConfig {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub sig_url: Option<String>,
    #[serde(default)]
    pub public_key: Option<String>,
    #[serde(default)]
    pub ttl_hours: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockFile {
    pub path: String,
    #[serde(default)]
    pub tools: HashMap<String, String>,
}

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
            map.entry(tool.clone())
                .or_default()
                .insert(version.clone());
        }
        for scope in &self.scopes {
            for (tool, version) in &scope.tools {
                map.entry(tool.clone())
                    .or_default()
                    .insert(version.clone());
            }
        }
        map
    }
}

impl LockFile {
    pub fn from_path_and_tools(path: &Path, tools: HashMap<String, String>) -> Self {
        LockFile {
            path: path.to_string_lossy().to_string(),
            tools,
        }
    }

    pub fn to_string(&self, format: crate::cli::Format) -> Result<String, AppError> {
        match format {
            crate::cli::Format::Toml => Ok(toml::to_string_pretty(self)?),
            crate::cli::Format::Json => Ok(serde_json::to_string_pretty(self)?),
        }
    }

    pub fn parse(contents: &str, format: crate::cli::Format) -> Result<Self, AppError> {
        match format {
            crate::cli::Format::Toml => Ok(toml::from_str(contents)?),
            crate::cli::Format::Json => Ok(serde_json::from_str(contents)?),
        }
    }
}
