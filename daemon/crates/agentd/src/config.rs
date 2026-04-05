use std::path::PathBuf;

use serde::Deserialize;
use tracing::info;

/// Daemon configuration loaded from ~/.triumvirate/config.toml
#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default = "default_web_port")]
    pub web_port: u16,

    #[serde(default = "default_db_path")]
    pub db_path: PathBuf,

    #[serde(default)]
    pub agents: AgentsConfig,
}

#[derive(Debug, Deserialize)]
pub struct AgentsConfig {
    #[serde(default = "default_true")]
    pub claude_enabled: bool,

    #[serde(default = "default_true")]
    pub gemini_enabled: bool,

    #[serde(default = "default_true")]
    pub codex_enabled: bool,
}

impl Default for AgentsConfig {
    fn default() -> Self {
        Self {
            claude_enabled: true,
            gemini_enabled: true,
            codex_enabled: true,
        }
    }
}

fn default_web_port() -> u16 {
    8080
}

fn default_db_path() -> PathBuf {
    dirs().join("memory.db")
}

fn default_true() -> bool {
    true
}

/// Resolve the config directory: ~/.triumvirate/
pub fn dirs() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".triumvirate")
}

/// Load config from disk, falling back to defaults if the file doesn't exist.
pub fn load() -> anyhow::Result<Config> {
    let config_path = dirs().join("config.toml");

    if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        let config: Config = toml::from_str(&content)?;
        info!(path = %config_path.display(), "loaded config");
        Ok(config)
    } else {
        info!("no config file found, using defaults");
        Ok(Config {
            web_port: default_web_port(),
            db_path: default_db_path(),
            agents: AgentsConfig::default(),
        })
    }
}

/// Ensure the config directory exists.
pub fn ensure_dirs() -> anyhow::Result<()> {
    let dir = dirs();
    if !dir.exists() {
        std::fs::create_dir_all(&dir)?;
        info!(path = %dir.display(), "created config directory");
    }

    let policies_dir = dir.join("policies");
    if !policies_dir.exists() {
        std::fs::create_dir_all(&policies_dir)?;
        info!(path = %policies_dir.display(), "created policy directory");
    }

    let default_policy = policies_dir.join("default.cedar");
    if !default_policy.exists() {
        std::fs::write(
            &default_policy,
            "permit(
    principal == User::\"human\",
    action in [Action::\"fleet_merge\", Action::\"git_push\", Action::\"file_delete\", Action::\"db_drop\"],
    resource
);

forbid(
    principal != User::\"human\",
    action in [Action::\"fleet_merge\", Action::\"git_push\", Action::\"file_delete\", Action::\"db_drop\"],
    resource
);",
        )?;
        info!(path = %default_policy.display(), "seeded default governance policy");
    }
    Ok(())
}
