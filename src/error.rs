use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("tool not installed: {tool}")]
    ToolNotInstalled { tool: String },
    #[error("config error: {message}")]
    Config { message: String },
    #[error("cache error: {message}")]
    Cache { message: String },
    #[error("io error: {message}")]
    Io { message: String },
    #[error("unexpected error: {message}")]
    Other { message: String },
}

impl AppError {
    pub fn exit_code(&self) -> i32 {
        match self {
            AppError::ToolNotInstalled { .. } => 3,
            AppError::Config { .. } => 4,
            AppError::Cache { .. } => 5,
            AppError::Io { .. } => 5,
            AppError::Other { .. } => 1,
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::Io {
            message: err.to_string(),
        }
    }
}

impl From<toml::de::Error> for AppError {
    fn from(err: toml::de::Error) -> Self {
        AppError::Config {
            message: err.to_string(),
        }
    }
}

impl From<toml::ser::Error> for AppError {
    fn from(err: toml::ser::Error) -> Self {
        AppError::Config {
            message: err.to_string(),
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::Config {
            message: err.to_string(),
        }
    }
}

