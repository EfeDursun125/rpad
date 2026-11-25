use crate::{edit::same_snapshot, search, AppWindow};
use slint::{ComponentHandle, SharedString};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc, Arc,
};

pub const BACKGROUND_THRESHOLD: usize = 1024 * 1024;

#[derive(Clone, Copy)]
pub enum Operation {
    Find(bool),
    ReplaceAll,
}

struct Job {
    generation: u64,
    operation: Operation,
    text: SharedString,
    needle: SharedString,
    replacement: SharedString,
    anchor: i32,
    cursor: i32,
}

enum Outcome {
    Find(Option<usize>, usize),
    Replace(Result<search::Replacement, &'static str>),
}

pub struct Worker {
    sender: mpsc::Sender<Option<Job>>,
    generation: Arc<AtomicU64>,
}

impl Worker {
    pub fn new(ui: &AppWindow) -> Self {
        let (sender, receiver) = mpsc::channel::<Option<Job>>();
        let generation = Arc::new(AtomicU64::new(0));
        let current = generation.clone();
        let weak = ui.as_weak();

        std::thread::spawn(move || {
            let mut cache = search::SearchCache::default();
            while let Ok(mut message) = receiver.recv() {
                // Coalesce pending jobs rather than scanning obsolete queries.
                for next in receiver.try_iter() {
                    message = next;
                }
                let Some(job) = message else {
                    cache.clear();
                    continue;
                };
                if current.load(Ordering::Relaxed) != job.generation {
                    continue;
                }

                let result = match job.operation {
                    Operation::Find(backwards) => {
                        let boundary = |offset: i32| {
                            let mut offset = (offset.max(0) as usize).min(job.text.len());
                            while !job.text.is_char_boundary(offset) {
                                offset -= 1;
                            }
                            offset
                        };
                        let anchor = boundary(job.anchor);
                        let cursor = boundary(job.cursor);
                        let (found, count) = cache.find(
                            job.text.clone(),
                            job.needle.clone(),
                            anchor.min(cursor),
                            anchor.max(cursor),
                            backwards,
                        );
                        Outcome::Find(found, count)
                    }
                    Operation::ReplaceAll => Outcome::Replace(search::replace_all(
                        &job.text,
                        &job.needle,
                        &job.replacement,
                    )),
                };

                if current.load(Ordering::Relaxed) != job.generation {
                    continue;
                }
                let weak = weak.clone();
                let current = current.clone();
                if slint::invoke_from_event_loop(move || {
                    let Some(ui) = weak.upgrade() else {
                        return;
                    };
                    if current.load(Ordering::Relaxed) != job.generation {
                        return;
                    }
                    // A result belongs only to its original document, query,
                    // and (for navigation) caret position.
                    let stale = !same_snapshot(&ui.get_document_text(), &job.text)
                        || ui.get_search_text() != job.needle
                        || match job.operation {
                            Operation::Find(_) => {
                                ui.get_anchor_offset() != job.anchor
                                    || ui.get_cursor_offset() != job.cursor
                            }
                            Operation::ReplaceAll => ui.get_replacement_text() != job.replacement,
                        };
                    if stale || ui.get_io_busy() || ui.get_dialog_kind() != 0 {
                        ui.set_search_status("Document or query changed; retry".into());
                        return;
                    }
                    match result {
                        Outcome::Find(found, count) => {
                            crate::apply_find_result(&ui, found, count, job.needle.len());
                        }
                        Outcome::Replace(result) => {
                            crate::apply_replace_all(&ui, result, job.replacement.len());
                        }
                    }
                })
                .is_err()
                {
                    break;
                }
            }
        });

        Self { sender, generation }
    }

    pub fn clear(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        let _ = self.sender.send(None);
    }

    pub fn submit(&self, ui: &AppWindow, operation: Operation) {
        let generation = self
            .generation
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        let needle = ui.get_search_text();
        if needle.is_empty() {
            ui.set_search_status("Enter text".into());
            let _ = self.sender.send(None);
            return;
        }
        let job = Job {
            generation,
            operation,
            text: ui.get_document_text(),
            needle,
            replacement: ui.get_replacement_text(),
            anchor: ui.get_anchor_offset(),
            cursor: ui.get_cursor_offset(),
        };
        ui.set_search_status(
            match operation {
                Operation::Find(_) => "Searching…",
                Operation::ReplaceAll => "Replacing…",
            }
            .into(),
        );
        if self.sender.send(Some(job)).is_err() {
            ui.set_search_status("Search worker unavailable".into());
        }
    }
}
