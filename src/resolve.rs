use std::collections::HashMap;
use std::path::Path;

use globset::Glob;

use crate::config::{Config, Scope};
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
    pub tools: HashMap<String, String>,
    pub scope: Option<ScopeMatch>,
}

#[derive(Debug, Clone)]
pub struct ScopeMatch {
    pub pattern: String,
    pub tools: HashMap<String, String>,
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

pub fn resolve_tool(
    config: &Config,
    cwd: &Path,
    tool: &str,
) -> Result<Resolution, AppError> {
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
    let cwd_str = cwd.to_string_lossy();
    let mut best: Option<(usize, String)> = None;
    let mut best_scope: Option<Scope> = None;

    for scope in config.normalized_scopes()? {
        let glob = Glob::new(&scope.pattern).map_err(|err| AppError::Config {
            message: format!("invalid scope glob '{}': {err}", scope.pattern),
        })?;
        let matcher = glob.compile_matcher();
        if matcher.is_match(cwd_str.as_ref()) {
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
