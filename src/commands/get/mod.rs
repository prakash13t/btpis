mod iflow;

use anyhow::Result;
use clap::Subcommand;
use crate::config::ConfigFile;

#[derive(Debug, Subcommand)]
pub enum GetCommands {
    /// Get details of a single integration flow
    Iflow {
        /// The ID of the integration flow
        id: String,
        /// The version of the integration flow
        version: String,
        /// Also display the iflow's externalised configuration parameters
        #[arg(long, default_value_t = false)]
        configurations: bool,
        /// Also display the iflow's resources (scripts, schemas, mappings)
        #[arg(long, default_value_t = false)]
        resources: bool,
    },
}

pub async fn handle(cmd: GetCommands, config: &ConfigFile, profile: &str) -> Result<()> {
    match cmd {
        GetCommands::Iflow {
            id,
            version,
            configurations,
            resources,
        } => iflow::handle(config, profile, id, version, configurations, resources).await,
    }
}
