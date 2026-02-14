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
    #[command(about = "Set the active tool version")]
    Use {
        tool: String,
        version: String,
        #[arg(short, long)]
        global: bool,
        #[arg(long)]
        path: Option<PathBuf>,
    },
    #[command(about = "Install a tool version")]
    Install {
        tool: String,
        version: Option<String>,
    },
    #[command(about = "Uninstall a specific version")]
    Uninstall { tool: String, version: String },
    #[command(about = "Search tools by name")]
    Search { query: Option<String> },
    #[command(about = "List installed tools")]
    List,
    #[command(about = "Clean cache and unused data")]
    Gc,
    #[command(about = "Export current settings")]
    Export {
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "toml")]
        format: Format,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    #[command(about = "Import settings from a file")]
    Import {
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "toml")]
        format: Format,
        file: PathBuf,
    },
    #[command(about = "Check environment health")]
    Doctor,
    #[command(about = "Show resolved executable path")]
    Which { tool: String },
    #[command(about = "Explain tool resolution")]
    Explain { tool: String },
    #[command(about = "Update the manifest")]
    UpdateManifest,
    #[command(about = "Manage shims")]
    Shim {
        #[command(subcommand)]
        command: ShimCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum ShimCommand {
    #[command(about = "Rebuild shims")]
    Rebuild,
    #[command(about = "Add a shim for a tool")]
    Add { tool: String },
}

#[derive(Debug, Copy, Clone, ValueEnum)]
pub enum Format {
    Toml,
    Json,
}
