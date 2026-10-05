//! A movie's cast and details, below its title: who is in it, who made
//! it, when and where.

use anchor::addon::{CastMember, MetaPreview};
use jiff::Timestamp;
use slint::Image;

use crate::ui::{CastItem, Fact};

/// The cast a movie's page shows: AIOMetadata's, with parts and pictures,
/// else the names in its links.
pub(super) fn cast(preview: &MetaPreview) -> Vec<CastMember> {
    if !preview.app_extras.cast.is_empty() {
        return preview.app_extras.cast.clone();
    }
    preview
        .people("Cast")
        .into_iter()
        .map(|name| CastMember {
            name: name.to_owned(),
            ..CastMember::default()
        })
        .collect()
}

pub(super) fn cast_item(person: &CastMember) -> CastItem {
    CastItem {
        name: person.name.as_str().into(),
        role: person.character.as_deref().unwrap_or_default().into(),
        initials: initials(&person.name).into(),
        photo: Image::default(),
        has_photo: false,
    }
}

/// `TL` for `Teo Larsen`: the first letters of the first and last names.
fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let last = words.last().filter(|_| words.len() > 1);
    [words.first(), last]
        .into_iter()
        .flatten()
        .filter_map(|w| w.chars().next())
        .flat_map(char::to_uppercase)
        .collect()
}

/// A movie's details: who made it, when and where. Two columns on the page.
pub(super) fn facts(preview: &MetaPreview) -> Vec<Fact> {
    let names = |category: &str| {
        let mut names = preview.people(category);
        let mut seen = std::collections::HashSet::new();
        names.retain(|n| seen.insert(*n));
        names.join(", ")
    };
    let released = preview
        .released
        .as_deref()
        .and_then(|r| r.parse::<Timestamp>().ok())
        .map(|t| t.strftime("%-d %B %Y").to_string());
    // TVDB sends a code, for example `usa`.
    let country = preview.country.as_deref().map(|c| {
        if c.len() <= 3 && c.chars().all(|ch| ch.is_ascii_lowercase()) {
            c.to_ascii_uppercase()
        } else {
            c.to_owned()
        }
    });
    [
        ("Directed by", names("Directors")),
        ("Written by", names("Writers")),
        ("Released", released.unwrap_or_default()),
        ("Country", country.unwrap_or_default()),
    ]
    .into_iter()
    .filter(|(_, value)| !value.is_empty())
    .map(|(label, value)| Fact {
        label: label.into(),
        value: value.into(),
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_movies_details_and_cast_from_the_links() {
        let preview: MetaPreview = serde_json::from_value(serde_json::json!({
            "id": "tt9", "type": "movie", "name": "X",
            "released": "2025-03-14T00:00:00.000Z", "country": "usa",
            "links": [
                {"name": "Ana Dimas", "category": "Directors"},
                {"name": "Ana Dimas", "category": "Writers"},
                {"name": "Ines Varga", "category": "Writers"},
                {"name": "Ana Dimas", "category": "Writers"},
                {"name": "Teo Larsen", "category": "Cast"}
            ]
        }))
        .unwrap();
        let facts: Vec<(String, String)> = facts(&preview)
            .iter()
            .map(|f| (f.label.to_string(), f.value.to_string()))
            .collect();
        assert_eq!(
            facts,
            [
                ("Directed by".to_owned(), "Ana Dimas".to_owned()),
                ("Written by".to_owned(), "Ana Dimas, Ines Varga".to_owned()),
                ("Released".to_owned(), "14 March 2025".to_owned()),
                ("Country".to_owned(), "USA".to_owned()),
            ]
        );
        let people = cast(&preview);
        assert_eq!(people.len(), 1);
        assert_eq!(cast_item(&people[0]).initials, "TL");
        assert_eq!(initials("Cher"), "C");
        assert_eq!(initials("Mary Elizabeth Winstead"), "MW");
    }
}
