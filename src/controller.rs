use crate::{document::Document, install_document, refresh, AppWindow};
use slint::{CloseRequestResponse, ComponentHandle};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Pending {
    New,
    Open,
    Quit,
}

pub struct Controller {
    ui: slint::Weak<AppWindow>,
    document: Rc<RefCell<Document>>,
    pending: Cell<Option<Pending>>,
    busy: Cell<bool>,
}
impl Controller {
    pub fn new(ui: &AppWindow, document: Rc<RefCell<Document>>) -> Rc<Self> {
        Rc::new(Self {
            ui: ui.as_weak(),
            document,
            pending: Cell::new(None),
            busy: Cell::new(false),
        })
    }
    fn dialog(&self, kind: i32, title: &str, body: &str) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_dialog_title(title.into());
            ui.set_dialog_body(body.into());
            ui.set_dialog_kind(kind);
        }
    }
    fn error(&self, title: &str, body: &str) {
        self.pending.set(None);
        self.dialog(2, title, body);
    }
    fn set_busy(&self, busy: bool) {
        self.busy.set(busy);
        if let Some(ui) = self.ui.upgrade() {
            ui.set_io_busy(busy);
        }
    }
    pub fn request(self: &Rc<Self>, action: &str) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        if self.busy.get() || ui.get_dialog_kind() != 0 {
            return;
        }
        ui.set_menu(0);
        let pending = match action {
            "new" => Some(Pending::New),
            "open" => Some(Pending::Open),
            "quit" => Some(Pending::Quit),
            _ => None,
        };
        if let Some(operation) = pending {
            if self.document.borrow().dirty(&ui.get_document_text()) {
                self.pending.set(Some(operation));
                self.dialog(
                    1,
                    "Save your changes?",
                    &format!(
                        "{} has unsaved changes. Save them before continuing?",
                        ui.get_document_name()
                    ),
                );
            } else {
                self.execute(operation);
            }
            return;
        }
        match action {
            "save" => self.save(false),
            "save-as" => self.save(true),
            "about" => self.dialog(
                3,
                "RPad",
                "A small, calm text editor, made for personal use.\nRust · Slint · Material 3\n\n\nEfeDursun125\nVersion 0.1.2 · GPL-3.0-only",
            ),
            _ => {}
        }
    }
    pub fn respond(self: &Rc<Self>, response: &str) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        if self.busy.get() || ui.get_dialog_kind() == 0 {
            return;
        }
        let kind = ui.get_dialog_kind();
        ui.set_dialog_kind(0);
        if kind != 1 {
            self.pending.set(None);
            ui.invoke_focus_editor();
            return;
        }
        match response {
            "save" => self.save(false),
            "discard" => {
                if let Some(operation) = self.pending.take() {
                    self.execute(operation);
                }
            }
            _ => {
                self.pending.set(None);
                ui.invoke_focus_editor();
            }
        }
    }
    pub fn close_requested(self: &Rc<Self>) -> CloseRequestResponse {
        let Some(ui) = self.ui.upgrade() else {
            return CloseRequestResponse::HideWindow;
        };
        if self.busy.get() || ui.get_dialog_kind() != 0 {
            return CloseRequestResponse::KeepWindowShown;
        }
        if !self.document.borrow().dirty(&ui.get_document_text()) {
            return CloseRequestResponse::HideWindow;
        }
        self.request("quit");
        CloseRequestResponse::KeepWindowShown
    }
    fn execute(self: &Rc<Self>, operation: Pending) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        match operation {
            Pending::New => install_document(&ui, &self.document, Document::default()),
            Pending::Quit => {
                let _ = ui.hide();
                let _ = slint::quit_event_loop();
            }
            Pending::Open => self.open(),
        }
    }
    fn open(self: &Rc<Self>) {
        self.set_busy(true);
        let handler = self.clone();
        let weak = self.ui.clone();
        // Rc stays on the event-loop thread. Only Send data crosses to the worker.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let opened = rfd::FileDialog::new()
                .set_title("Open text file")
                .pick_file()
                .map(|path| {
                    Document::open(&path).map_err(|e| format!("{}\n\n{e}", path.display()))
                });
            let _ = tx.send(opened);
        });
        Self::poll(move || {
            if weak.upgrade().is_none() {
                return true;
            }
            match rx.try_recv() {
                Ok(result) => {
                    handler.set_busy(false);
                    match result {
                        Some(Ok(document)) => {
                            if let Some(ui) = handler.ui.upgrade() {
                                install_document(&ui, &handler.document, document);
                            }
                        }
                        Some(Err(error)) => handler.error("Could not open", &error),
                        None => {
                            if let Some(ui) = handler.ui.upgrade() {
                                ui.invoke_focus_editor();
                            }
                        }
                    }
                    true
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => false,
                Err(_) => {
                    handler.set_busy(false);
                    handler.error("Could not open", "The file chooser could not finish.");
                    true
                }
            }
        });
    }
    fn save(self: &Rc<Self>, save_as: bool) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        self.set_busy(true);
        let mut document = self.document.borrow().clone();
        let text = ui.get_document_text().to_string();
        let handler = self.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let path = if save_as || document.path.is_none() {
                choose_save_path(document.path.as_ref())
            } else {
                document.path.clone()
            };
            let result = path.map(|path| {
                document
                    .save(&path, &text)
                    .map(|()| document)
                    .map_err(|e| format!("{}\n\n{e}", path.display()))
            });
            let _ = tx.send(result);
        });
        Self::poll(move || match rx.try_recv() {
            Ok(result) => {
                handler.set_busy(false);
                match result {
                    Some(Ok(document)) => {
                        *handler.document.borrow_mut() = document;
                        if let Some(ui) = handler.ui.upgrade() {
                            refresh(&ui, &handler.document.borrow());
                            ui.set_status("Saved".into());
                        }
                        if let Some(operation) = handler.pending.take() {
                            handler.execute(operation);
                        } else if let Some(ui) = handler.ui.upgrade() {
                            ui.invoke_focus_editor();
                        }
                    }
                    Some(Err(error)) => handler.error("Could not save", &error),
                    None => {
                        handler.pending.set(None);
                        if let Some(ui) = handler.ui.upgrade() {
                            ui.invoke_focus_editor();
                        }
                    }
                }
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(_) => {
                handler.set_busy(false);
                handler.error("Could not save", "The save operation could not finish.");
                true
            }
        });
    }
    fn poll(mut callback: impl FnMut() -> bool + 'static) {
        slint::Timer::single_shot(std::time::Duration::from_millis(16), move || {
            if !callback() {
                Self::poll(callback);
            }
        });
    }
}
fn choose_save_path(existing: Option<&PathBuf>) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().set_title("Save as");
    if let Some(path) = existing {
        if let Some(parent) = path.parent() {
            dialog = dialog.set_directory(parent);
        }
        if let Some(name) = path.file_name() {
            dialog = dialog.set_file_name(name.to_string_lossy());
        }
    } else {
        dialog = dialog.set_file_name("Untitled.txt");
    }
    dialog.save_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (AppWindow, Rc<Controller>) {
        i_slint_backend_testing::init_no_event_loop();
        let ui = AppWindow::new().unwrap();
        let controller = Controller::new(&ui, Rc::new(RefCell::new(Document::default())));
        ui.show().unwrap();
        (ui, controller)
    }
    #[test]
    fn clean_close_never_prompts_and_repeated_dirty_close_has_one_dialog() {
        let (ui, controller) = setup();
        assert_eq!(
            controller.close_requested(),
            CloseRequestResponse::HideWindow
        );
        assert_eq!(ui.get_dialog_kind(), 0);
        ui.set_document_text("unsaved".into());
        assert_eq!(
            controller.close_requested(),
            CloseRequestResponse::KeepWindowShown
        );
        assert_eq!(ui.get_dialog_kind(), 1);
        controller.close_requested();
        controller.request("new");
        assert_eq!(controller.pending.get(), Some(Pending::Quit));
        controller.respond("cancel");
        assert_eq!(ui.get_dialog_kind(), 0);
        assert_eq!(ui.get_document_text(), "unsaved");
        assert_eq!(controller.pending.get(), None);
    }
    #[test]
    fn discard_continues_new_and_save_worker_completes_without_blocking_ui() {
        let (ui, controller) = setup();
        ui.set_document_text("discard me".into());
        controller.request("new");
        controller.respond("discard");
        assert_eq!(ui.get_document_text(), "");
        assert_eq!(ui.get_dialog_kind(), 0);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.txt");
        std::fs::write(&path, "original").unwrap();
        *controller.document.borrow_mut() = Document::open(&path).unwrap();
        ui.set_document_text("saved change".into());
        controller.request("new");
        controller.respond("save");
        assert!(ui.get_io_busy());
        assert_eq!(
            controller.close_requested(),
            CloseRequestResponse::KeepWindowShown
        );
        for _ in 0..1000 {
            i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(16));
            if !ui.get_io_busy() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(!ui.get_io_busy());
        assert_eq!(ui.get_dialog_kind(), 0);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "saved change");
        assert_eq!(ui.get_document_text(), "");
    }
}
