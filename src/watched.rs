//! Which episodes of a series were watched: Stremio's watched bitfield,
//! ported from stremio-core (`stremio-watched-bitfield`).
//!
//! A library item keeps it as `{anchor video id}:{anchor length}:{bits}`,
//! where the bits are zlib-compressed and base64-encoded, one per video in
//! the order of [`MetaItem::bitfield_ids`][crate::addon::MetaItem::bitfield_ids],
//! the lowest bit of each byte first. The anchor names the last video
//! watched, so the bits still line up after episodes are added before it.

use std::io::{Read, Write};

use base64::Engine;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use base64::engine::{DecodePaddingMode, general_purpose};
use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;

/// The compression level stremio-core writes with.
const LEVEL: u32 = 6;
/// The anchor written when there are no videos.
const NO_VIDEO: &str = "undefined";
/// Standard base64 that reads with or without padding.
const BASE64_ANY_PADDING: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// Which of a title's videos were watched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Watched {
    bits: Bits,
    video_ids: Vec<String>,
}

/// A growable run of bits, the lowest of each byte first.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Bits {
    length: usize,
    bytes: Vec<u8>,
}

/// What can go wrong reading a watched field.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// It does not have the `{id}:{length}:{bits}` shape.
    Shape(String),
    /// Its bits do not decode.
    Bits(String),
}

// Public API
impl Watched {
    /// None of `video_ids` watched.
    pub fn none(video_ids: Vec<String>) -> Self {
        Self {
            bits: Bits::new(video_ids.len()),
            video_ids,
        }
    }

    /// Reads `field` against the title's videos as they are now; a field
    /// whose anchor video is gone reads as nothing watched.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] when `field` is not a watched field.
    pub fn parse(field: &str, video_ids: Vec<String>) -> Result<Self, Error> {
        let mut parts: Vec<&str> = field.split(':').collect();
        if parts.len() < 3 {
            return Err(Error::Shape(field.to_owned()));
        }
        let encoded = parts.pop().expect("there are three parts");
        let anchor_length: usize = parts
            .pop()
            .expect("there are three parts")
            .parse()
            .map_err(|_| Error::Shape(field.to_owned()))?;
        let anchor = parts.join(":");
        let bits = Bits::decode(encoded)?;

        let Some(anchor_index) = video_ids.iter().position(|id| *id == anchor) else {
            return Ok(Self::none(video_ids));
        };
        // How far the videos moved since the field was written.
        let offset = anchor_length as isize - anchor_index as isize - 1;
        let old = Bits::with_bytes(bits.bytes, video_ids.len());
        if offset == 0 {
            return Ok(Self {
                bits: old,
                video_ids,
            });
        }
        let mut watched = Self::none(video_ids);
        for i in 0..watched.video_ids.len() {
            let before = i as isize + offset;
            if before >= 0 && (before as usize) < old.length {
                watched.bits.set(i, old.get(before as usize));
            }
        }
        Ok(watched)
    }

    /// Whether video `id` was watched.
    pub fn get(&self, id: &str) -> bool {
        self.video_ids
            .iter()
            .position(|v| v == id)
            .is_some_and(|i| self.bits.get(i))
    }

    /// Marks video `id` watched, or not; an id not among the videos is
    /// ignored.
    pub fn set(&mut self, id: &str, watched: bool) {
        if let Some(i) = self.video_ids.iter().position(|v| v == id) {
            self.bits.set(i, watched);
        }
    }

    /// The field as a library item keeps it.
    pub fn serialize(&self) -> String {
        let last = self.bits.last_set().unwrap_or(0);
        let anchor = self.video_ids.get(last).map_or(NO_VIDEO, String::as_str);
        format!("{anchor}:{}:{}", last + 1, self.bits.encode())
    }
}

impl std::error::Error for Error {}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::Shape(field) => write!(f, "not a watched field, got {field}"),
            Error::Bits(why) => write!(f, "the watched bits do not decode: {why}"),
        }
    }
}

// Private API
impl Bits {
    fn new(length: usize) -> Self {
        Self {
            length,
            bytes: vec![0; length.div_ceil(8)],
        }
    }

    fn with_bytes(mut bytes: Vec<u8>, length: usize) -> Self {
        if bytes.len() < length.div_ceil(8) {
            bytes.resize(length.div_ceil(8), 0);
        }
        Self { length, bytes }
    }

    fn get(&self, i: usize) -> bool {
        self.bytes
            .get(i / 8)
            .is_some_and(|byte| (byte >> (i % 8)) & 1 != 0)
    }

    fn set(&mut self, i: usize, value: bool) {
        let (index, mask) = (i / 8, 1 << (i % 8));
        if index >= self.bytes.len() {
            self.bytes.resize(index + 1, 0);
            self.length = self.bytes.len() * 8;
        }
        if value {
            self.bytes[index] |= mask;
        } else {
            self.bytes[index] &= !mask;
        }
    }

    fn last_set(&self) -> Option<usize> {
        (0..self.length).rev().find(|&i| self.get(i))
    }

    fn decode(encoded: &str) -> Result<Self, Error> {
        let compressed = BASE64_ANY_PADDING
            .decode(encoded)
            .map_err(|e| Error::Bits(e.to_string()))?;
        let mut bytes = Vec::new();
        ZlibDecoder::new(compressed.as_slice())
            .read_to_end(&mut bytes)
            .map_err(|e| Error::Bits(e.to_string()))?;
        let length = bytes.len() * 8;
        Ok(Self { length, bytes })
    }

    fn encode(&self) -> String {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(LEVEL));
        encoder
            .write_all(&self.bytes)
            .expect("writing to memory does not fail");
        let compressed = encoder.finish().expect("writing to memory does not fail");
        general_purpose::STANDARD.encode(compressed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// stremio-core's test vector: videos 1 to 5 of nine watched.
    const FIELD: &str = "tt2934286:1:5:5:eJyTZwAAAEAAIA==";

    fn episodes(range: std::ops::RangeInclusive<u32>) -> Vec<String> {
        range.map(|e| format!("tt2934286:1:{e}")).collect()
    }

    #[test]
    fn reads_and_writes_cores_vector() {
        let watched = Watched::parse(FIELD, episodes(1..=9)).unwrap();
        for e in 1..=5 {
            assert!(watched.get(&format!("tt2934286:1:{e}")), "episode {e}");
        }
        for e in 6..=9 {
            assert!(!watched.get(&format!("tt2934286:1:{e}")), "episode {e}");
        }
        assert_eq!(watched.serialize(), FIELD);
    }

    #[test]
    fn episodes_added_before_the_anchor_shift_the_bits() {
        let mut ids = vec!["tt2934286:0:1".to_owned(), "tt2934286:0:2".to_owned()];
        ids.extend(episodes(1..=9));
        let watched = Watched::parse(FIELD, ids).unwrap();
        assert!(!watched.get("tt2934286:0:1") && !watched.get("tt2934286:0:2"));
        for e in 1..=5 {
            assert!(watched.get(&format!("tt2934286:1:{e}")), "episode {e}");
        }
        assert!(!watched.get("tt2934286:1:6"));
    }

    #[test]
    fn a_missing_anchor_reads_as_nothing_watched() {
        let watched = Watched::parse(FIELD, vec!["other:1:1".to_owned()]).unwrap();
        assert!(!watched.get("other:1:1"));
    }

    #[test]
    fn cores_other_vectors() {
        // Seasons 1 and 2 before it: S3 E8 is the 24th video, all watched.
        let ids: Vec<String> = (1..=3)
            .flat_map(|s| (1..=8).map(move |e| format!("tt7767422:{s}:{e}")))
            .collect();
        let watched = Watched::parse("tt7767422:3:8:24:eJz7//8/AAX9Av4=", ids.clone()).unwrap();
        assert!(ids.iter().all(|id| watched.get(id)));
        assert_eq!(watched.serialize(), "tt7767422:3:8:24:eJz7//8/AAX9Av4=");
        assert_eq!(
            Watched::parse("undefined:1:eJwDAAAAAAE=", Vec::new()).unwrap(),
            Watched::none(Vec::new())
        );
        assert_eq!(
            Watched::none(Vec::new()).serialize(),
            "undefined:1:eJwDAAAAAAE="
        );
    }

    #[test]
    fn set_and_serialize_round_trip() {
        let ids = episodes(1..=12);
        let mut watched = Watched::none(ids.clone());
        watched.set("tt2934286:1:3", true);
        watched.set("tt2934286:1:10", true);
        watched.set("not one of them", true);
        let field = watched.serialize();
        assert!(field.starts_with("tt2934286:1:10:10:"), "{field}");
        let again = Watched::parse(&field, ids).unwrap();
        assert!(again.get("tt2934286:1:3") && again.get("tt2934286:1:10"));
        assert!(!again.get("tt2934286:1:4"));
        watched.set("tt2934286:1:10", false);
        assert!(watched.serialize().starts_with("tt2934286:1:3:3:"));
    }

    #[test]
    fn bad_fields_are_errors() {
        assert!(matches!(
            Watched::parse("tt1:5", vec![]),
            Err(Error::Shape(_))
        ));
        assert!(matches!(
            Watched::parse("tt1:x:abc", vec![]),
            Err(Error::Shape(_))
        ));
        assert!(matches!(
            Watched::parse("tt1:1:5:!!!", vec![]),
            Err(Error::Bits(_))
        ));
        // Unpadded base64 reads too.
        assert!(Watched::parse("tt2934286:1:5:5:eJyTZwAAAEAAIA", episodes(1..=9)).is_ok());
    }
}
