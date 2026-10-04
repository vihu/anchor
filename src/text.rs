//! Words and numbers as the screens show them: time left, lengths, episode
//! codes, years, and a stable tint for titles without a picture.

use slint::Color;

/// Milliseconds in a minute.
const MINUTE_MS: u64 = 60_000;

/// What is left of a video, for example `18 min left` or `1 h 2 min left`;
/// empty when nothing is known or nothing is left.
pub fn left(offset_ms: u64, duration_ms: u64) -> String {
    if duration_ms == 0 || offset_ms >= duration_ms {
        return String::new();
    }
    format!("{} left", length(duration_ms - offset_ms))
}

/// A length, for example `47 min` or `2 h 4 min`.
pub fn length(ms: u64) -> String {
    let minutes = ms.div_ceil(MINUTE_MS);
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// A runtime as addons write it (`148 min`, `2h 28m`), the way anchor
/// writes lengths; as it was when it does not read.
pub fn runtime(text: &str) -> String {
    let digits: Vec<u64> = text
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|n| n.parse().ok())
        .collect();
    let lower = text.to_lowercase();
    let minutes = match digits.as_slice() {
        [h, m] if lower.contains('h') => h * 60 + m,
        [h] if lower.contains('h') && !lower.contains("min") => h * 60,
        [m] => *m,
        _ => return text.to_owned(),
    };
    length(minutes * MINUTE_MS)
}

/// Season and episode from a video id such as `tt0903747:2:4`.
pub fn episode(video_id: &str) -> Option<(u32, u32)> {
    let mut parts = video_id.rsplit(':');
    let episode = parts.next()?.parse().ok()?;
    let season = parts.next()?.parse().ok()?;
    parts.next()?;
    Some((season, episode))
}

/// `S2 E4`.
pub fn code(season: u32, episode: u32) -> String {
    format!("S{season} E{episode}")
}

/// The year a title came out, from its release info (`2010`, `2008-2013`).
pub fn year(release_info: Option<&str>) -> String {
    release_info
        .unwrap_or_default()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect()
}

/// A dark tint from `id`, the same every time, for a title without a
/// picture.
pub fn tint(id: &str) -> Color {
    // FNV-1a: stable across runs, unlike the std hasher.
    let hash = id.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    let channel = |shift: u32| 0x30 + ((hash >> shift) & 0x3f) as u8;
    Color::from_rgb_u8(channel(0), channel(8), channel(16))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_left() {
        assert_eq!(left(30 * 60_000, 48 * 60_000), "18 min left");
        assert_eq!(left(0, 62 * 60_000 + 1), "1 h 3 min left");
        assert_eq!(left(10, 0), "");
        assert_eq!(left(50, 50), "");
    }

    #[test]
    fn lengths_and_runtimes() {
        assert_eq!(length(47 * 60_000), "47 min");
        assert_eq!(length(124 * 60_000), "2 h 4 min");
        assert_eq!(length(120 * 60_000), "2 h");
        assert_eq!(runtime("148 min"), "2 h 28 min");
        assert_eq!(runtime("2h 28m"), "2 h 28 min");
        assert_eq!(runtime("1h"), "1 h");
        assert_eq!(runtime("45"), "45 min");
        assert_eq!(runtime("about an hour"), "about an hour");
    }

    #[test]
    fn episodes_from_ids() {
        assert_eq!(episode("tt0903747:2:4"), Some((2, 4)));
        assert_eq!(episode("tmdb:1399:1:10"), Some((1, 10)));
        assert_eq!(episode("tt1375666"), None);
        assert_eq!(episode("kitsu:1:x"), None);
        assert_eq!(code(2, 4), "S2 E4");
    }

    #[test]
    fn years() {
        assert_eq!(year(Some("2008-2013")), "2008");
        assert_eq!(year(Some("2025")), "2025");
        assert_eq!(year(None), "");
    }

    #[test]
    fn tints_are_stable_and_dark() {
        assert_eq!(tint("tt1"), tint("tt1"));
        assert_ne!(tint("tt1"), tint("tt2"));
        let c = tint("anything");
        assert!(c.red() < 0x70 && c.green() < 0x70 && c.blue() < 0x70);
    }
}
