use std::{collections::HashMap, path::Path};

use chrono::{DateTime, Utc};
use tokio::{io, process::Command};
use tracing::info;

use crate::schedule::parser::TimeSpec;

/// Evaluate `${shell cmd}` variable values in-place; leave unchanged on error or plain strings.
pub async fn resolve_variables(vars: &mut HashMap<String, String>, cwd: &Path) -> io::Result<()> {
    let keys: Vec<String> = vars.keys().cloned().collect();
    for name in keys {
        if let Some(cmd) = vars[&name]
            .strip_prefix("${")
            .and_then(|s| s.strip_suffix('}').to_owned())
        {
            Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .current_dir(cwd)
                .output()
                .await
                .and_then(|output| {
                    if output.status.success() {
                        let val = String::from_utf8_lossy(&output.stdout).trim().to_string();
                        info!(name = %name, value = %val, "resolved variable");
                        *vars.get_mut(&name).unwrap() = val;
                        Ok(())
                    } else {
                        Err(std::io::Error::other(format!(
                            "process exited with return code {:?}",
                            output.status.code()
                        )))
                    }
                })?;
        }
    }

    Ok(())
}

/// Replace `$VAR` in command strings with resolved variable values.
/// Only replaces known variables -- unknown `$REF` pass through to the shell unchanged.
pub fn substitute_variables(cmd: &str, vars: &HashMap<String, String>) -> String {
    let mut result = cmd.to_string();
    // Replace longest names first to avoid prefix collisions (e.g. $FOO before $FO)
    let mut entries: Vec<_> = vars.iter().collect();
    entries.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    for (name, value) in entries {
        result = result.replace(&format!("${name}"), value);
    }
    result
}

/// Convert `TimeSpec` (Absolute or Relative) to `DateTime<Utc>`.
pub fn resolve_time(spec: &TimeSpec, vars: &HashMap<String, String>) -> Option<DateTime<Utc>> {
    match spec {
        TimeSpec::Absolute(dt) => Some(*dt),
        TimeSpec::Relative { variable, offset } => {
            let base_str = vars.get(variable.as_str())?;
            let base = DateTime::parse_from_rfc3339(base_str)
                .ok()?
                .with_timezone(&Utc);
            Some(base + *offset)
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::*;

    // --- Unit tests: substitute_variables ---

    #[test]
    fn substitute_basic_dollar_var() {
        let vars = HashMap::from([("FOO".into(), "bar".into())]);
        assert_eq!(substitute_variables("echo $FOO", &vars), "echo bar");
    }

    #[test]
    fn substitute_unknown_vars_pass_through() {
        let vars = HashMap::from([("FOO".into(), "bar".into())]);
        assert_eq!(
            substitute_variables("echo $FOO $HOME", &vars),
            "echo bar $HOME"
        );
    }

    #[test]
    fn substitute_longer_name_replaced_first() {
        let vars = HashMap::from([("FO".into(), "short".into()), ("FOO".into(), "long".into())]);
        assert_eq!(substitute_variables("echo $FOO", &vars), "echo long");
    }

    #[test]
    fn substitute_no_vars() {
        let vars = HashMap::new();
        assert_eq!(
            substitute_variables("echo hello world", &vars),
            "echo hello world"
        );
    }

    #[test]
    fn substitute_dollar_at_end() {
        let vars = HashMap::new();
        assert_eq!(substitute_variables("echo $", &vars), "echo $");
    }

    // --- Unit tests: resolve_time ---

    #[test]
    fn resolve_time_absolute() {
        let dt = Utc::now();
        let spec = TimeSpec::Absolute(dt);
        assert_eq!(resolve_time(&spec, &HashMap::new()), Some(dt));
    }

    #[test]
    fn resolve_time_relative_with_offset() {
        let vars = HashMap::from([("start".into(), "2026-01-12T10:00:00Z".into())]);
        let spec = TimeSpec::Relative {
            variable: "start".into(),
            offset: TimeDelta::seconds(30),
        };
        let result = resolve_time(&spec, &vars).unwrap();
        assert_eq!(
            result,
            DateTime::parse_from_rfc3339("2026-01-12T10:00:30Z")
                .unwrap()
                .with_timezone(&Utc)
        );
    }

    #[test]
    fn resolve_time_undefined_variable() {
        let spec = TimeSpec::Relative {
            variable: "missing".into(),
            offset: TimeDelta::seconds(0),
        };
        assert_eq!(resolve_time(&spec, &HashMap::new()), None);
    }
}
