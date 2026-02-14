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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_match_error_kind() {
        assert_eq!(
            AppError::ToolNotInstalled { tool: "x".into() }.exit_code(),
            3
        );
        assert_eq!(
            AppError::Config {
                message: "x".into()
            }
            .exit_code(),
            4
        );
        assert_eq!(
            AppError::Cache {
                message: "x".into()
            }
            .exit_code(),
            5
        );
        assert_eq!(
            AppError::Io {
                message: "x".into()
            }
            .exit_code(),
            5
        );
        assert_eq!(
            AppError::Other {
                message: "x".into()
            }
            .exit_code(),
            1
        );
    }

    #[test]
    fn from_conversions_map_to_expected_variant() {
        let io_err = std::io::Error::other("io");
        assert!(matches!(AppError::from(io_err), AppError::Io { .. }));

        let de_err = toml::from_str::<toml::Value>("=").expect_err("invalid toml");
        assert!(matches!(AppError::from(de_err), AppError::Config { .. }));

        let ser_err = toml::to_string_pretty(&f64::NAN).expect_err("nan is invalid in toml");
        assert!(matches!(AppError::from(ser_err), AppError::Config { .. }));

        let json_err = serde_json::from_str::<serde_json::Value>("{").expect_err("invalid json");
        assert!(matches!(AppError::from(json_err), AppError::Config { .. }));
    }
}
