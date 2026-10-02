//! Minimal logger: one line per record on stderr, with an ISO-8601 UTC
//! timestamp, the level and a short module name.
//!
//! ```text
//! 2026-10-01T12:34:56.123Z INFO  backend::sdrplay: GAIN: band=Band3 step=14/28 ...
//! ```
//!
//! The level comes from `--verbose` (debug) or, by default, `info`. The
//! `RUST_LOG` environment variable overrides it, with the usual syntax:
//! a global level (`debug`), per-module levels (`backend=trace,output=debug`),
//! or both (`warn,backend::sdrplay=debug`).

use log::{LevelFilter, Log, Metadata, Record};
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

/// Parsed `RUST_LOG`-style filter.
#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    default: LevelFilter,
    directives: Vec<(String, LevelFilter)>,
}

impl Filter {
    /// Parses a filter specification. Unknown words are ignored.
    pub fn parse(spec: &str, default: LevelFilter) -> Filter {
        let mut filter = Filter {
            default,
            directives: Vec::new(),
        };

        for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            match part.split_once('=') {
                Some((target, level)) => {
                    if let Ok(level) = level.trim().parse::<LevelFilter>() {
                        filter.directives.push((target.trim().to_string(), level));
                    }
                }
                None => match part.parse::<LevelFilter>() {
                    Ok(level) => filter.default = level,
                    // A bare word that is not a level is a module name:
                    // enable everything for it.
                    Err(_) => filter
                        .directives
                        .push((part.to_string(), LevelFilter::Trace)),
                },
            }
        }

        // Longest module name first, so the most specific directive wins.
        filter
            .directives
            .sort_by_key(|(target, _)| std::cmp::Reverse(target.len()));

        filter
    }

    /// Most verbose level that can be enabled anywhere.
    pub fn max_level(&self) -> LevelFilter {
        self.directives
            .iter()
            .map(|(_, level)| *level)
            .chain(std::iter::once(self.default))
            .max()
            .unwrap_or(self.default)
    }

    /// Level that applies to a module path such as `sdr_universal::backend::mock`.
    pub fn level_for(&self, target: &str) -> LevelFilter {
        let short = short_target(target);

        for (prefix, level) in &self.directives {
            if short == prefix
                || short.starts_with(&format!("{}::", prefix))
                || target == prefix
                || target.starts_with(&format!("{}::", prefix))
            {
                return *level;
            }
        }

        self.default
    }
}

/// `sdr_universal::backend::sdrplay` -> `backend::sdrplay`.
fn short_target(target: &str) -> &str {
    target.strip_prefix("sdr_universal::").unwrap_or(target)
}

struct Logger {
    filter: Filter,
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.filter.level_for(metadata.target())
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();

        let line = format!(
            "{} {:<5} {}: {}\n",
            format_timestamp(now.as_secs(), now.subsec_millis()),
            record.level(),
            short_target(record.target()),
            record.args()
        );

        // One write per record keeps lines intact when several threads log.
        let _ = std::io::stderr().lock().write_all(line.as_bytes());
    }

    fn flush(&self) {}
}

/// Installs the logger. `default` applies when `RUST_LOG` is not set.
pub fn init(default: LevelFilter) {
    let filter = match std::env::var("RUST_LOG") {
        Ok(spec) if !spec.trim().is_empty() => Filter::parse(&spec, default),
        _ => Filter::parse("", default),
    };

    log::set_max_level(filter.max_level());
    // Ignore the error: a logger may already be installed (tests, embedding).
    let _ = log::set_boxed_logger(Box::new(Logger { filter }));
}

/// `2026-10-01T12:34:56.123Z` from a Unix time.
pub fn format_timestamp(secs: u64, millis: u32) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (year, month, day) = civil_from_days(days);

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year,
        month,
        day,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60,
        millis
    )
}

/// Days since 1970-01-01 -> (year, month, day), proleptic Gregorian calendar
/// (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);

    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_are_correct() {
        assert_eq!(format_timestamp(0, 0), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            format_timestamp(1_700_000_000, 5),
            "2023-11-14T22:13:20.005Z"
        );
        // Leap days.
        assert_eq!(format_timestamp(951_782_400, 0), "2000-02-29T00:00:00.000Z");
        assert_eq!(
            format_timestamp(1_709_164_800, 999),
            "2024-02-29T00:00:00.999Z"
        );
        // Day after a leap day, and a year boundary.
        assert_eq!(
            format_timestamp(1_709_251_200, 0),
            "2024-03-01T00:00:00.000Z"
        );
        assert_eq!(
            format_timestamp(1_767_225_599, 0),
            "2025-12-31T23:59:59.000Z"
        );
        assert_eq!(
            format_timestamp(1_767_225_600, 0),
            "2026-01-01T00:00:00.000Z"
        );
    }

    #[test]
    fn global_level_only() {
        let f = Filter::parse("debug", LevelFilter::Info);
        assert_eq!(
            f.level_for("sdr_universal::backend::mock"),
            LevelFilter::Debug
        );
        assert_eq!(f.max_level(), LevelFilter::Debug);
    }

    #[test]
    fn default_level_without_spec() {
        let f = Filter::parse("", LevelFilter::Info);
        assert_eq!(f.level_for("anything"), LevelFilter::Info);
        assert_eq!(f.max_level(), LevelFilter::Info);
    }

    #[test]
    fn per_module_levels_and_most_specific_wins() {
        let f = Filter::parse(
            "warn,backend=debug,backend::sdrplay=trace",
            LevelFilter::Info,
        );

        assert_eq!(f.level_for("sdr_universal::main"), LevelFilter::Warn);
        assert_eq!(
            f.level_for("sdr_universal::backend::mock"),
            LevelFilter::Debug
        );
        assert_eq!(
            f.level_for("sdr_universal::backend::sdrplay"),
            LevelFilter::Trace
        );
        assert_eq!(f.max_level(), LevelFilter::Trace);
    }

    #[test]
    fn prefix_must_match_whole_module_names() {
        let f = Filter::parse("back=trace", LevelFilter::Info);
        assert_eq!(
            f.level_for("sdr_universal::backend::mock"),
            LevelFilter::Info
        );
    }

    #[test]
    fn bare_module_name_enables_everything_for_it() {
        let f = Filter::parse("output", LevelFilter::Warn);
        assert_eq!(
            f.level_for("sdr_universal::output::rtltcp"),
            LevelFilter::Trace
        );
        assert_eq!(
            f.level_for("sdr_universal::core::receiver"),
            LevelFilter::Warn
        );
    }

    #[test]
    fn invalid_levels_are_ignored() {
        let f = Filter::parse("backend=loud,info", LevelFilter::Error);
        assert_eq!(f.level_for("sdr_universal::backend"), LevelFilter::Info);
    }
}
