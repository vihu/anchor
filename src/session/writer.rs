//! Writes to the account's library on one thread, in the order they were
//! made, so a later write never lands before an earlier one; the window
//! waits for them before it closes.

use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};

use anchor::api::Api;
use anchor::library::LibraryItem;

/// The queue of library writes and the thread working through it.
pub(super) struct Writer {
    queue: Option<Sender<(String, Vec<LibraryItem>)>>,
    thread: Option<JoinHandle<()>>,
}

impl Writer {
    /// Starts the thread that writes through `api`.
    pub(super) fn start(api: Api) -> Self {
        let (queue, writes) = mpsc::channel::<(String, Vec<LibraryItem>)>();
        let thread = thread::spawn(move || {
            for (key, items) in writes {
                if let Err(e) = api.put_library(&key, &items) {
                    eprintln!("library: {e}");
                }
            }
        });
        Self {
            queue: Some(queue),
            thread: Some(thread),
        }
    }

    /// Queues `items` for the account of `key`.
    pub(super) fn send(&self, key: String, items: Vec<LibraryItem>) {
        if let Some(queue) = &self.queue {
            // Err only once the thread is gone, after `finish`.
            let _ = queue.send((key, items));
        }
    }

    /// Waits for every queued write; later ones are dropped.
    pub(super) fn finish(&mut self) {
        self.queue.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
