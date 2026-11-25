//! Per-window, private crash snapshots. Never overwrite an original document.
use crate::{edit::same_snapshot, AppWindow};
use slint::{ComponentHandle, SharedString};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::Duration,
};

const SNAPSHOT: &str = "unsaved.txt";

fn lock(path: &Path, create: bool) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    if create {
        options.create_new(true);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(file)
}

fn snapshot(directory: &Path, text: Option<&str>) -> io::Result<()> {
    let path = directory.join(SNAPSHOT);
    if let Some(text) = text {
        let mut temp = tempfile::NamedTempFile::new_in(directory)?;
        temp.write_all(text.as_bytes())?;
        temp.as_file().sync_all()?;
        temp.persist(path).map_err(|error| error.error)?;
    } else {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    #[cfg(unix)]
    File::open(directory)?.sync_all()?;
    Ok(())
}

fn recover(directory: &Path) -> Option<(PathBuf, File, SharedString)> {
    let entries = fs::read_dir(directory).ok()?;
    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().ok().is_some_and(|kind| kind.is_dir())
            || !entry.file_name().to_string_lossy().starts_with("session-")
        {
            continue;
        }
        let path = entry.path();
        let Ok(file) = lock(&path.join("lock"), false) else {
            continue; // Another live window owns this session.
        };
        let Ok(metadata) = fs::symlink_metadata(path.join(SNAPSHOT)) else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > i32::MAX as u64 {
            continue;
        }
        candidates.push((metadata.modified().ok(), path, file));
    }
    candidates.sort_by_key(|entry| entry.0);
    let (_, path, file) = candidates.pop()?;
    let text = fs::read_to_string(path.join(SNAPSHOT)).ok()?;
    Some((path, file, text.into()))
}

enum Message {
    Snapshot(Option<SharedString>),
    Finish,
}

pub struct Session {
    timer: slint::Timer,
    sender: mpsc::SyncSender<Message>,
    worker: Option<thread::JoinHandle<()>>,
    recovered: Option<(PathBuf, File)>,
}

impl Session {
    pub fn start(ui: &AppWindow, settings: &Path, offer: bool) -> io::Result<Self> {
        let base = settings
            .parent()
            .ok_or_else(|| io::Error::other("No configuration directory"))?
            .join("recovery");
        fs::create_dir_all(&base)?;
        let directory = tempfile::Builder::new()
            .prefix("session-")
            .tempdir_in(&base)?;
        let guard = lock(&directory.path().join("lock"), true)?;
        let directory = directory.keep();

        let recovered = if offer {
            recover(&base).and_then(|(path, file, text)| {
                let answer = rfd::MessageDialog::new()
                    .set_title("Recover unsaved text?")
                    .set_description("RPad found a previous unsaved session. Restore its text as a new, unsaved document? The original file will not be overwritten.")
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show();
                if answer == rfd::MessageDialogResult::Yes {
                    ui.set_document_text(text);
                    ui.invoke_edited();
                    Some((path, file))
                } else {
                    None
                }
            })
        } else {
            None
        };

        let (sender, receiver) = mpsc::sync_channel::<Message>(1);
        let (errors, received_errors) = mpsc::channel();
        let worker = thread::spawn(move || {
            while let Ok(message) = receiver.recv() {
                match message {
                    Message::Snapshot(text) => {
                        let text = text.as_ref().map(|text| text.as_str());
                        if let Err(error) = snapshot(&directory, text) {
                            let _ = errors.send(error.to_string());
                        }
                    }
                    Message::Finish => {
                        drop(guard);
                        let _ = fs::remove_dir_all(directory);
                        return;
                    }
                }
            }
        });

        if ui.get_dirty() {
            let _ = sender.try_send(Message::Snapshot(Some(ui.get_document_text())));
        }
        let timer = slint::Timer::default();
        let weak = ui.as_weak();
        let tx = sender.clone();
        let mut previous: Option<SharedString> = None;
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_secs(2),
            move || {
                let Some(ui) = weak.upgrade() else {
                    return;
                };
                if let Some(error) = received_errors.try_iter().last() {
                    ui.set_status(format!("Recovery snapshot failed: {error}").into());
                    previous = None;
                }
                let next = ui.get_dirty().then(|| ui.get_document_text());
                let unchanged = match (&previous, &next) {
                    (Some(before), Some(after)) => same_snapshot(before, after),
                    (None, None) => true,
                    _ => false,
                };
                if !unchanged && tx.try_send(Message::Snapshot(next.clone())).is_ok() {
                    previous = next;
                }
            },
        );

        Ok(Self {
            timer,
            sender,
            worker: Some(worker),
            recovered,
        })
    }

    pub fn finish(mut self) {
        self.timer.stop();
        let _ = self.sender.send(Message::Finish);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some((path, guard)) = self.recovered.take() {
            drop(guard);
            let _ = fs::remove_dir_all(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_are_atomic_and_live_sessions_are_excluded() {
        let base = tempfile::tempdir().unwrap();
        let path = base.path().join("session-test");
        fs::create_dir(&path).unwrap();
        let guard = lock(&path.join("lock"), true).unwrap();
        snapshot(&path, Some("Türkçe 🦀\n")).unwrap();
        assert!(recover(base.path()).is_none());
        drop(guard);
        let (_, guard, text) = recover(base.path()).unwrap();
        assert_eq!(text, "Türkçe 🦀\n");
        drop(guard);
        snapshot(&path, None).unwrap();
        assert!(recover(base.path()).is_none());
    }
}
