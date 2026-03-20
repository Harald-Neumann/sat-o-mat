use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, TimeDelta, Utc};
use serde::Deserialize;
use serde::Serialize;
use serde::de;
use serde::ser::{self, SerializeMap};
use serde_yaml::Value;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid YAML: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("missing required variable: {0}")]
    MissingVariable(&'static str),
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Task {
    #[serde(default)]
    pub variables: HashMap<String, String>,
    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default)]
    pub cleanup: Vec<Step>,
}

pub const TASK_STATES: &[&str] = &["Active", "PendingApproval", "Completed", "Failed"];
pub const TASK_EXTENSION: &str = ".yaml";

impl Task {
    pub fn from_yaml_str(yaml: &str) -> Result<Self, Error> {
        let schedule: Task = serde_yaml::from_str(yaml)?;
        if !schedule.variables.contains_key("end") {
            return Err(Error::MissingVariable("end"));
        }
        Ok(schedule)
    }

    /// Resolve a task ID to its filename on disk.
    pub fn filename(id: &str) -> String {
        format!("{id}{TASK_EXTENSION}")
    }

    /// Strip the `.yaml` extension from a filename to get the task ID.
    pub fn id_from_filename(filename: &str) -> &str {
        filename.strip_suffix(TASK_EXTENSION).unwrap_or(filename)
    }

    /// Find a task file across all state directories. Returns (state_name, file_contents).
    pub async fn find(tasks_path: &Path, id: &str) -> Option<(String, String)> {
        // Reject path traversal
        if id.contains('/') || id.contains('\\') || id == ".." || id == "." {
            return None;
        }

        let filename = Self::filename(id);
        for &dir in TASK_STATES {
            let path = tasks_path.join(dir).join(&filename);
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                return Some((dir.to_string(), content));
            }
        }
        None
    }

    /// Extract start/end times from this task's variables.
    pub fn time_range(&self) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        let start = self
            .variables
            .get("start")
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))?;
        let end = self
            .variables
            .get("end")
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))?;
        Some((start, end))
    }
}

#[derive(Debug, Clone)]
pub struct Step {
    pub cmd: String,
    pub time: Option<TimeSpec>,
    pub wait: bool,
    pub on_fail: OnFail,
}

#[derive(Debug, Clone)]
pub enum TimeSpec {
    Absolute(DateTime<Utc>),
    Relative { variable: String, offset: TimeDelta },
}

#[derive(Debug, Default, Clone)]
pub enum OnFail {
    #[default]
    Abort,
    Continue,
    Retry(u32),
}

// --- deserialization ---
const STEP_FIELDS: &[&str] = &["cmd", "time", "wait", "on_fail"];

impl<'de> Deserialize<'de> for Step {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::String(cmd) => Ok(Step {
                cmd,
                time: None,
                wait: false,
                on_fail: OnFail::Abort,
            }),
            Value::Mapping(map) => {
                // Make sure only expected keys are given
                for key in map.keys() {
                    let field = match key {
                        Value::String(s) => s.as_str(),
                        _ => return Err(de::Error::custom("step keys must be strings")),
                    };
                    if !STEP_FIELDS.contains(&field) {
                        return Err(de::Error::unknown_field(field, STEP_FIELDS));
                    }
                }

                Ok(Step {
                    cmd: get_field(&map, "cmd")?.ok_or_else(|| de::Error::missing_field("cmd"))?,
                    time: get_field(&map, "time")?,
                    wait: get_field(&map, "wait")?.unwrap_or_default(),
                    on_fail: get_field(&map, "on_fail")?.unwrap_or_default(),
                })
            }
            _ => Err(de::Error::custom("step must be a string or mapping")),
        }
    }
}

impl<'de> Deserialize<'de> for TimeSpec {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;

        // Absolute time
        if let Ok(dt) = DateTime::parse_from_rfc3339(&s) {
            return Ok(TimeSpec::Absolute(dt.with_timezone(&Utc)));
        }

        // All relative forms: T+dur, T-dur, $var+dur, $var-dur.
        let (variable, rest) = if let Some(rest) = s.strip_prefix('T') {
            ("start", rest)
        } else if let Some(rest) = s.strip_prefix('$') {
            let pos = rest
                .find(['+', '-'])
                .ok_or_else(|| de::Error::custom(format!("invalid time spec: {s}")))?;
            (rest[..pos].trim(), &rest[pos..])
        } else {
            return Err(de::Error::custom(format!("invalid time spec: {s}")));
        };

        // Reject empty variable names
        if variable.is_empty() {
            return Err(de::Error::custom(format!("invalid time spec: {s}")));
        }

        // Negative or positive duration offset?
        let (negate, dur_str) = if let Some(d) = rest.strip_prefix('+') {
            (false, d)
        } else if let Some(d) = rest.strip_prefix('-') {
            (true, d)
        } else {
            return Err(de::Error::custom(format!("invalid time spec: {s}")));
        };

        // Parse duration
        let dur = humantime::parse_duration(dur_str.trim()).map_err(de::Error::custom)?;
        let offset = TimeDelta::from_std(dur).map_err(de::Error::custom)?;

        Ok(TimeSpec::Relative {
            variable: variable.into(),
            offset: if negate { -offset } else { offset },
        })
    }
}

impl<'de> Deserialize<'de> for OnFail {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "abort" => Ok(OnFail::Abort),
            "continue" => Ok(OnFail::Continue),
            _ if s.starts_with("retry(") && s.ends_with(')') => {
                let n: u32 = s["retry(".len()..s.len() - 1]
                    .parse()
                    .map_err(de::Error::custom)?;
                Ok(OnFail::Retry(n))
            }
            _ => Err(de::Error::custom(format!("invalid on_fail: {s}"))),
        }
    }
}

// --- serialization ---

impl Serialize for OnFail {
    fn serialize<S: ser::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let s = match self {
            OnFail::Abort => "abort",
            OnFail::Continue => "continue",
            OnFail::Retry(n) => &format!("retry({n})"),
        };
        serializer.serialize_str(s)
    }
}

impl Serialize for TimeSpec {
    fn serialize<S: ser::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let s = match self {
            TimeSpec::Absolute(dt) => dt.to_rfc3339(),
            TimeSpec::Relative { variable, offset } => {
                let (sign, abs_offset) = if *offset < TimeDelta::zero() {
                    ("-", -*offset)
                } else {
                    ("+", *offset)
                };
                let dur = abs_offset.to_std().map_err(ser::Error::custom)?;
                format!("{variable}{sign}{}", humantime::format_duration(dur))
            }
        };
        serializer.serialize_str(&s)
    }
}

impl Serialize for Step {
    fn serialize<S: ser::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let is_simple = self.time.is_none() && !self.wait && matches!(self.on_fail, OnFail::Abort);
        if is_simple {
            return serializer.serialize_str(&self.cmd);
        }

        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("cmd", &self.cmd)?;
        if let Some(time) = &self.time {
            map.serialize_entry("time", time)?;
        }
        if self.wait {
            map.serialize_entry("wait", &self.wait)?;
        }
        if !matches!(self.on_fail, OnFail::Abort) {
            map.serialize_entry("on_fail", &self.on_fail)?;
        }
        map.end()
    }
}

fn get_field<T: de::DeserializeOwned, E: de::Error>(
    map: &serde_yaml::Mapping,
    key: &str,
) -> Result<Option<T>, E> {
    map.get(key)
        .map(|v| serde_yaml::from_value(v.clone()).map_err(de::Error::custom))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_YAML: &str = r#"
variables:
  start: "2026-01-12T10:00:00Z"
  end: "2026-01-12T10:10:00Z"

steps:
  - echo hello
  - time: "$end - 10 seconds"
    cmd: echo "Pass about to end!"
    wait: true
    on_fail: retry(3)

cleanup:
  - echo "hello from cleanup"
"#;

    #[test]
    fn parse_full_schedule() {
        let sched = Task::from_yaml_str(FULL_YAML).unwrap();
        assert_eq!(sched.variables["start"], "2026-01-12T10:00:00Z");
        assert_eq!(sched.steps.len(), 2);
        assert_eq!(sched.cleanup.len(), 1);

        assert_eq!(sched.steps[0].cmd, "echo hello");
        assert!(sched.steps[0].time.is_none());
        assert!(!sched.steps[0].wait);

        let step = &sched.steps[1];
        assert!(step.wait);
        assert!(
            matches!(&step.time, Some(TimeSpec::Relative { variable, offset })
            if variable == "end" && *offset == TimeDelta::seconds(-10))
        );
        assert!(matches!(&step.on_fail, OnFail::Retry(3)));
    }

    #[test]
    fn missing_end_variable() {
        let yaml = "variables:\n  start: '2026-01-01T00:00:00Z'\nsteps: []\n";
        assert!(matches!(
            Task::from_yaml_str(yaml),
            Err(Error::MissingVariable("end"))
        ));
    }

    fn deser_time_spec(s: &str) -> TimeSpec {
        serde_yaml::from_value(Value::String(s.into())).unwrap()
    }

    #[test]
    fn time_spec_absolute() {
        assert!(matches!(
            deser_time_spec("2026-01-12T10:00:00Z"),
            TimeSpec::Absolute(_)
        ));
    }

    #[test]
    fn time_spec_t_plus() {
        assert!(matches!(
            deser_time_spec("T+30 seconds"),
            TimeSpec::Relative { variable, offset }
            if variable == "start" && offset == TimeDelta::seconds(30)
        ));
    }

    #[test]
    fn time_spec_var_minus() {
        assert!(matches!(
            deser_time_spec("$end - 1 minute"),
            TimeSpec::Relative { variable, offset }
            if variable == "end" && offset == TimeDelta::minutes(-1)
        ));
    }

    #[test]
    fn time_spec_rejects_empty_variable() {
        let err =
            serde_yaml::from_value::<TimeSpec>(Value::String("$ + 10 seconds".into())).unwrap_err();
        assert!(err.to_string().contains("invalid time spec"));
    }

    #[test]
    fn time_spec_large_duration_returns_error() {
        let err =
            serde_yaml::from_value::<TimeSpec>(Value::String("T+9223372036854775808s".into()))
                .unwrap_err();
        assert!(err.to_string().contains("out of range"));
    }

    #[test]
    fn step_rejects_unknown_fields() {
        let yaml = r#"
variables:
  end: "2026-01-12T10:10:00Z"
steps:
  - cmd: echo hello
    waait: true
"#;

        let err = Task::from_yaml_str(yaml).unwrap_err();
        match err {
            Error::Yaml(e) => assert!(e.to_string().contains("unknown field")),
            _ => panic!("expected yaml error"),
        }
    }
}
