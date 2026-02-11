use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(name = "ampland", version, about = "Tool manager with native shims")]
pub struct Cli {
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    #[arg(long, global = true)]
    pub cache_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    pub shims_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    pub path: Option<PathBuf>,
    #[arg(long, global = true)]
    pub json: bool,
    #[arg(long, global = true)]
    pub quiet: bool,
    #[arg(long, global = true)]
    pub verbose: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Install {
        tool: String,
        version: Option<String>,
    },
    Uninstall {
        tool: String,
        version: String,
    },
    List,
    Gc,
    Export {
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "toml")]
        format: Format,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Import {
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "toml")]
        format: Format,
        file: PathBuf,
    },
    Doctor,
    Which {
        tool: String,
    },
    Explain {
        tool: String,
    },
    Shim {
        #[command(subcommand)]
        command: ShimCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum ShimCommand {
    Rebuild,
    Add { tool: String },
}

#[derive(Debug, Copy, Clone, ValueEnum)]
pub enum Format {
    Toml,
    Json,
}
