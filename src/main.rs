#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod controller;
mod document;
mod font;
mod history;
mod settings;
mod theme;

use document::Document;
use slint::{ComponentHandle, Model};
use std::{cell::RefCell, path::PathBuf, rc::Rc};
slint::include_modules!();

fn refresh(ui: &AppWindow, doc: &Document) {
    ui.set_document_name(
        doc.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into())
            .into(),
    );
    ui.set_document_path(
        doc.path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
            .into(),
    );
    ui.set_dirty(doc.dirty(&ui.get_document_text()));
    ui.set_format_label(
        format!(
            "UTF-8{} · {}",
            if doc.bom { " BOM" } else { "" },
            if doc.crlf { "CRLF" } else { "LF" }
        )
        .into(),
    );
    update_position(ui);
}
fn update_position(ui: &AppWindow) {
    let text = ui.get_document_text();
    let mut offset = (ui.get_cursor_offset().max(0) as usize).min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &text[..offset];
    let line = before.bytes().filter(|b| *b == b'\n').count() + 1;
    let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    ui.set_position_label(format!("Ln {line}, Col {column}").into());
}
fn update_lines(ui: &AppWindow) {
    let text = ui.get_document_text();
    let lines: Vec<_> = text.split('\n').collect();
    let count = lines.len();
    let mut handle = ui.get_document_lines();
    if handle
        .as_any()
        .downcast_ref::<slint::VecModel<LineRow>>()
        .is_none()
    {
        handle = slint::ModelRc::new(slint::VecModel::<LineRow>::default());
        ui.set_document_lines(handle.clone());
    }
    let model = handle
        .as_any()
        .downcast_ref::<slint::VecModel<LineRow>>()
        .unwrap();
    // Update existing rows in place so normal typing never restarts their fade.
    for (index, content) in lines.into_iter().enumerate() {
        let row = LineRow {
            content: content.into(),
            shown: true,
        };
        if let Some(previous) = model.row_data(index) {
            if previous != row {
                model.set_row_data(index, row);
            }
        } else {
            model.push(row);
        }
    }
    for index in count..model.row_count() {
        if let Some(mut row) = model.row_data(index) {
            if row.shown {
                row.shown = false;
                model.set_row_data(index, row);
            }
        }
    }
    if ui.get_active_lines() == count as i32 {
        return;
    }
    ui.set_active_lines(count as i32);
    let revision = ui.get_line_revision().wrapping_add(1);
    ui.set_line_revision(revision);
    if model.row_count() > count {
        let weak = ui.as_weak();
        slint::Timer::single_shot(std::time::Duration::from_millis(200), move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            // A newer insert/delete owns the rows now; don't interrupt its fade.
            if ui.get_line_revision() != revision {
                return;
            }
            let handle = ui.get_document_lines();
            if let Some(model) = handle.as_any().downcast_ref::<slint::VecModel<LineRow>>() {
                while model.row_count() > ui.get_active_lines() as usize {
                    model.remove(model.row_count() - 1);
                }
            }
        });
    }
}
fn install_document(ui: &AppWindow, state: &Rc<RefCell<Document>>, doc: Document) {
    ui.set_document_text(doc.text.clone().into());
    ui.invoke_select_range(0, 0);
    *state.borrow_mut() = doc;
    refresh(ui, &state.borrow());
    update_lines(ui);
    ui.invoke_history_reset();
    ui.set_search_status("".into());
    ui.set_status("Ready".into());
}
fn find(ui: &AppWindow, backwards: bool) {
    let text = ui.get_document_text();
    let needle = ui.get_search_text();
    if needle.is_empty() {
        ui.set_search_status("Enter text".into());
        return;
    }
    let cursor = (ui.get_cursor_offset().max(0) as usize).min(text.len());
    let anchor = (ui.get_anchor_offset().max(0) as usize).min(text.len());
    let start = cursor.min(anchor);
    let end = cursor.max(anchor);
    let found = if backwards {
        text[..start]
            .rfind(needle.as_str())
            .or_else(|| text[start..].rfind(needle.as_str()).map(|n| start + n))
    } else {
        text[end..]
            .find(needle.as_str())
            .map(|n| end + n)
            .or_else(|| text[..end].find(needle.as_str()))
    };
    if let Some(index) = found {
        ui.invoke_select_range(index as i32, (index + needle.len()) as i32);
        let count = text.matches(needle.as_str()).count();
        ui.set_search_status(format!("{count} matches").into());
    } else {
        ui.set_search_status("No matches".into());
    }
}
fn replace(ui: &AppWindow, all: bool) {
    let needle = ui.get_search_text();
    if needle.is_empty() {
        return;
    }
    let text = ui.get_document_text();
    let replacement = ui.get_replacement_text();
    if all {
        let count = text.matches(needle.as_str()).count();
        if count > 0 {
            ui.set_document_text(text.replace(needle.as_str(), replacement.as_str()).into());
            ui.invoke_select_range(0, 0);
            ui.invoke_edited();
        }
        ui.set_search_status(format!("{count} replaced").into());
    } else {
        let a = (ui.get_cursor_offset().max(0) as usize).min(text.len());
        let b = (ui.get_anchor_offset().max(0) as usize).min(text.len());
        if text.get(a.min(b)..a.max(b)) == Some(needle.as_str()) {
            let mut changed = text.to_string();
            changed.replace_range(a.min(b)..a.max(b), replacement.as_str());
            ui.set_document_text(changed.into());
            let end = a.min(b) + replacement.len();
            ui.invoke_select_range(end as i32, end as i32);
            ui.invoke_edited();
        }
        find(ui, false);
    }
}
fn selection(ui: &AppWindow) -> history::Selection {
    history::Selection {
        anchor: ui.get_anchor_offset(),
        cursor: ui.get_cursor_offset(),
    }
}
fn bind_editor(ui: &AppWindow, state: &Rc<RefCell<Document>>) {
    let history = Rc::new(RefCell::new(history::History::new(
        ui.get_document_text().to_string(),
    )));
    let weak = ui.as_weak();
    let doc = state.clone();
    let edits = history.clone();
    ui.on_edited(move || {
        if let Some(ui) = weak.upgrade() {
            edits
                .borrow_mut()
                .record(&ui.get_document_text(), selection(&ui));
            refresh(&ui, &doc.borrow());
            update_lines(&ui);
        }
    });
    let weak = ui.as_weak();
    let edits = history.clone();
    ui.on_cursor_moved(move || {
        if let Some(ui) = weak.upgrade() {
            edits
                .borrow_mut()
                .selection(&ui.get_document_text(), selection(&ui));
            update_position(&ui);
        }
    });
    let weak = ui.as_weak();
    let edits = history.clone();
    ui.on_history_reset(move || {
        if let Some(ui) = weak.upgrade() {
            *edits.borrow_mut() = history::History::new(ui.get_document_text().to_string());
        }
    });
    let weak = ui.as_weak();
    let edits = history.clone();
    let doc = state.clone();
    ui.on_history_command(move |redo| {
        let result = edits.borrow_mut().step(redo);
        if let (Some(ui), Some((text, selection))) = (weak.upgrade(), result) {
            ui.set_document_text(text.into());
            ui.invoke_select_range(selection.anchor, selection.cursor);
            refresh(&ui, &doc.borrow());
            update_lines(&ui);
        }
    });
    let weak = ui.as_weak();
    ui.on_insert_at_caret(move |inserted| {
        if let Some(ui) = weak.upgrade() {
            let mut text = ui.get_document_text().to_string();
            let selection = selection(&ui);
            let start = selection.anchor.min(selection.cursor).max(0) as usize;
            let end = selection.anchor.max(selection.cursor).max(0) as usize;
            text.replace_range(start..end, &inserted);
            ui.set_document_text(text.into());
            let cursor = (start + inserted.len()) as i32;
            ui.invoke_select_range(cursor, cursor);
            ui.invoke_edited();
        }
    });
    let weak = ui.as_weak();
    ui.on_find_next(move |backwards| {
        if let Some(ui) = weak.upgrade() {
            find(&ui, backwards);
        }
    });
    let weak = ui.as_weak();
    ui.on_replace_text(move |all| {
        if let Some(ui) = weak.upgrade() {
            replace(&ui, all);
        }
    });
}

fn arguments() -> Result<Option<PathBuf>, String> {
    let mut path = None;
    let mut positional = false;
    for arg in std::env::args_os().skip(1) {
        if !positional && arg == "--" {
            positional = true;
            continue;
        }
        if !positional && (arg == "--help" || arg == "-h") {
            println!("RPad — a small Material 3 text editor\n\nUsage: rpad [FILE]\n       rpad -- [FILE]\n       sudo rpad /etc/systemd/zram-generator.conf\n\n-h, --help     Show help\n-V, --version  Show version");
            std::process::exit(0);
        }
        if !positional && (arg == "--version" || arg == "-V") {
            println!("RPad {}", env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        }
        if !positional && arg.to_string_lossy().starts_with('-') {
            return Err(format!("Unknown option: {}", arg.to_string_lossy()));
        }
        if path.is_some() {
            return Err("Open one file per window: rpad [FILE]".into());
        }
        path = Some(PathBuf::from(arg));
    }
    Ok(path)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = arguments().unwrap_or_else(|error| {
        eprintln!("rpad: {error}");
        std::process::exit(2)
    });
    theme::prepare_sudo_display();
    let initial = if let Some(path) = path {
        Document::open(&path).unwrap_or_else(|error| {
            eprintln!("rpad: {}: {error}", path.display());
            std::process::exit(1)
        })
    } else {
        Document::default()
    };
    let ui = AppWindow::new()?;
    if let Some(font) = font::detect_ui() {
        ui.set_ui_font(font.family.into());
    }

    let settings_path = settings::path();
    let mut saved_settings =
        settings::Settings::load(&settings_path, settings::Settings::capture(&ui));
    saved_settings.apply(&ui);

    let settings_timer = slint::Timer::default();
    let settings_ui = ui.as_weak();
    let timer_path = settings_path.clone();
    settings_timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(500),
        move || {
            let Some(ui) = settings_ui.upgrade() else {
                return;
            };
            let current = settings::Settings::capture(&ui);

            if current == saved_settings {
                return;
            }

            match current.save(&timer_path) {
                Ok(()) => saved_settings = current,
                Err(error) => ui.set_status(format!("Could not save settings: {error}").into()),
            }
        },
    );
    if let Some(font) = font::detect() {
        ui.set_editor_font(font.family.into());
        ui.set_editor_font_weight(font.weight);
    }
    let state = Rc::new(RefCell::new(Document::default()));
    install_document(&ui, &state, initial);
    // File reads and portal queries run away from the event loop. Reapply only changed palettes.
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let mut previous = None;
        loop {
            let colors = theme::read();
            if previous.as_ref() != Some(&colors) {
                previous = Some(colors.clone());
                let weak = weak.clone();
                if slint::invoke_from_event_loop(move || {
                    if let Some(ui) = weak.upgrade() {
                        theme::apply(&ui, &colors);
                    }
                })
                .is_err()
                {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
    bind_editor(&ui, &state);
    let controller = controller::Controller::new(&ui, state.clone());
    let handler = controller.clone();
    ui.on_action(move |action| handler.request(&action));
    let handler = controller.clone();
    ui.on_dialog_response(move |response| handler.respond(&response));
    let handler = controller.clone();
    ui.window()
        .on_close_requested(move || handler.close_requested());
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(30), move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_select_range(0, 0);
        }
    });
    ui.run()?;
    settings_timer.stop();

    if let Err(error) = settings::Settings::capture(&ui).save(&settings_path) {
        eprintln!("rpad: could not save settings: {error}");
    }

    Ok(())
}

#[cfg(test)]
mod editor_tests {
    use super::*;
    use std::time::Duration;
    fn advance(ms: u64) {
        // Flush reactive Timer::running changes before moving the mock clock.
        i_slint_backend_testing::mock_elapsed_time(Duration::ZERO);
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(ms));
        slint::platform::update_timers_and_animations();
    }
    slint::slint! {
        import { AnimatedLineNumber } from "ui/line-number.slint";
        export component NumberFadeTest inherits Window {
            width: 100px; height: 40px;
            in-out property <bool> shown: true;
            in-out property <bool> animated: true;
            out property <float> alpha: number.opacity;
            number := AnimatedLineNumber { text: "2"; shown: root.shown; animated: root.animated; }
        }
    }

    mod frame_pacing {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/vendor/i-slint-backend-winit/frame_pacing.rs"
        ));
    }

    #[test]
    fn display_refresh_interval_never_exceeds_180_fps() {
        use frame_pacing::render_interval;
        assert_eq!(render_interval(Some(60_000)).as_nanos(), 16_666_667);
        assert_eq!(render_interval(Some(120_000)).as_nanos(), 8_333_334);
        assert_eq!(render_interval(Some(180_000)).as_nanos(), 5_555_556);
        assert_eq!(
            render_interval(Some(240_000)),
            render_interval(Some(180_000))
        );
        assert_eq!(render_interval(Some(59_940)).as_nanos(), 16_683_351);
        assert_eq!(render_interval(None), render_interval(Some(60_000)));
        assert_eq!(render_interval(Some(0)), render_interval(None));
    }

    #[test]
    fn find_button_toggles_but_shortcut_open_keeps_search_visible() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = AppWindow::new().unwrap();
        ui.invoke_toggle_find();
        assert!(ui.get_find_open());
        ui.invoke_open_find();
        assert!(ui.get_find_open());
        ui.invoke_toggle_find();
        assert!(!ui.get_find_open());
    }

    #[test]
    fn number_fades_in_slowly_and_fades_out_quickly() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = NumberFadeTest::new().unwrap();
        ui.show().unwrap();
        assert_eq!(ui.get_alpha(), 0.);
        advance(1);
        advance(1);
        advance(45);
        let early = ui.get_alpha();
        advance(45);
        let middle = ui.get_alpha();
        assert!(early > 0. && early < middle && middle < 0.5);
        assert!(middle - early > early, "fade-in must accelerate");
        advance(200);
        assert_eq!(ui.get_alpha(), 1.);
        ui.set_shown(false);
        advance(45);
        let leaving = ui.get_alpha();
        assert!(leaving > 0. && leaving < 1.);
        assert!(
            1. - leaving > early,
            "fade-out must start faster than fade-in"
        );
        advance(200);
        assert_eq!(ui.get_alpha(), 0.);
        ui.hide().unwrap();
    }

    #[test]
    fn disabling_number_animation_finishes_active_and_future_fades() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = NumberFadeTest::new().unwrap();
        ui.show().unwrap();
        advance(45);
        ui.set_animated(false);
        advance(1);
        assert_eq!(ui.get_alpha(), 1.);

        ui.set_shown(false);
        advance(1);
        assert_eq!(ui.get_alpha(), 0.);

        ui.set_shown(true);
        advance(1);
        assert_eq!(ui.get_alpha(), 1.);
        ui.hide().unwrap();
    }

    #[test]
    fn rapid_numbers_animate_even_before_their_first_opacity_read() {
        i_slint_backend_testing::init_no_event_loop();
        let mut numbers = Vec::new();
        for _ in 0..5 {
            let ui = NumberFadeTest::new().unwrap();
            ui.show().unwrap();
            numbers.push(ui);
            advance(25);
        }
        // A late first render must see five different intermediate opacities,
        // never a skipped transition caused by a lazily evaluated binding.
        let alphas: Vec<_> = numbers.iter().map(NumberFadeTest::get_alpha).collect();
        assert!(alphas.iter().all(|a| *a > 0. && *a < 1.), "{alphas:?}");
        assert!(alphas.windows(2).all(|pair| pair[0] > pair[1]));
        advance(200);
        for number in numbers {
            assert_eq!(number.get_alpha(), 1.);
            number.hide().unwrap();
        }
    }

    #[test]
    fn deleted_numbers_stay_until_faded_and_rapid_reinsert_is_preserved() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = AppWindow::new().unwrap();
        ui.set_document_text("one\ntwo\nthree".into());
        update_lines(&ui);
        let handle = ui.get_document_lines();
        ui.set_document_text("one".into());
        update_lines(&ui);
        assert_eq!(handle.row_count(), 3);
        assert!(!handle.row_data(1).unwrap().shown);
        advance(90);
        assert_eq!(handle.row_count(), 3);
        ui.set_document_text("one\ntwo".into());
        update_lines(&ui);
        assert!(handle.row_data(1).unwrap().shown);
        advance(120);
        assert_eq!(
            handle.row_count(),
            3,
            "old removal must not truncate a newer fade"
        );
        advance(100);
        assert_eq!(handle.row_count(), 2);
        ui.set_document_text("ONE\ntwo".into());
        update_lines(&ui);
        assert_eq!(
            handle,
            ui.get_document_lines(),
            "typing must keep the same model"
        );
    }

    #[test]
    fn character_fades_keep_separate_ages_and_deleted_text_never_brightens() {
        use i_slint_core::items::text_fade::TextFadeState;
        let mut effects = TextFadeState::default();
        effects.record("", "a", 0, Some(0..0));
        effects.record("a", "ab", 40, Some(1..1));
        assert_eq!(effects.insertions.len(), 2);
        assert!(effects.insertions[0].opacity(80) > effects.insertions[1].opacity(80));
        assert!((effects.insertions[0].opacity(90) - 0.25).abs() < 0.001);
        let opacity_at_delete = effects.insertions[1].opacity(80);
        effects.record("ab", "a", 80, Some(1..2));
        assert_eq!(effects.insertions.len(), 1);
        let ghost = &effects.deletions[0];
        assert_eq!(&ghost.text[ghost.range.clone()], "b");
        let frozen = ghost.insertions.iter().find(|i| i.range == (1..2)).unwrap();
        assert_eq!(frozen.opacity(ghost.started), opacity_at_delete);
        assert!(ghost.opacity(125) < 1. && ghost.opacity(125) > 0.);
        effects.prune(260);
        assert!(effects.insertions.is_empty() && effects.deletions.is_empty());
    }

    #[test]
    fn character_fade_offsets_follow_unicode_edits_and_repeated_characters() {
        use i_slint_core::items::text_fade::TextFadeState;
        let mut effects = TextFadeState::default();
        effects.record("", "ğab", 0, Some(0..0));
        effects.record("ğab", "öğab", 10, Some(0..0));
        assert_eq!(effects.insertions[0].range, 2..6);
        assert_eq!(effects.insertions[1].range, 0..2);
        for insertion in &effects.insertions {
            assert!("öğab".get(insertion.range.clone()).is_some());
        }
        effects.clear();
        effects.record("aaa", "aaaa", 20, Some(0..0));
        assert_eq!(
            effects.insertions[0].range,
            0..1,
            "fade the typed a, not the last a"
        );
        effects.clear();
        assert!(effects.insertions.is_empty() && effects.deletions.is_empty());
    }

    #[test]
    fn zero_width_disables_native_caret_without_disabling_editing_geometry() {
        i_slint_backend_testing::init_no_event_loop();
        let input = Box::pin(i_slint_core::items::TextInput::default());
        input.enabled.set(true);
        input.cursor_visible.set(true);
        input.text.set("text".into());
        input
            .text_cursor_width
            .set(i_slint_core::lengths::LogicalLength::new(0.));
        assert!(input
            .as_ref()
            .visual_representation()
            .cursor_position
            .is_none());
        input
            .text_cursor_width
            .set(i_slint_core::lengths::LogicalLength::new(2.));
        assert_eq!(
            input.as_ref().visual_representation().cursor_position,
            Some(0)
        );
    }

    #[test]
    fn typing_replacement_undo_document_switch_and_caret_animation() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = AppWindow::new().unwrap();
        let state = Rc::new(RefCell::new(Document::default()));
        bind_editor(&ui, &state);
        ui.window().set_size(slint::LogicalSize::new(980.0, 700.0));
        ui.show().unwrap();
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::WindowActiveChanged(true));
        ui.invoke_focus_editor();
        advance(1);
        assert!(ui.get_caret_focused(), "editor should have focus");
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                text: "alpha beta alpha".into(),
            });
        advance(1);
        assert_eq!(ui.get_document_text(), "alpha beta alpha");
        assert!(ui.get_dirty());
        ui.set_search_text("alpha".into());
        ui.set_replacement_text("Türkçe".into());
        replace(&ui, true);
        advance(1);
        assert_eq!(ui.get_document_text(), "Türkçe beta Türkçe");
        ui.invoke_edit_command("undo".into());
        advance(1);
        assert_eq!(ui.get_document_text(), "alpha beta alpha");
        ui.invoke_edit_command("redo".into());
        advance(1);
        assert_eq!(ui.get_document_text(), "Türkçe beta Türkçe");
        ui.invoke_select_range(0, 0);
        advance(100);
        let origin = ui.get_smooth_x();
        ui.invoke_select_range(7, 7);
        let target = ui.get_caret_x();
        assert!(target > origin);
        let immediate = ui.get_smooth_x();
        assert!(immediate < target, "caret should not teleport");
        advance(40);
        let halfway = ui.get_smooth_x();
        assert!(
            halfway > immediate && halfway < target,
            "caret must interpolate: origin={origin} immediate={immediate} halfway={halfway} target={target}"
        );
        advance(220);
        assert!((ui.get_smooth_x() - target).abs() < 0.1);

        // Horizontal and vertical moves must have the same normalized response.
        let x0 = ui.get_smooth_x();
        let y0 = ui.get_smooth_y();
        ui.invoke_move_caret(x0 + 80., y0 + 24.);
        advance(8);
        let x_fraction = (ui.get_smooth_x() - x0) / 80.;
        let y_fraction = (ui.get_smooth_y() - y0) / 24.;
        assert!(x_fraction > 0. && x_fraction < 1.);
        assert!((x_fraction - y_fraction).abs() < 0.0001);
        let x_previous = ui.get_smooth_x();
        let y_previous = ui.get_smooth_y();
        ui.invoke_move_caret(x0 - 40., y0 - 48.);
        assert_eq!(ui.get_smooth_x(), x_previous);
        assert_eq!(ui.get_smooth_y(), y_previous);
        advance(8);
        assert!(ui.get_smooth_x() < x_previous && ui.get_smooth_y() < y_previous);
        advance(300);
        assert!((ui.get_smooth_y() - ui.get_caret_y()).abs() < 0.01);

        // Exercise real line navigation, not just synthetic coordinates.
        ui.invoke_insert_at_caret("\nsecond line\nthird line".into());
        ui.invoke_select_range(0, 0);
        advance(300);
        let old_y = ui.get_smooth_y();
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                text: slint::platform::Key::DownArrow.into(),
            });
        let down_target = ui.get_caret_y();
        assert!(down_target > old_y);
        assert_eq!(ui.get_smooth_y(), old_y);
        advance(8);
        let down_step = ui.get_smooth_y();
        assert!(down_step > old_y && down_step < down_target);
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                text: slint::platform::Key::UpArrow.into(),
            });
        assert_eq!(ui.get_smooth_y(), down_step);
        advance(8);
        assert!(ui.get_smooth_y() < down_step);
        advance(300);
        advance(600);
        advance(600);
        advance(180);
        let alpha = ui.get_blink();
        assert!(
            alpha > 0.08 && alpha < 1.,
            "blink must fade through intermediate opacity: {alpha}"
        );
        ui.invoke_insert_at_caret("\tfirst\r\n".into());
        ui.set_search_text("first".into());
        ui.set_replacement_text("second".into());
        replace(&ui, true);
        advance(1);
        assert!(ui.get_document_text().contains("\tsecond\r\n"));
        ui.invoke_edit_command("undo".into());
        advance(1);
        assert!(ui.get_document_text().contains("\tfirst\r\n"));
        install_document(&ui, &state, Document::default());
        advance(1);
        ui.invoke_edit_command("undo".into());
        advance(1);
        assert_eq!(
            ui.get_document_text(),
            "",
            "undo must not leak the preceding document"
        );
        ui.hide().unwrap();
    }
}
