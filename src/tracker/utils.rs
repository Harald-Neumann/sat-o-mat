use std::str::FromStr;

#[derive(Clone, Debug, Copy)]
pub struct Frequency(u64);

#[derive(Clone, Debug)]
/// The possible destinations to send tracker output information.
pub enum Output {
    /// Send frequency information to a `rigctld`-compatible server at the specified address.
    /// Assumes split mode operation.
    Rigctl(String),
    /// Send (azimuth, elevation) angles to a `rotctld`-compatible server at the specified address.
    Rotctl(String),
    /// Record all outputs to the given file.
    File(String),
    /// Publish all tracker events to the specified Zenoh topic
    Zenoh(String),
}

/// Parses strings like:
/// ```
/// rotctl=127.0.0.1:4533
/// rigctl=127.0.0.1:9998
/// file=tracker.json
/// zenoh=tracker/foo
/// ```
impl FromStr for Output {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (key, value) = s
            .split_once('=')
            .ok_or_else(|| format!("expected 'type=address', got '{s}'"))?;

        let v = value.to_string();

        match key.trim().to_lowercase().as_str() {
            "rigctl" => Ok(Output::Rigctl(v)),
            "rotctl" => Ok(Output::Rotctl(v)),
            "file" => Ok(Output::File(v)),
            "zenoh" => Ok(Output::Zenoh(v)),
            other => Err(format!(
                "unknown output type '{other}', expected rigctl/rotctl/file/zenoh"
            )),
        }
    }
}

/// Parses frequency expressions like "100.4 MHz", "2.4 GHz", "10000 kHz" into Hertz.
impl FromStr for Frequency {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let (num_str, unit) = s
            .rsplit_once(char::is_whitespace)
            .ok_or_else(|| format!("expected '<number> <unit>', got '{s}'"))?;

        let value: f64 = num_str
            .trim()
            .parse()
            .map_err(|e| format!("invalid number '{num_str}': {e}"))?;

        let multiplier: f64 = match unit.trim().to_lowercase().as_str() {
            "hz" => 1.0,
            "khz" => 1_000.0,
            "mhz" => 1_000_000.0,
            "ghz" => 1_000_000_000.0,
            other => return Err(format!("unknown unit '{other}', expected Hz/kHz/MHz/GHz")),
        };

        Ok(Frequency((value * multiplier).round() as u64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_frequency_valid() {
        let f: Frequency = "100.4 MHz".parse().unwrap();
        assert_eq!(f.0, 100_400_000);

        let f: Frequency = "2.4 GHz".parse().unwrap();
        assert_eq!(f.0, 2_400_000_000);

        let f: Frequency = "10000 kHz".parse().unwrap();
        assert_eq!(f.0, 10_000_000);

        let f: Frequency = "440000 Hz".parse().unwrap();
        assert_eq!(f.0, 440_000);

        let f: Frequency = "145.8 mhz".parse().unwrap();
        assert_eq!(f.0, 145_800_000);
    }

    #[test]
    fn parse_frequency_invalid() {
        assert!("12345".parse::<Frequency>().is_err());
        assert!("abc MHz".parse::<Frequency>().is_err());
        assert!("100 THz".parse::<Frequency>().is_err());
    }

    #[test]
    fn parse_tracker_output_valid() {
        let o: Output = "rigctl=127.0.0.1:9998".parse().unwrap();
        assert!(matches!(o, Output::Rigctl(a) if a == "127.0.0.1:9998"));

        let o: Output = "rotctl=127.0.0.1:4533".parse().unwrap();
        assert!(matches!(o, Output::Rotctl(a) if a == "127.0.0.1:4533"));

        let o: Output = "file=tracker.json".parse().unwrap();
        assert!(matches!(o, Output::File(p) if p == "tracker.json"));

        let o: Output = "zenoh=tracker/foo".parse().unwrap();
        assert!(matches!(o, Output::Zenoh(t) if t == "tracker/foo"));
    }

    #[test]
    fn parse_tracker_output_invalid() {
        assert!("rigctl:localhost".parse::<Output>().is_err());
        assert!("mqtt=localhost:1883".parse::<Output>().is_err());
    }
}
