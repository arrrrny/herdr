//! CPU scheduling priority ("niceness") for agent launches.
//!
//! A launch may request a Unix scheduling priority. The priority becomes part
//! of the launch line, so the agent process inherits it before it forks any
//! child and every descendant keeps it.
//!
//! The range matches the operating system's own range. Raising priority above
//! the default needs elevated privileges; an unprivileged Herdr cannot grant
//! it and the operating system reports the refusal when the agent starts.

/// Lowest scheduling priority a launch may request (most aggressive).
pub(crate) const MIN: i32 = -20;
/// Highest scheduling priority a launch may request (most deferred).
pub(crate) const MAX: i32 = 19;

/// Returns whether `nice` is inside the supported range.
pub(crate) fn is_valid(nice: i32) -> bool {
    (MIN..=MAX).contains(&nice)
}

/// Builds the operator-facing message for a rejected priority.
pub(crate) fn invalid_message(value: &str) -> String {
    format!("invalid value for --nice: {value} (expected {MIN}..={MAX})")
}

/// Parses a requested scheduling priority, rejecting anything outside the range.
pub(crate) fn parse(value: &str) -> Result<i32, String> {
    let nice = value
        .trim()
        .parse::<i32>()
        .map_err(|_| invalid_message(value))?;
    if is_valid(nice) {
        Ok(nice)
    } else {
        Err(invalid_message(value))
    }
}

/// Reports whether a program is reachable through the search path.
///
/// The launch line runs in the pane's shell, which inherits the server's
/// environment, so the server's search path decides whether the program is
/// found there.
pub(crate) fn program_on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .any(|dir| is_executable(&dir.join(program)))
}

/// Reports whether a file exists and may be run.
fn is_executable(path: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::metadata(path)
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Resolves the effective scheduling priority for a launch.
///
/// A requested priority wins over the configured default, including an
/// explicit zero, which means "today's behavior" and therefore no prefix.
/// `None` means the launch line gains no priority prefix at all.
pub(crate) fn resolve(requested: Option<i32>, configured: i32) -> Option<i32> {
    let nice = requested.unwrap_or(configured);
    if nice == 0 {
        None
    } else {
        Some(nice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_supported_range_matches_the_operating_system_range() {
        assert!(is_valid(MIN));
        assert!(is_valid(0));
        assert!(is_valid(MAX));
        assert!(!is_valid(MIN - 1));
        assert!(!is_valid(MAX + 1));
    }

    #[test]
    fn an_out_of_range_value_is_rejected_and_never_clamped() {
        for value in ["20", "-21", "1000"] {
            let error = parse(value).expect_err("expected a rejection");
            assert!(
                error.contains(value),
                "message must name the value: {error}"
            );
            assert!(
                error.contains("-20..=19"),
                "message must name the range: {error}"
            );
        }
    }

    #[test]
    fn a_value_inside_the_range_is_accepted() {
        assert_eq!(parse("19"), Ok(19));
        assert_eq!(parse("0"), Ok(0));
        assert_eq!(parse("-20"), Ok(-20));
    }

    #[test]
    fn a_non_numeric_value_is_rejected() {
        assert!(parse("nineteen").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn a_requested_priority_overrides_the_configured_default() {
        assert_eq!(resolve(Some(5), 19), Some(5));
    }

    #[test]
    fn an_explicit_zero_overrides_the_configured_default() {
        assert_eq!(resolve(Some(0), 19), None);
    }

    #[test]
    fn an_absent_request_falls_back_to_the_configured_default() {
        assert_eq!(resolve(None, 19), Some(19));
        assert_eq!(resolve(None, 0), None);
    }

    #[test]
    fn a_program_that_is_not_on_the_search_path_is_reported_as_missing() {
        assert!(!program_on_path(
            "herdr-a-priority-program-that-does-not-exist"
        ));
    }
}
