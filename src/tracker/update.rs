use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct Update {
    pub timestamp: DateTime<Utc>,

    pub azimuth_degrees: f64,
    pub elevation_degrees: f64,
    pub range_meters: f64,
    pub range_rate_meters_per_second: f64,

    pub tx_frequency_hertz: Option<u64>,
    pub rx_frequency_hertz: Option<u64>,
}
