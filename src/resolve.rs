use std::path::Path;

use globset::Glob;

use crate::config::{Config, Scope, ToolVersions};
use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct Resolution {
    pub tool: String,
    pub version: String,
    pub source: ResolutionSource,
}

#[derive(Debug, Clone)]
pub enum ResolutionSource {
    Global,
    Scope { pattern: String },
    ScopedFallback { pattern: String },
}

#[derive(Debug, Clone)]
pub struct ResolveResult {
    pub tools: ToolVersions,
    pub scope: Option<ScopeMatch>,
}

#[derive(Debug, Clone)]
pub struct ScopeMatch {
    pub pattern: String,
    pub tools: ToolVersions,
}

pub fn resolve_tools(config: &Config, cwd: &Path) -> Result<ResolveResult, AppError> {
    let scope = select_scope(config, cwd)?;
    let mut tools = config.global.tools.clone();
    if let Some(scope_match) = &scope {
        for (tool, version) in &scope_match.tools {
            tools.insert(tool.clone(), version.clone());
        }
    }

    Ok(ResolveResult { tools, scope })
}

pub fn resolve_tool(config: &Config, cwd: &Path, tool: &str) -> Result<Resolution, AppError> {
    let resolve = resolve_tools(config, cwd)?;
    if let Some(version) = resolve.tools.get(tool) {
        if let Some(scope) = resolve.scope {
            let source = if scope.tools.contains_key(tool) {
                ResolutionSource::Scope {
                    pattern: scope.pattern.clone(),
                }
            } else {
                ResolutionSource::ScopedFallback {
                    pattern: scope.pattern.clone(),
                }
            };
            return Ok(Resolution {
                tool: tool.to_string(),
                version: version.clone(),
                source,
            });
        }

        return Ok(Resolution {
            tool: tool.to_string(),
            version: version.clone(),
            source: ResolutionSource::Global,
        });
    }

    Err(AppError::Config {
        message: format!("no version configured for {tool}"),
    })
}

fn select_scope(config: &Config, cwd: &Path) -> Result<Option<ScopeMatch>, AppError> {
    let cwd_str = normalize_separators(cwd.to_string_lossy().as_ref());
    let mut best: Option<(usize, String)> = None;
    let mut best_scope: Option<Scope> = None;

    for scope in config.normalized_scopes()? {
        if scope_matches(&scope.pattern, &cwd_str)? {
            let score = scope.pattern.len();
            match &best {
                Some((best_score, _)) if *best_score >= score => {}
                _ => {
                    best = Some((score, scope.pattern.clone()));
                    best_scope = Some(scope.clone());
                }
            }
        }
    }

    Ok(best_scope.map(|scope| ScopeMatch {
        pattern: scope.pattern,
        tools: scope.tools,
    }))
}

fn scope_matches(pattern: &str, cwd: &str) -> Result<bool, AppError> {
    let normalized_pattern = normalize_separators(pattern);
    let glob = Glob::new(&normalized_pattern).map_err(|err| AppError::Config {
        message: format!("invalid scope glob '{pattern}': {err}"),
    })?;
    let matcher = glob.compile_matcher();
    if matcher.is_match(cwd) {
        return Ok(true);
    }

    let prefix = normalized_pattern.strip_suffix("/**");
    Ok(match prefix {
        Some(prefix) => cwd == prefix,
        None => false,
    })
}

fn normalize_separators(path: &str) -> String {
    path.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Global, Scope};

    fn map(entries: &[(&str, &str)]) -> ToolVersions {
        entries
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn resolves_from_global_when_no_scope_matches() {
        let config = Config {
            global: Global {
                tools: map(&[("node", "20.11.0")]),
            },
            scopes: vec![Scope {
                pattern: "workspace-*".to_string(),
                tools: map(&[("node", "22.0.0")]),
            }],
            ..Default::default()
        };

        let resolved = resolve_tool(&config, Path::new("other-dir"), "node").unwrap();
        assert_eq!(resolved.version, "20.11.0");
        assert!(matches!(resolved.source, ResolutionSource::Global));
    }

    #[test]
    fn resolves_scope_override_and_scoped_fallback() {
        let config = Config {
            global: Global {
                tools: map(&[("node", "20.11.0"), ("bun", "1.1.0")]),
            },
            scopes: vec![Scope {
                pattern: "*".to_string(),
                tools: map(&[("node", "22.0.0")]),
            }],
            ..Default::default()
        };

        let scoped = resolve_tool(&config, Path::new("workspace"), "node").unwrap();
        assert_eq!(scoped.version, "22.0.0");
        assert!(matches!(
            scoped.source,
            ResolutionSource::Scope { pattern } if pattern == "*"
        ));

        let fallback = resolve_tool(&config, Path::new("workspace"), "bun").unwrap();
        assert_eq!(fallback.version, "1.1.0");
        assert!(matches!(
            fallback.source,
            ResolutionSource::ScopedFallback { pattern } if pattern == "*"
        ));
    }

    #[test]
    fn selects_most_specific_matching_scope() {
        let config = Config {
            global: Global {
                tools: map(&[("node", "20.11.0")]),
            },
            scopes: vec![
                Scope {
                    pattern: "*".to_string(),
                    tools: map(&[("node", "21.0.0")]),
                },
                Scope {
                    pattern: "workspace*".to_string(),
                    tools: map(&[("node", "22.0.0")]),
                },
            ],
            ..Default::default()
        };

        let resolved = resolve_tool(&config, Path::new("workspace"), "node").unwrap();
        assert_eq!(resolved.version, "22.0.0");
        assert!(matches!(
            resolved.source,
            ResolutionSource::Scope { pattern } if pattern == "workspace*"
        ));
    }

    #[test]
    fn returns_errors_for_invalid_glob_and_unknown_tool() {
        let config = Config {
            scopes: vec![Scope {
                pattern: "[".to_string(),
                tools: map(&[("node", "22.0.0")]),
            }],
            ..Default::default()
        };
        let err = resolve_tools(&config, Path::new("workspace")).expect_err("invalid glob");
        assert!(matches!(err, AppError::Config { .. }));

        let config = Config::default();
        let err = resolve_tool(&config, Path::new("workspace"), "node").expect_err("missing");
        assert!(matches!(err, AppError::Config { .. }));
    }

    #[test]
    fn matches_scope_when_cwd_uses_windows_separators() {
        let config = Config {
            global: Global {
                tools: map(&[("node", "20.11.0")]),
            },
            scopes: vec![Scope {
                pattern: "C:/workspace/**".to_string(),
                tools: map(&[("node", "22.0.0")]),
            }],
            ..Default::default()
        };

        let resolved = resolve_tool(&config, Path::new(r"C:\workspace\project"), "node").unwrap();
        assert_eq!(resolved.version, "22.0.0");
        assert!(matches!(
            resolved.source,
            ResolutionSource::Scope { pattern } if pattern == "C:/workspace/**"
        ));
    }

    #[test]
    fn matches_scope_when_pattern_uses_windows_separators() {
        let config = Config {
            global: Global {
                tools: map(&[("node", "20.11.0")]),
            },
            scopes: vec![Scope {
                pattern: r"C:\workspace\**".to_string(),
                tools: map(&[("node", "22.0.0")]),
            }],
            ..Default::default()
        };

        let resolved = resolve_tool(&config, Path::new("C:/workspace/project"), "node").unwrap();
        assert_eq!(resolved.version, "22.0.0");
        assert!(matches!(
            resolved.source,
            ResolutionSource::Scope { pattern } if pattern == r"C:\workspace\**"
        ));
    }
}
