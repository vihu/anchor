//! Posters, backdrops and episode stills: downloaded, decoded and scaled on
//! worker threads, then handed to the UI thread as pixel buffers (itele's
//! `art.rs`).
//!
//! Nothing here keeps a decoded picture. The screens hold the pictures of
//! the titles near the visible ones and drop the rest, so scrolling through
//! long catalogs keeps memory flat; a picture scrolled back
//! into view is decoded again from the disk cache.

use std::collections::HashSet;
use std::fs;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use anchor::net::Client;
use camino::{Utf8Path, Utf8PathBuf};
use slint::{Image, Rgb8Pixel, Rgba8Pixel, SharedPixelBuffer};

/// Parallel downloads and decodes.
const WORKERS: usize = 4;
/// Largest picture accepted, in bytes.
const MAX_BYTES: u64 = 8 * 1024 * 1024;
/// Extensions kept from the URL, so a cached file is named for what it is.
const EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "svg", "webp", "gif"];

/// What a picture is for, which sets the size it is scaled down to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Size {
    /// A grid poster, 2:3.
    Poster,
    /// A detail page's wide background.
    Backdrop,
    /// An episode still or a card's picture, 16:9.
    Still,
    /// A cast member's face, shown in an 88px circle.
    Face,
}

/// A decoded picture, ready to become a Slint image on the UI thread.
pub enum Picture {
    /// Opaque pixels.
    Rgb(SharedPixelBuffer<Rgb8Pixel>),
    /// Pixels with transparency.
    Rgba(SharedPixelBuffer<Rgba8Pixel>),
    /// SVG source, which Slint draws at any size.
    Svg(Vec<u8>),
}

/// The download queue and the pictures on their way.
pub struct Art {
    queue: Sender<(String, Size)>,
    pending: HashSet<(String, Size)>,
}

// Public API
impl Art {
    /// Starts the workers; `done` runs on a worker thread with the URL, the
    /// size asked for, and the picture, or `None` when it failed.
    pub fn start(
        dir: Utf8PathBuf,
        done: impl Fn(String, Size, Option<Picture>) + Send + Sync + 'static,
    ) -> Self {
        let (queue, requests) = mpsc::channel::<(String, Size)>();
        let requests = Arc::new(Mutex::new(requests));
        let done = Arc::new(done);
        let client = Client::new();
        for _ in 0..WORKERS {
            let (requests, done, client, dir) = (
                Arc::clone(&requests),
                Arc::clone(&done),
                client.clone(),
                dir.clone(),
            );
            thread::spawn(move || work(&requests, &client, &dir, &*done));
        }
        Self {
            queue,
            pending: HashSet::new(),
        }
    }

    /// Queues `url` at `size` unless it is empty or already on its way.
    pub fn request(&mut self, url: &str, size: Size) {
        if url.is_empty() || !self.pending.insert((url.to_owned(), size)) {
            return;
        }
        // Err only once every worker is gone; the title keeps its tile.
        let _ = self.queue.send((url.to_owned(), size));
    }

    /// Records that `url` at `size` arrived or failed, so it can be asked
    /// for again.
    pub fn finish(&mut self, url: &str, size: Size) {
        self.pending.remove(&(url.to_owned(), size));
    }
}

impl Size {
    /// The largest width and height a picture is scaled down to, about
    /// twice the size it is shown at.
    const fn bounds(self) -> (u32, u32) {
        match self {
            Size::Poster => (320, 480),
            Size::Backdrop => (1600, 900),
            Size::Still => (600, 338),
            Size::Face => (176, 224),
        }
    }
}

impl Picture {
    /// The picture as a Slint image; `None` when an SVG does not load.
    pub fn image(self) -> Option<Image> {
        match self {
            Picture::Rgb(pixels) => Some(Image::from_rgb8(pixels)),
            Picture::Rgba(pixels) => Some(Image::from_rgba8(pixels)),
            Picture::Svg(source) => Image::load_from_svg_data(&source).ok(),
        }
    }
}

fn work(
    requests: &Mutex<Receiver<(String, Size)>>,
    client: &Client,
    dir: &Utf8Path,
    done: &(dyn Fn(String, Size, Option<Picture>) + Send + Sync),
) {
    loop {
        // Hold the lock only to take one request.
        let Ok((url, size)) = requests.lock().expect("no worker panics holding it").recv() else {
            return;
        };
        let cached = cache_path(dir, &url);
        let path = if cached.exists() {
            Some(cached)
        } else {
            download(client, dir, &url)
        };
        let picture = path.and_then(|p| decode(&p, size));
        done(url, size, picture);
    }
}

/// Downloads `url` into the cache; `None` on any failure, or when the
/// picture is larger than `MAX_BYTES`.
fn download(client: &Client, dir: &Utf8Path, url: &str) -> Option<Utf8PathBuf> {
    let bytes = client.bytes(url, MAX_BYTES).ok()?;
    if bytes.is_empty() {
        return None;
    }
    fs::create_dir_all(dir).ok()?;
    let path = cache_path(dir, url);
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, &bytes).ok()?;
    fs::rename(&tmp, &path).ok()?;
    Some(path)
}

/// `dir/<hash of url>.<extension from url>`; `png` when the URL has none.
fn cache_path(dir: &Utf8Path, url: &str) -> Utf8PathBuf {
    // FNV-1a: stable across runs, unlike the std hasher.
    let hash = url.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let extension = path
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .filter(|ext| EXTENSIONS.contains(&ext.as_str()))
        .unwrap_or_else(|| "png".to_owned());
    dir.join(format!("{hash:016x}.{extension}"))
}

/// Decodes and scales the file at `path` for `size`; SVG is passed on. The
/// content decides the format: providers' URLs often name the wrong one.
fn decode(path: &Utf8Path, size: Size) -> Option<Picture> {
    let bytes = fs::read(path).ok()?;
    if is_svg(&bytes) {
        return Some(Picture::Svg(bytes));
    }
    let image = image::load_from_memory(&bytes).ok()?;
    let (width, height) = size.bounds();
    let image = if image.width() > width || image.height() > height {
        image.thumbnail(width, height)
    } else {
        image
    };
    Some(if image.color().has_alpha() {
        let rgba = image.to_rgba8();
        Picture::Rgba(SharedPixelBuffer::clone_from_slice(
            rgba.as_raw(),
            rgba.width(),
            rgba.height(),
        ))
    } else {
        let rgb = image.to_rgb8();
        Picture::Rgb(SharedPixelBuffer::clone_from_slice(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
        ))
    })
}

/// Whether `bytes` look like SVG (or other XML) rather than a raster image.
fn is_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(512)];
    let text = String::from_utf8_lossy(head);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    text.starts_with("<?xml") || text.starts_with("<svg")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_scales_down_and_keeps_alpha() {
        let dir = tempfile::tempdir().unwrap();
        let dir = Utf8Path::from_path(dir.path()).unwrap();
        let big = dir.join("big.png");
        image::RgbImage::new(1000, 1500).save(&big).unwrap();
        let Some(Picture::Rgb(pixels)) = decode(&big, Size::Poster) else {
            panic!("an opaque PNG decodes to RGB");
        };
        assert_eq!((pixels.width(), pixels.height()), (320, 480));

        let small = dir.join("small.png");
        image::RgbaImage::new(40, 60).save(&small).unwrap();
        let Some(Picture::Rgba(pixels)) = decode(&small, Size::Poster) else {
            panic!("a PNG with alpha decodes to RGBA");
        };
        assert_eq!(
            (pixels.width(), pixels.height()),
            (40, 60),
            "never scaled up"
        );

        let broken = dir.join("broken.jpg");
        fs::write(&broken, b"not an image").unwrap();
        assert!(decode(&broken, Size::Poster).is_none());

        // The content decides, whatever the name says.
        let vector = dir.join("logo.png");
        fs::write(
            &vector,
            "\u{feff}<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
        )
        .unwrap();
        assert!(matches!(
            decode(&vector, Size::Poster),
            Some(Picture::Svg(_))
        ));
        let raster = dir.join("poster.svg");
        fs::copy(&small, &raster).unwrap();
        assert!(matches!(
            decode(&raster, Size::Poster),
            Some(Picture::Rgba(_))
        ));
    }

    #[test]
    fn cache_path_keeps_known_extensions() {
        let dir = Utf8Path::new("/cache/logos");
        let svg = cache_path(dir, "http://x.tv/logo/1.SVG?size=big");
        assert_eq!(svg.extension(), Some("svg"));
        assert_eq!(svg.parent(), Some(dir));
        assert_eq!(cache_path(dir, "http://x.tv/logo").extension(), Some("png"));
        assert_eq!(
            cache_path(dir, "http://x.tv/a.php?id=3").extension(),
            Some("png")
        );
    }

    #[test]
    fn cache_path_is_stable_and_distinct() {
        let dir = Utf8Path::new("/c");
        assert_eq!(
            cache_path(dir, "http://a/1.png"),
            cache_path(dir, "http://a/1.png")
        );
        assert_ne!(
            cache_path(dir, "http://a/1.png"),
            cache_path(dir, "http://a/2.png")
        );
    }
}
