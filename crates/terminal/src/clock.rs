use std::process::Command;

const MAX_CLOCK_OFFSET_SECONDS: i64 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockSyncCheck {
    pub is_synced: bool,
    pub reason: Option<String>,
}

impl ClockSyncCheck {
    fn synced() -> Self {
        Self {
            is_synced: true,
            reason: None,
        }
    }

    fn unsynced(reason: impl Into<String>) -> Self {
        Self {
            is_synced: false,
            reason: Some(reason.into()),
        }
    }
}

pub fn is_server_offset_synced(offset_seconds: i64) -> bool {
    offset_seconds <= MAX_CLOCK_OFFSET_SECONDS
}

pub fn check_os_clock_sync() -> ClockSyncCheck {
    match run_platform_clock_command() {
        Ok(output) => parse_platform_clock_sync(&output),
        Err(e) => ClockSyncCheck::unsynced(format!("failed to check OS clock sync: {e}")),
    }
}

#[cfg(target_os = "linux")]
fn run_platform_clock_command() -> Result<String, std::io::Error> {
    let output = Command::new("timedatectl").arg("status").output()?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "macos")]
fn run_platform_clock_command() -> Result<String, std::io::Error> {
    let output = Command::new("sntp")
        .arg("-s")
        .arg("time.apple.com")
        .output()?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(text)
}

#[cfg(target_os = "windows")]
fn run_platform_clock_command() -> Result<String, std::io::Error> {
    let output = Command::new("w32tm").args(["/query", "/status"]).output()?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn run_platform_clock_command() -> Result<String, std::io::Error> {
    Ok(String::new())
}

#[cfg(target_os = "linux")]
fn parse_platform_clock_sync(output: &str) -> ClockSyncCheck {
    parse_timedatectl_sync(output)
}

#[cfg(target_os = "macos")]
fn parse_platform_clock_sync(output: &str) -> ClockSyncCheck {
    parse_sntp_sync(output)
}

#[cfg(target_os = "windows")]
fn parse_platform_clock_sync(output: &str) -> ClockSyncCheck {
    parse_w32tm_sync(output)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn parse_platform_clock_sync(_output: &str) -> ClockSyncCheck {
    ClockSyncCheck::synced()
}

#[cfg(any(test, target_os = "linux"))]
pub fn parse_timedatectl_sync(output: &str) -> ClockSyncCheck {
    for line in output.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.trim() == "System clock synchronized" {
            return if value.trim().eq_ignore_ascii_case("yes") {
                ClockSyncCheck::synced()
            } else {
                ClockSyncCheck::unsynced("system clock is not synchronized")
            };
        }
    }

    ClockSyncCheck::unsynced("System clock synchronized field is missing")
}

#[cfg(any(test, target_os = "macos"))]
pub fn parse_sntp_sync(output: &str) -> ClockSyncCheck {
    match parse_sntp_offset_seconds(output) {
        Some(offset) if offset.abs() <= MAX_CLOCK_OFFSET_SECONDS as f64 => ClockSyncCheck::synced(),
        Some(offset) => ClockSyncCheck::unsynced(format!("sntp offset exceeds limit: {offset}")),
        None => ClockSyncCheck::unsynced("sntp offset is missing"),
    }
}

#[cfg(any(test, target_os = "macos"))]
pub fn parse_sntp_offset_seconds(output: &str) -> Option<f64> {
    let tokens: Vec<&str> = output.split_whitespace().collect();
    tokens.windows(2).find_map(|window| {
        if window[0].eq_ignore_ascii_case("offset") {
            window[1].parse::<f64>().ok()
        } else {
            None
        }
    })
}

#[cfg(any(test, target_os = "windows"))]
pub fn parse_w32tm_sync(output: &str) -> ClockSyncCheck {
    for line in output.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("Leap Indicator") {
            let value = value.trim();
            let normalized = value.to_ascii_lowercase();
            return if value.starts_with('3')
                || normalized.contains("not synchronized")
                || normalized.contains("unsynchronized")
            {
                ClockSyncCheck::unsynced("windows time service is unsynchronized")
            } else {
                ClockSyncCheck::synced()
            };
        }
    }

    ClockSyncCheck::unsynced("Leap Indicator field is missing")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // Linux の timedatectl が yes を返す場合は OS 時刻同期済みと判定する。
    fn treats_timedatectl_yes_as_synced() {
        let output = "               Local time: Tue 2026-04-28 10:00:00 JST\n System clock synchronized: yes\n";

        let result = parse_timedatectl_sync(output);

        assert!(result.is_synced);
    }

    #[test]
    // Linux の timedatectl が no を返す場合は OS 時刻未同期と判定する。
    fn treats_timedatectl_no_as_unsynced() {
        let output = " System clock synchronized: no\n";

        let result = parse_timedatectl_sync(output);

        assert!(!result.is_synced);
    }

    #[test]
    // macOS の sntp 出力から offset 秒を読み取る。
    fn parses_sntp_offset_seconds() {
        let output =
            "2026-04-28 10:00:00.000000 (+0900) +0.00042 +/- 0.001 offset -0.125 delay 0.010\n";

        let result = parse_sntp_offset_seconds(output);

        assert_eq!(result, Some(-0.125));
    }

    #[test]
    // macOS の sntp offset が10秒以内なら OS 時刻同期済みと判定する。
    fn treats_sntp_offset_within_limit_as_synced() {
        let output = "server time offset 9.999 delay 0.010\n";

        let result = parse_sntp_sync(output);

        assert!(result.is_synced);
    }

    #[test]
    // macOS の sntp offset が10秒を超えたら OS 時刻未同期と判定する。
    fn treats_sntp_offset_over_limit_as_unsynced() {
        let output = "server time offset -10.001 delay 0.010\n";

        let result = parse_sntp_sync(output);

        assert!(!result.is_synced);
    }

    #[test]
    // Windows の Leap Indicator が unsynchronized でなければ OS 時刻同期済みと判定する。
    fn treats_w32tm_non_unsynchronized_leap_indicator_as_synced() {
        let output =
            "Leap Indicator: 0(no warning)\nStratum: 3 (secondary reference - syncd by (S)NTP)\n";

        let result = parse_w32tm_sync(output);

        assert!(result.is_synced);
    }

    #[test]
    // Windows の Leap Indicator が3なら OS 時刻未同期と判定する。
    fn treats_w32tm_leap_indicator_three_as_unsynced() {
        let output = "Leap Indicator: 3(not synchronized)\n";

        let result = parse_w32tm_sync(output);

        assert!(!result.is_synced);
    }

    #[test]
    // Server-Time 差分が10秒以内なら同期済み扱いにする。
    fn treats_server_offset_within_limit_as_synced() {
        assert!(is_server_offset_synced(10));
    }

    #[test]
    // Server-Time 差分が10秒を超えたら未同期扱いにする。
    fn treats_server_offset_over_limit_as_unsynced() {
        assert!(!is_server_offset_synced(11));
    }
}
