use std::{fs, path::PathBuf};

use anyhow::Context;
use cross_xdg::BaseDirs;
use serde::{Deserialize, Serialize};
use tracing::info;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Config {
    pub station_name: String,
    pub api: ApiConfig,
    pub tasks_path: PathBuf,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ApiConfig {
    pub keys: Vec<ApiKey>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ApiKey {
    pub key: String,
    pub permissions: Vec<Permission>,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub enum Permission {
    ViewTasks,
    SubmitTask,
    EditTask,
    DeleteTask,
    AutoApproveTask,
}

pub fn load(path: Option<&PathBuf>) -> anyhow::Result<Config> {
    let default_config_path = PathBuf::from(BaseDirs::new()?.config_home())
        .join("sat-o-mat")
        .join("config.yml");
    let config_path = path.unwrap_or(&default_config_path);

    if !fs::exists(config_path)? {
        info!("Creating default config file");
        fs::create_dir_all(config_path.parent().unwrap())?;
        fs::write(config_path, serde_yaml::to_string(&Config::default())?)?;
    }

    serde_yaml::from_str(
        fs::read_to_string(config_path)
            .context(format!("Error reading config file {:?}", config_path))?
            .as_ref(),
    )
    .context("Error parsing Config file")
}

impl Default for Config {
    fn default() -> Self {
        let dirs = BaseDirs::new().unwrap();
        let base = dirs.state_home().join("sat-o-mat");

        Self {
            station_name: "Sat-o-Mat Test Station".to_string(),
            api: ApiConfig { keys: Vec::new() },
            tasks_path: base.join("tasks"),
        }
    }
}
