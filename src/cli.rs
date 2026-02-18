use std::path::PathBuf;

use clap::{Parser, Subcommand};

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
        version: Option<String>,
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
    Uninstall {
        tool: String,
        version: Option<String>,
    },
    #[command(about = "Search tools by name")]
    Search { query: Option<String> },
    #[command(about = "List installed tools")]
    List,
    #[command(about = "Clean cache and unused data")]
    Gc,
    #[command(about = "Check environment health")]
    Doctor,
    #[command(about = "Show resolved executable path")]
    Which { tool: String },
    #[command(about = "Explain tool resolution")]
    Explain { tool: String },
    #[command(about = "Print shell code to add shims to PATH")]
    Activate,
    #[command(about = "Manage shims")]
    Shim {
        #[command(subcommand)]
        command: ShimCommand,
    },
    #[command(about = "Manage configuration")]
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    #[command(about = "Show config file path and contents")]
    Show,
    #[command(about = "Edit config file in default editor")]
    Edit,
}

#[derive(Debug, Subcommand)]
pub enum ShimCommand {
    #[command(about = "Rebuild shims")]
    Rebuild,
    #[command(about = "Add a shim for a tool")]
    Add { tool: String },
}
