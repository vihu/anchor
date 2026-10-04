//! The user's settings: kept in `settings.json` in the config directory.
//!
//! Every field has a default and unknown fields are ignored, so a file from
//! an older or newer anchor, or one edited by hand, still loads.

use serde::{Deserialize, Serialize};

/// Everything the Settings screen and the sidebar change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Where mpv is; empty to look for it on the `PATH`.
    pub mpv_path: String,
    /// Arguments added to every mpv launch, as a shell would split them.
    pub mpv_args: String,
    /// Pass subtitles from subtitle addons to mpv.
    pub addon_subtitles: bool,
    /// The languages of the addon subtitles passed, as OpenSubtitles names
    /// them (ISO 639-2, for example `eng`), in order of preference.
    pub subtitle_languages: Vec<String>,
    /// The sidebar with labels, or icons only.
    pub sidebar: Sidebar,
    /// Sidebar sections folded away: `movie`, `series`.
    pub folded: Vec<String>,
}

/// How the sidebar shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sidebar {
    /// Icons with labels, and the catalogs.
    Expanded,
    /// Icons only.
    Collapsed,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mpv_path: String::new(),
            mpv_args: String::new(),
            addon_subtitles: true,
            subtitle_languages: vec!["eng".to_owned()],
            sidebar: Sidebar::Expanded,
            folded: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip() {
        let settings = Settings {
            mpv_args: "--fs".into(),
            subtitle_languages: vec!["fre".into(), "eng".into()],
            sidebar: Sidebar::Collapsed,
            folded: vec!["series".into()],
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert!(json.contains(r#""sidebar":"collapsed""#), "{json}");
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), settings);
    }

    #[test]
    fn partial_and_unknown_fields_load() {
        let settings: Settings =
            serde_json::from_str(r#"{"mpv_args":"--fs","from_the_future":1}"#).unwrap();
        assert_eq!(settings.mpv_args, "--fs");
        assert!(settings.addon_subtitles, "defaulted");
        assert_eq!(
            serde_json::from_str::<Settings>("{}").unwrap(),
            Settings::default()
        );
    }
}
