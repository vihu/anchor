//! Subtitle languages as OpenSubtitles names them (ISO 639-2, and `pob`
//! for Brazilian Portuguese), with their names in English.

/// Every language Settings offers, by code and name.
pub const LANGUAGES: [(&str, &str); 31] = [
    ("eng", "English"),
    ("fre", "French"),
    ("spa", "Spanish"),
    ("ger", "German"),
    ("ita", "Italian"),
    ("por", "Portuguese"),
    ("pob", "Portuguese (Brazil)"),
    ("dut", "Dutch"),
    ("swe", "Swedish"),
    ("nor", "Norwegian"),
    ("dan", "Danish"),
    ("fin", "Finnish"),
    ("pol", "Polish"),
    ("cze", "Czech"),
    ("hun", "Hungarian"),
    ("rum", "Romanian"),
    ("gre", "Greek"),
    ("tur", "Turkish"),
    ("rus", "Russian"),
    ("ukr", "Ukrainian"),
    ("ara", "Arabic"),
    ("heb", "Hebrew"),
    ("per", "Persian"),
    ("hin", "Hindi"),
    ("tha", "Thai"),
    ("vie", "Vietnamese"),
    ("ind", "Indonesian"),
    ("may", "Malay"),
    ("jpn", "Japanese"),
    ("kor", "Korean"),
    ("chi", "Chinese"),
];

/// The name of language `code`, or the code when it is not one of them.
pub fn name(code: &str) -> &str {
    LANGUAGES
        .iter()
        .find(|(c, _)| *c == code)
        .map_or(code, |(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_by_code() {
        assert_eq!(name("fre"), "French");
        assert_eq!(name("pob"), "Portuguese (Brazil)");
        assert_eq!(name("xyz"), "xyz");
        let mut codes: Vec<&str> = LANGUAGES.iter().map(|(c, _)| *c).collect();
        codes.dedup();
        assert_eq!(codes.len(), LANGUAGES.len(), "no code twice");
    }
}
