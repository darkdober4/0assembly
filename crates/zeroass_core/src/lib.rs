//! `0ass` — cœur : langages, scan multi-langages, manifeste, stratégies.
//!
//! Le cœur ne touche jamais au projet analysé : il lit, analyse et décrit.

pub mod lang;
pub mod manifest;
pub mod profile;
pub mod scan;
pub mod strategy;
pub mod summary;

/// Version du cœur.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Horodatage ISO-8601 approximatif (UTC, sans dépendance externe).
pub fn now_iso8601() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Conversion грубо simplifiée : jours depuis 1970 -> date (algorithme de Howard Hinnant).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if mth <= 2 { y + 1 } else { y };
    format!("{year:04}-{mth:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Date compacte (`YYYY-MM-DD`) pour nommer les campagnes.
pub fn today_compact() -> String {
    now_iso8601()
        .split('T')
        .next()
        .unwrap_or("0000-00-00")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_has_expected_shape() {
        let t = now_iso8601();
        assert_eq!(t.len(), 20, "{t}");
        assert!(t.ends_with('Z'));
        assert_eq!(t.matches('-').count(), 2);
    }

    #[test]
    fn date_matches_timestamp() {
        assert_eq!(today_compact().len(), 10);
    }
}
