use std::env;
use std::path::{Path, PathBuf};

const FC_MAX_TURNS_ENV: &str = "FC_MAX_TURNS";
const FC_MAX_COMMANDS_ENV: &str = "FC_MAX_COMMANDS";
const FC_TIMEOUT_MS_ENV: &str = "FC_TIMEOUT_MS";
const SCOPE_SNAPSHOT_TREE_DEPTH_ENV: &str = "SCOPE_SNAPSHOT_TREE_DEPTH";
const FC_RESULT_MAX_LINES_ENV: &str = "FC_RESULT_MAX_LINES";
const FC_READFILE_MAX_LINES_ENV: &str = "FC_READFILE_MAX_LINES";
const FC_LINE_MAX_CHARS_ENV: &str = "FC_LINE_MAX_CHARS";
const SWEGREP_DEBUG_ENV: &str = "SWEGREP_DEBUG";
const SWEGREP_DEBUG_LOG_ENV: &str = "SWEGREP_DEBUG_LOG";
const SWEGREP_PATH_FILTER_ENV: &str = "SWEGREP_PATH_FILTER";
const SWEGREP_RG_PATH_ENV: &str = "SWEGREP_RG_PATH";

pub(crate) const DEFAULT_FC_MAX_TURNS: usize = 4;
pub(crate) const FC_MAX_TURNS_RANGE: std::ops::RangeInclusive<usize> = 4..=6;
pub(crate) const DEFAULT_FC_MAX_COMMANDS: usize = 8;
pub(crate) const FC_MAX_COMMANDS_RANGE: std::ops::RangeInclusive<usize> = 1..=8;
pub(crate) const DEFAULT_FC_TIMEOUT_MS: u64 = 30_000;
pub(crate) const FC_TIMEOUT_MS_RANGE: std::ops::RangeInclusive<u64> = 1_000..=300_000;
pub(crate) const DEFAULT_SCOPE_SNAPSHOT_TREE_DEPTH: usize = 4;
pub(crate) const SCOPE_SNAPSHOT_TREE_DEPTH_RANGE: std::ops::RangeInclusive<usize> = 0..=8;
pub(crate) const DEFAULT_FC_RESULT_MAX_LINES: usize = 80;
pub(crate) const FC_RESULT_MAX_LINES_RANGE: std::ops::RangeInclusive<usize> = 1..=500;
pub(crate) const DEFAULT_FC_READFILE_MAX_LINES: usize = 400;
pub(crate) const FC_READFILE_MAX_LINES_RANGE: std::ops::RangeInclusive<usize> = 1..=10_000;
pub(crate) const DEFAULT_FC_LINE_MAX_CHARS: usize = 300;
pub(crate) const FC_LINE_MAX_CHARS_RANGE: std::ops::RangeInclusive<usize> = 20..=10_000;

/// Default Windsurf application version sent in protocol metadata.
pub const DEFAULT_WS_APP_VER: &str = "1.48.2";
/// Default Windsurf language-server version sent in protocol metadata.
pub const DEFAULT_WS_LS_VER: &str = "1.9544.35";

const CONSTRAINED_ENV_NAMES: [&str; 9] = [
    FC_MAX_TURNS_ENV,
    FC_MAX_COMMANDS_ENV,
    FC_TIMEOUT_MS_ENV,
    SCOPE_SNAPSHOT_TREE_DEPTH_ENV,
    FC_RESULT_MAX_LINES_ENV,
    FC_READFILE_MAX_LINES_ENV,
    FC_LINE_MAX_CHARS_ENV,
    SWEGREP_DEBUG_ENV,
    SWEGREP_PATH_FILTER_ENV,
];

pub(crate) fn fc_max_turns() -> usize {
    read_env_range(FC_MAX_TURNS_ENV, DEFAULT_FC_MAX_TURNS, FC_MAX_TURNS_RANGE)
}

pub(crate) fn fc_max_commands() -> usize {
    read_env_range(
        FC_MAX_COMMANDS_ENV,
        DEFAULT_FC_MAX_COMMANDS,
        FC_MAX_COMMANDS_RANGE,
    )
}

pub(crate) fn fc_timeout_ms() -> u64 {
    read_env_u64_range(FC_TIMEOUT_MS_ENV, FC_TIMEOUT_MS_RANGE).unwrap_or(DEFAULT_FC_TIMEOUT_MS)
}

pub(crate) fn scope_snapshot_tree_depth() -> usize {
    read_env_range(
        SCOPE_SNAPSHOT_TREE_DEPTH_ENV,
        DEFAULT_SCOPE_SNAPSHOT_TREE_DEPTH,
        SCOPE_SNAPSHOT_TREE_DEPTH_RANGE,
    )
}

pub(crate) fn fc_result_max_lines(override_value: Option<usize>) -> usize {
    bounded_int(
        override_value,
        read_env_range(
            FC_RESULT_MAX_LINES_ENV,
            DEFAULT_FC_RESULT_MAX_LINES,
            FC_RESULT_MAX_LINES_RANGE,
        ),
        *FC_RESULT_MAX_LINES_RANGE.start(),
        *FC_RESULT_MAX_LINES_RANGE.end(),
    )
}

pub(crate) fn fc_readfile_max_lines() -> usize {
    read_env_range(
        FC_READFILE_MAX_LINES_ENV,
        DEFAULT_FC_READFILE_MAX_LINES,
        FC_READFILE_MAX_LINES_RANGE,
    )
}

pub(crate) fn fc_line_max_chars(override_value: Option<usize>) -> usize {
    bounded_int(
        override_value,
        read_env_range(
            FC_LINE_MAX_CHARS_ENV,
            DEFAULT_FC_LINE_MAX_CHARS,
            FC_LINE_MAX_CHARS_RANGE,
        ),
        *FC_LINE_MAX_CHARS_RANGE.start(),
        *FC_LINE_MAX_CHARS_RANGE.end(),
    )
}

pub(crate) fn ws_app_version(override_value: Option<&str>) -> String {
    protocol_setting(override_value, "WS_APP_VER", DEFAULT_WS_APP_VER)
}

pub(crate) fn ws_ls_version(override_value: Option<&str>) -> String {
    protocol_setting(override_value, "WS_LS_VER", DEFAULT_WS_LS_VER)
}

pub(crate) fn swegrep_debug_enabled() -> bool {
    read_bool_env(SWEGREP_DEBUG_ENV).unwrap_or(false)
}

pub(crate) fn swegrep_debug_log_path() -> PathBuf {
    env::var_os(SWEGREP_DEBUG_LOG_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(default_debug_log_path)
}

pub(crate) fn swegrep_path_filter_enabled() -> Option<bool> {
    read_bool_env(SWEGREP_PATH_FILTER_ENV)
}

pub(crate) fn swegrep_rg_path() -> Option<PathBuf> {
    env::var_os(SWEGREP_RG_PATH_ENV).map(PathBuf::from)
}

pub(crate) fn invalid_env_warnings() -> Vec<String> {
    CONSTRAINED_ENV_NAMES
        .iter()
        .filter_map(|name| {
            let raw = env::var_os(name)?;
            value_warning(name, &raw.to_string_lossy())
        })
        .collect()
}

fn value_warning(name: &str, raw: &str) -> Option<String> {
    let (valid, expected, default) = match name {
        FC_MAX_TURNS_ENV => (
            parse_env_range_value(raw, FC_MAX_TURNS_RANGE).is_some(),
            "an integer from 4 to 6",
            DEFAULT_FC_MAX_TURNS.to_string(),
        ),
        FC_MAX_COMMANDS_ENV => (
            parse_env_range_value(raw, FC_MAX_COMMANDS_RANGE).is_some(),
            "an integer from 1 to 8",
            DEFAULT_FC_MAX_COMMANDS.to_string(),
        ),
        FC_TIMEOUT_MS_ENV => (
            parse_env_u64_range_value(raw, FC_TIMEOUT_MS_RANGE).is_some(),
            "an integer from 1000 to 300000",
            DEFAULT_FC_TIMEOUT_MS.to_string(),
        ),
        SCOPE_SNAPSHOT_TREE_DEPTH_ENV => (
            parse_env_range_value(raw, SCOPE_SNAPSHOT_TREE_DEPTH_RANGE).is_some(),
            "an integer from 0 to 8",
            DEFAULT_SCOPE_SNAPSHOT_TREE_DEPTH.to_string(),
        ),
        FC_RESULT_MAX_LINES_ENV => (
            parse_env_range_value(raw, FC_RESULT_MAX_LINES_RANGE).is_some(),
            "an integer from 1 to 500",
            DEFAULT_FC_RESULT_MAX_LINES.to_string(),
        ),
        FC_READFILE_MAX_LINES_ENV => (
            parse_env_range_value(raw, FC_READFILE_MAX_LINES_RANGE).is_some(),
            "an integer from 1 to 10000",
            DEFAULT_FC_READFILE_MAX_LINES.to_string(),
        ),
        FC_LINE_MAX_CHARS_ENV => (
            parse_env_range_value(raw, FC_LINE_MAX_CHARS_RANGE).is_some(),
            "an integer from 20 to 10000",
            DEFAULT_FC_LINE_MAX_CHARS.to_string(),
        ),
        SWEGREP_DEBUG_ENV => (
            parse_bool_env(raw).is_some(),
            "1/0, true/false, yes/no, or on/off",
            "false".to_string(),
        ),
        SWEGREP_PATH_FILTER_ENV => (
            parse_bool_env(raw).is_some(),
            "1/0, true/false, yes/no, or on/off",
            "true".to_string(),
        ),
        _ => return None,
    };

    (!valid).then(|| {
        format!("Invalid {name} value {raw:?}; using default {default} (expected {expected})")
    })
}

fn default_debug_log_path() -> PathBuf {
    crate::core::get_config_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("debug.log")
}

fn protocol_setting(value: Option<&str>, env_name: &str, default: &str) -> String {
    value
        .map(ToOwned::to_owned)
        .or_else(|| env::var(env_name).ok())
        .unwrap_or_else(|| default.to_string())
}

fn parse_bool_env(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn read_bool_env(name: &str) -> Option<bool> {
    env::var(name).ok().and_then(|raw| parse_bool_env(&raw))
}

fn read_env_range(name: &str, default: usize, range: std::ops::RangeInclusive<usize>) -> usize {
    read_env_range_value(name, range).unwrap_or(default)
}

fn read_env_range_value(name: &str, range: std::ops::RangeInclusive<usize>) -> Option<usize> {
    env::var(name)
        .ok()
        .and_then(|raw| parse_env_range_value(&raw, range))
}

pub(crate) fn parse_env_range_value(
    raw: &str,
    range: std::ops::RangeInclusive<usize>,
) -> Option<usize> {
    raw.parse::<usize>()
        .ok()
        .filter(|value| range.contains(value))
}

fn read_env_u64_range(name: &str, range: std::ops::RangeInclusive<u64>) -> Option<u64> {
    env::var(name)
        .ok()
        .and_then(|raw| parse_env_u64_range_value(&raw, range))
}

pub(crate) fn parse_env_u64_range_value(
    raw: &str,
    range: std::ops::RangeInclusive<u64>,
) -> Option<u64> {
    raw.parse::<u64>()
        .ok()
        .filter(|value| range.contains(value))
}

pub(crate) fn parse_range(
    value: &str,
    range: std::ops::RangeInclusive<usize>,
    label: &str,
) -> Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("{label} must be an integer"))?;
    if range.contains(&parsed) {
        Ok(parsed)
    } else {
        Err(format!(
            "{label} must be between {} and {}",
            range.start(),
            range.end()
        ))
    }
}

fn bounded_int(value: Option<usize>, default: usize, min: usize, max: usize) -> usize {
    value.unwrap_or(default).clamp(min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constrained_environment_values_report_invalid_input() {
        let cases = [
            (FC_MAX_TURNS_ENV, "4", ["3", "not-a-number"]),
            (FC_MAX_COMMANDS_ENV, "8", ["9", "not-a-number"]),
            (FC_TIMEOUT_MS_ENV, "30000", ["999", "not-a-number"]),
            (SCOPE_SNAPSHOT_TREE_DEPTH_ENV, "0", ["9", "not-a-number"]),
            (FC_RESULT_MAX_LINES_ENV, "80", ["0", "not-a-number"]),
            (FC_READFILE_MAX_LINES_ENV, "400", ["10001", "not-a-number"]),
            (FC_LINE_MAX_CHARS_ENV, "300", ["19", "not-a-number"]),
            (SWEGREP_DEBUG_ENV, "true", ["sometimes", ""]),
            (SWEGREP_PATH_FILTER_ENV, "off", ["sometimes", ""]),
        ];

        for (name, valid, invalid_values) in cases {
            assert_eq!(value_warning(name, valid), None, "{name}");
            for invalid in invalid_values {
                let warning = value_warning(name, invalid).expect("invalid value should warn");
                assert!(warning.contains(name));
                assert!(warning.contains("using default"));
            }
        }
    }

    #[test]
    fn invalid_values_fall_back_to_documented_defaults() {
        assert_eq!(
            parse_env_range_value("3", FC_MAX_TURNS_RANGE).unwrap_or(DEFAULT_FC_MAX_TURNS),
            DEFAULT_FC_MAX_TURNS
        );
        assert_eq!(
            parse_env_range_value("invalid", FC_MAX_COMMANDS_RANGE)
                .unwrap_or(DEFAULT_FC_MAX_COMMANDS),
            DEFAULT_FC_MAX_COMMANDS
        );
        assert_eq!(
            parse_env_u64_range_value("999", FC_TIMEOUT_MS_RANGE).unwrap_or(DEFAULT_FC_TIMEOUT_MS),
            DEFAULT_FC_TIMEOUT_MS
        );
        assert_eq!(
            parse_env_range_value("9", SCOPE_SNAPSHOT_TREE_DEPTH_RANGE)
                .unwrap_or(DEFAULT_SCOPE_SNAPSHOT_TREE_DEPTH),
            DEFAULT_SCOPE_SNAPSHOT_TREE_DEPTH
        );
        assert_eq!(
            parse_env_range_value("0", FC_RESULT_MAX_LINES_RANGE)
                .unwrap_or(DEFAULT_FC_RESULT_MAX_LINES),
            DEFAULT_FC_RESULT_MAX_LINES
        );
        assert_eq!(
            parse_env_range_value("10001", FC_READFILE_MAX_LINES_RANGE)
                .unwrap_or(DEFAULT_FC_READFILE_MAX_LINES),
            DEFAULT_FC_READFILE_MAX_LINES
        );
        assert_eq!(
            parse_env_range_value("19", FC_LINE_MAX_CHARS_RANGE)
                .unwrap_or(DEFAULT_FC_LINE_MAX_CHARS),
            DEFAULT_FC_LINE_MAX_CHARS
        );
        assert!(!parse_bool_env("invalid").unwrap_or(false));
        assert!(parse_bool_env("invalid").unwrap_or(true));
    }

    #[test]
    fn boolean_values_accept_all_documented_spellings() {
        for raw in ["1", "true", "yes", "on"] {
            assert_eq!(parse_bool_env(raw), Some(true));
        }
        for raw in ["0", "false", "no", "off"] {
            assert_eq!(parse_bool_env(raw), Some(false));
        }
    }

    #[test]
    fn explicit_limits_are_clamped_independently_of_environment_defaults() {
        assert_eq!(bounded_int(Some(0), 80, 1, 500), 1);
        assert_eq!(bounded_int(Some(501), 80, 1, 500), 500);
        assert_eq!(bounded_int(None, 80, 1, 500), 80);
    }
}
