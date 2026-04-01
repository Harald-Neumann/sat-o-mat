use std::{fs, path::PathBuf};

use anyhow::Context;
use cross_xdg::BaseDirs;
use lox_space::{
    analysis::visibility::ElevationMask,
    bodies::DynOrigin,
    core::coords::LonLatAlt,
    prelude::{GroundLocation, GroundStation},
};
use serde::{Deserialize, Serialize, Serializer, de, ser::SerializeStruct};
use tracing::info;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Config {
    pub station_name: String,
    pub api: ApiConfig,
    pub tasks_path: PathBuf,
    pub tle_path: PathBuf,
    #[serde(
        default,
        deserialize_with = "deserialize_ground_station",
        serialize_with = "serialize_ground_station"
    )]
    pub ground_station: Option<GroundStation>,
}

#[derive(Deserialize)]
struct GroundStationDef {
    longitude: f64,
    latitude: f64,
    altitude: f64,
    min_elevation: f64,
}

fn deserialize_ground_station<'de, D>(deserializer: D) -> Result<Option<GroundStation>, D::Error>
where
    D: de::Deserializer<'de>,
{
    Option::<GroundStationDef>::deserialize(deserializer)?
        .map(|def| {
            let coords = LonLatAlt::from_degrees(def.longitude, def.latitude, def.altitude)
                .map_err(de::Error::custom)?;
            let location =
                GroundLocation::try_new(coords, DynOrigin::Earth).map_err(de::Error::custom)?;
            let mask = ElevationMask::with_fixed_elevation(def.min_elevation.to_radians());
            Ok(GroundStation::new("GS", location, mask))
        })
        .transpose()
}

fn serialize_ground_station<S>(gs: &Option<GroundStation>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match gs {
        None => serializer.serialize_none(),
        Some(gs) => {
            let coords = gs.location().coordinates();
            let mut state = serializer.serialize_struct("GroundStation", 4)?;
            state.serialize_field("longitude", &coords.lon().to_degrees())?;
            state.serialize_field("latitude", &coords.lat().to_degrees())?;
            state.serialize_field("altitude", &coords.alt().to_meters())?;
            state.serialize_field("min_elevation", &gs.mask().min_elevation(0.0).to_degrees())?;
            state.end()
        }
    }
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
    SubmitFromTemplate,
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
            tle_path: base.join("tle"),
            ground_station: Some(GroundStation::new(
                "GS",
                GroundLocation::try_new(
                    LonLatAlt::from_degrees(13.4, 52.52, 100.0).unwrap(),
                    DynOrigin::Earth,
                )
                .unwrap(),
                ElevationMask::with_fixed_elevation(0.0),
            )),
        }
    }
}
