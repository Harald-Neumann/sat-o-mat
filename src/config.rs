use std::{fs, path::PathBuf};

use anyhow::Context;
use cross_xdg::BaseDirs;
use serde::{Deserialize, Serialize};
use tracing::info;

#[derive(Deserialize, Serialize, Debug)]
pub struct Config {
    station_name: String,
    api: ApiConfig,
    task_directory: String,
    artifact_directory: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ApiConfig {
    keys: Vec<ApiKey>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ApiKey {
    key: String,
    permission: Vec<Permission>,
}

#[derive(Deserialize, Serialize, Debug)]
pub enum Permission {
    ViewTasks,
    SubmitTask,
    DeleteTask,
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
        Self {
            station_name: "Sat-o-Mat Test Station".to_string(),
            api: ApiConfig { keys: Vec::new() },
            task_directory: Default::default(),
            artifact_directory: Default::default(),
        }
    }
}
