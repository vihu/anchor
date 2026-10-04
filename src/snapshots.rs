//! A tour of the window without a display, for checking the screens: Slint's
//! software renderer draws the real window, the tour clicks through it as a
//! user would, and each step is saved as a PNG.
//!
//! Built only with the `snapshots` feature. Against `tools/fake-stremio.py`:
//!
//! ```sh
//! ANCHOR_API_URL=http://127.0.0.1:8099/api/ \
//!     cargo run --features snapshots -- --snapshots target/snapshots
//! ```

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use camino::Utf8PathBuf;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{EventLoopProxy, Key, Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, PhysicalSize, PlatformError, Rgb8Pixel, SharedString};

use crate::ui::{AppWindow, NowPlaying, SearchData, Shell};

/// The window's size, as designed.
const SIZE: (u32, u32) = (1440, 900);
/// How often the loop runs timers and the queue.
const TICK: Duration = Duration::from_millis(16);
/// The flag that starts the tour, followed by the directory for the PNGs.
const FLAG: &str = "--snapshots";
/// The fake account the tour signs in with.
const EMAIL: &str = "demo@example.com";
const PASSWORD: &str = "demo";

/// One step: what to do, how long to wait for it, and the picture's name.
struct Step {
    name: &'static str,
    wait: Duration,
    action: fn(&AppWindow),
}

type Job = Box<dyn FnOnce() + Send>;

struct Headless {
    window: Rc<MinimalSoftwareWindow>,
    start: Instant,
    jobs: Arc<Mutex<VecDeque<Job>>>,
    quit: Arc<AtomicBool>,
    dir: Utf8PathBuf,
}

struct Proxy {
    jobs: Arc<Mutex<VecDeque<Job>>>,
    quit: Arc<AtomicBool>,
}

thread_local! {
    static APP: RefCell<Option<slint::Weak<AppWindow>>> = const { RefCell::new(None) };
}

/// The directory `--snapshots <dir>` names, if the tour was asked for.
pub fn requested() -> Option<Utf8PathBuf> {
    let mut args = std::env::args().skip_while(|a| a != FLAG).skip(1);
    args.next().map(Utf8PathBuf::from)
}

/// Draws the window in memory instead of on a display, and runs the tour
/// once the window exists.
///
/// # Errors
///
/// Returns an error when Slint already has a platform.
pub fn install(dir: Utf8PathBuf) -> Result<(), PlatformError> {
    std::fs::create_dir_all(&dir).map_err(|e| PlatformError::Other(e.to_string()))?;
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(PhysicalSize::new(SIZE.0, SIZE.1));
    slint::platform::set_platform(Box::new(Headless {
        window,
        start: Instant::now(),
        jobs: Arc::default(),
        quit: Arc::default(),
        dir,
    }))
    .map_err(|e| PlatformError::Other(e.to_string()))
}

/// Hands the tour the window.
pub fn attach(app: &AppWindow) {
    APP.with(|a| *a.borrow_mut() = Some(app.as_weak()));
}

fn steps() -> Vec<Step> {
    vec![
        Step {
            name: "sign-in",
            wait: Duration::from_secs(4),
            action: |_| {},
        },
        Step {
            name: "sign-in-error",
            wait: Duration::from_secs(2),
            action: |app| {
                app.set_login_email("nobody@example.com".into());
                app.set_login_password("wrong".into());
                app.invoke_login();
            },
        },
        Step {
            name: "home",
            wait: Duration::from_secs(4),
            action: |app| {
                app.set_login_email(EMAIL.into());
                app.set_login_password(PASSWORD.into());
                app.invoke_login();
            },
        },
        Step {
            name: "home-rows",
            wait: Duration::from_secs(2),
            action: |app| {
                for _ in 0..3 {
                    press(app, Key::DownArrow);
                }
                press(app, Key::RightArrow);
                press(app, Key::RightArrow);
            },
        },
        Step {
            name: "catalog",
            wait: Duration::from_secs(3),
            action: |app| app.global::<Shell>().invoke_open_catalog(0, 0),
        },
        Step {
            name: "catalog-paged",
            wait: Duration::from_secs(3),
            action: |app| {
                for _ in 0..4 {
                    press(app, Key::PageDown);
                }
            },
        },
        Step {
            name: "catalog-genre",
            wait: Duration::from_secs(2),
            action: |app| app.invoke_catalog_genre_selected(3),
        },
        Step {
            name: "catalog-series",
            wait: Duration::from_secs(3),
            action: |app| app.global::<Shell>().invoke_open_catalog(1, 1),
        },
        Step {
            name: "search-dropdown",
            wait: Duration::from_secs(2),
            action: |app| {
                app.invoke_focus_search();
                let search = app.global::<SearchData>();
                search.set_query("burrow".into());
                search.invoke_edited("burrow".into());
            },
        },
        Step {
            name: "search-chosen",
            wait: Duration::from_millis(500),
            action: |app| {
                press(app, Key::DownArrow);
                press(app, Key::DownArrow);
            },
        },
        Step {
            name: "search-screen",
            wait: Duration::from_secs(1),
            action: |app| app.global::<SearchData>().invoke_all(),
        },
        Step {
            name: "title-series",
            wait: Duration::from_secs(3),
            action: |app| app.invoke_home_open_card(0),
        },
        Step {
            name: "title-episode-watched",
            wait: Duration::from_secs(1),
            action: |app| {
                press(app, Key::UpArrow);
                press(app, Key::UpArrow);
                press(app, Key::UpArrow);
                press(app, Key::UpArrow);
                press(app, Key::UpArrow);
                press_text(app, "w");
            },
        },
        Step {
            name: "title-next-season",
            wait: Duration::from_secs(2),
            action: |app| press(app, Key::RightArrow),
        },
        Step {
            name: "title-movie",
            wait: Duration::from_secs(3),
            action: |app| {
                app.invoke_back();
                app.invoke_home_open_card(1);
            },
        },
        Step {
            name: "streams",
            wait: Duration::from_secs(2),
            action: |app| {
                app.invoke_back();
                app.invoke_home_play_hero();
            },
        },
        Step {
            name: "playing",
            wait: Duration::from_secs(2),
            action: |app| press(app, Key::Return),
        },
        Step {
            name: "stopped",
            wait: Duration::from_secs(2),
            action: |app| app.global::<NowPlaying>().invoke_stop(),
        },
        Step {
            name: "streams-movie",
            wait: Duration::from_secs(2),
            action: |app| {
                app.invoke_back();
                app.invoke_home_open_card(1);
                app.invoke_title_play();
            },
        },
        Step {
            name: "settings-account",
            wait: Duration::from_secs(1),
            action: |app| app.invoke_navigate(6),
        },
        Step {
            name: "settings-addons",
            wait: Duration::from_millis(500),
            action: |app| app.set_settings_section(1),
        },
        Step {
            name: "settings-player",
            wait: Duration::from_secs(1),
            action: |app| app.set_settings_section(2),
        },
        Step {
            name: "settings-about",
            wait: Duration::from_secs(1),
            action: |app| app.set_settings_section(3),
        },
        Step {
            name: "sidebar-folded",
            wait: Duration::from_millis(500),
            action: |app| app.global::<Shell>().invoke_toggle_sidebar(),
        },
        Step {
            name: "sidebar-section-folded",
            wait: Duration::from_millis(500),
            action: |app| {
                let shell = app.global::<Shell>();
                shell.invoke_toggle_sidebar();
                shell.invoke_fold(1);
            },
        },
    ]
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> Duration {
        self.start.elapsed()
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy {
            jobs: Arc::clone(&self.jobs),
            quit: Arc::clone(&self.quit),
        }))
    }

    fn run_event_loop(&self) -> Result<(), PlatformError> {
        let mut steps = steps().into_iter().enumerate().peekable();
        let mut due = Instant::now() + steps.peek().map_or(Duration::ZERO, |(_, s)| s.wait);
        let mut pending: Option<(usize, &'static str)> = None;
        if let Some((n, step)) = steps.next() {
            with_app(step.action);
            pending = Some((n, step.name));
        }
        loop {
            slint::platform::update_timers_and_animations();
            let jobs: Vec<Job> = self.jobs.lock().expect("no job panics").drain(..).collect();
            for job in jobs {
                job();
            }
            if self.quit.load(Ordering::Relaxed) {
                return Ok(());
            }
            if Instant::now() >= due {
                if let Some((n, name)) = pending.take() {
                    self.save(n, name);
                }
                let Some((n, step)) = steps.next() else {
                    return Ok(());
                };
                with_app(step.action);
                pending = Some((n, step.name));
                due = Instant::now() + step.wait;
            }
            std::thread::sleep(TICK);
        }
    }
}

impl Headless {
    /// Draws the window and saves it as `{n}-{name}.png`.
    fn save(&self, n: usize, name: &str) {
        let (width, height) = SIZE;
        let mut pixels = vec![Rgb8Pixel::default(); (width * height) as usize];
        self.window.request_redraw();
        self.window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, width as usize);
        });
        let bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
        let path = self.dir.join(format!("{:02}-{name}.png", n + 1));
        match image::RgbImage::from_raw(width, height, bytes).map(|i| i.save(&path)) {
            Some(Ok(())) => println!("{path}"),
            Some(Err(e)) => eprintln!("{path}: {e}"),
            None => eprintln!("{path}: wrong size"),
        }
    }
}

impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), slint::EventLoopError> {
        self.quit.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn invoke_from_event_loop(&self, event: Job) -> Result<(), slint::EventLoopError> {
        self.jobs.lock().expect("no job panics").push_back(event);
        Ok(())
    }
}

/// Presses and releases `key` in the window.
fn press(app: &AppWindow, key: Key) {
    press_text(app, &SharedString::from(key));
}

/// Presses and releases the key that types `text`.
fn press_text(app: &AppWindow, text: &str) {
    let text = SharedString::from(text);
    app.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    app.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn with_app(action: fn(&AppWindow)) {
    let app = APP.with(|a| a.borrow().as_ref().and_then(slint::Weak::upgrade));
    if let Some(app) = app {
        action(&app);
    }
}
