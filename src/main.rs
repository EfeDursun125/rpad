#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod controller;
mod document;
mod edit;
mod font;
mod history;
mod indent;
mod lines;
mod search;
mod settings;
mod syntax;
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
    refresh_content(ui, doc);
    ui.set_format_label(
        format!(
            "UTF-8{} · {}",
            if doc.bom { " BOM" } else { "" },
            if doc.crlf { "CRLF" } else { "LF" }
        )
        .into(),
    );
}

fn refresh_content(ui: &AppWindow, doc: &Document) {
    ui.set_dirty(doc.dirty(&ui.get_document_text()));
    update_position(ui);
}

fn update_position(ui: &AppWindow) {
    let text = ui.get_document_text();
    let mut offset = (ui.get_cursor_offset().max(0) as usize).min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let handle = ui.get_document_lines();
    let cached = handle
        .as_any()
        .downcast_ref::<lines::LineModel>()
        .and_then(|model| model.position(&text, offset));
    let (line, column) = cached.unwrap_or_else(|| {
        let before = &text[..offset];

        (
            before.bytes().filter(|byte| *byte == b'\n').count() + 1,
            edit::column(before.rsplit('\n').next().unwrap_or("")),
        )
    });
    ui.set_position_label(format!("Ln {line}, Col {column}").into());
}
fn update_lines(ui: &AppWindow) {
    update_lines_edit(ui, None);
}

fn update_lines_edit(ui: &AppWindow, edit: Option<edit::EditRange>) {
    let mut handle = ui.get_document_lines();

    if handle.as_any().downcast_ref::<lines::LineModel>().is_none() {
        handle = slint::ModelRc::new(lines::LineModel::default());
        ui.set_document_lines(handle.clone());
    }

    let model = handle.as_any().downcast_ref::<lines::LineModel>().unwrap();
    let count = model.sync(ui.get_document_text(), edit, ui.get_show_lines());
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
            if let Some(model) = handle.as_any().downcast_ref::<lines::LineModel>() {
                model.trim(ui.get_active_lines() as usize);
            }
        });
    }
}
fn install_document(ui: &AppWindow, state: &Rc<RefCell<Document>>, doc: Document) {
    // Existing rows belong to the previous document; load new rows without per-line fade timers.
    ui.set_document_lines(slint::ModelRc::new(lines::LineModel::default()));
    let filename = doc
        .path
        .as_ref()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Untitled".into());
    syntax::configure_filename(ui, filename.into());
    ui.set_document_text(doc.text.clone().into());
    ui.invoke_select_range(0, 0);
    *state.borrow_mut() = doc;
    refresh(ui, &state.borrow());
    update_lines(ui);
    ui.invoke_history_reset();
    ui.set_search_status("".into());
    ui.set_status("Ready".into());
}
fn find(ui: &AppWindow, backwards: bool, cache: &mut search::SearchCache) {
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
    let (found, count) = cache.find(text, needle.clone(), start, end, backwards);
    if let Some(index) = found {
        ui.invoke_select_range(index as i32, (index + needle.len()) as i32);
        ui.set_search_status(format!("{count} matches").into());
    } else {
        ui.set_search_status("No matches".into());
    }
}
fn replace(ui: &AppWindow, all: bool, cache: &mut search::SearchCache) {
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
        find(ui, false, cache);
    }
}
fn selection(ui: &AppWindow) -> history::Selection {
    history::Selection {
        anchor: ui.get_anchor_offset(),
        cursor: ui.get_cursor_offset(),
    }
}
fn bind_line_metrics(ui: &AppWindow) {
    let weak = ui.as_weak();

    ui.on_editor_line_height(move |family, weight, pixel_size| {
        let Some(ui) = weak.upgrade() else {
            return 0.0;
        };

        let adapter = i_slint_core::window::WindowInner::from_pub(ui.window()).window_adapter();
        let font = i_slint_core::graphics::FontRequest {
            family: Some(family),
            weight: Some(weight),
            pixel_size: Some(i_slint_core::lengths::LogicalLength::new(pixel_size)),
            ..Default::default()
        };

        adapter
            .renderer()
            .text_line_height(font)
            .map_or(0.0, |height| height.get())
    });
}

fn bind_editor(ui: &AppWindow, state: &Rc<RefCell<Document>>) {
    let search = Rc::new(RefCell::new(search::SearchCache::default()));
    let weak = ui.as_weak();
    ui.on_lines_visibility_changed(move || {
        if let Some(ui) = weak.upgrade() {
            update_lines(&ui);
        }
    });

    let weak = ui.as_weak();
    ui.on_syntax_changed(move || {
        if let Some(ui) = weak.upgrade() {
            syntax::configure(&ui);
        }
    });
    syntax::configure(ui);

    let weak = ui.as_weak();
    ui.on_indent_lines(move |reverse| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let selected = selection(&ui);
        if !reverse && selected.anchor == selected.cursor {
            ui.invoke_insert_at_caret("\t".into());
            return;
        }
        let original = ui.get_document_text();
        let (text, selected) = indent::indent(&original, selected, reverse);
        if text == original.as_str() {
            return;
        }
        ui.set_document_text(text.into());
        ui.invoke_select_range(selected.anchor, selected.cursor);
        ui.invoke_edited();
    });

    let history = Rc::new(RefCell::new(history::History::new(ui.get_document_text())));
    let weak = ui.as_weak();
    let doc = state.clone();
    let edits = history.clone();
    ui.on_edited(move || {
        if let Some(ui) = weak.upgrade() {
            let edit = edits
                .borrow_mut()
                .record(ui.get_document_text(), selection(&ui));
            update_lines_edit(&ui, edit);
            refresh_content(&ui, &doc.borrow());
        }
    });
    let weak = ui.as_weak();
    let edits = history.clone();
    ui.on_cursor_moved(move || {
        if let Some(ui) = weak.upgrade() {
            if edits
                .borrow_mut()
                .selection(&ui.get_document_text(), selection(&ui))
            {
                update_position(&ui);
            }
        }
    });
    let weak = ui.as_weak();
    let edits = history.clone();
    let matches = search.clone();
    ui.on_history_reset(move || {
        if let Some(ui) = weak.upgrade() {
            *edits.borrow_mut() = history::History::new(ui.get_document_text());
            matches.borrow_mut().clear();
        }
    });
    let weak = ui.as_weak();
    let edits = history.clone();
    let doc = state.clone();
    ui.on_history_command(move |redo| {
        let result = edits.borrow_mut().step(redo);
        if let (Some(ui), Some((text, selection))) = (weak.upgrade(), result) {
            ui.set_document_text(text);
            update_lines(&ui);
            ui.invoke_select_range(selection.anchor, selection.cursor);
            refresh_content(&ui, &doc.borrow());
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
    let matches = search.clone();
    let weak = ui.as_weak();
    ui.on_find_next(move |backwards| {
        if let Some(ui) = weak.upgrade() {
            find(&ui, backwards, &mut matches.borrow_mut());
        }
    });
    let weak = ui.as_weak();
    ui.on_replace_text(move |all| {
        if let Some(ui) = weak.upgrade() {
            replace(&ui, all, &mut search.borrow_mut());
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
    i_slint_core::api::set_xdg_app_id("rpad")?;
    bind_line_metrics(&ui);
    if let Some(font) = font::detect_ui() {
        ui.set_ui_font(font.family.into());
    }

    let settings_path = settings::path();
    let mut saved_settings =
        settings::Settings::load(&settings_path, settings::Settings::capture(&ui));
    saved_settings.apply(&ui);
    let window_settings = Rc::new(RefCell::new(saved_settings.clone()));
    let timer_settings = window_settings.clone();

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
            let current =
                settings::Settings::capture_preserving_window(&ui, &timer_settings.borrow());
            *timer_settings.borrow_mut() = current.clone();

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
        let mut reader = theme::Reader::default();
        loop {
            let colors = reader.read();
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

    if let Err(error) =
        settings::Settings::capture_preserving_window(&ui, &window_settings.borrow())
            .save(&settings_path)
    {
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
    #[ignore = "requires a display and SLINT_EMIT_DEBUG_INFO=1"]
    fn real_font_gutter_matches_editor_line_positions() {
        use i_slint_backend_testing::ElementHandle;

        let ui = AppWindow::new().unwrap();
        bind_line_metrics(&ui);
        ui.set_smooth_caret(false);
        ui.show().unwrap();

        let test_ui = ui.clone_strong();
        slint::Timer::single_shot(std::time::Duration::from_millis(100), move || {
            let ui = test_ui;
            let mut fonts = vec![
                ("JetBrains Mono Nerd Font".to_owned(), 400),
                ("Noto Sans Mono".to_owned(), 400),
                ("DejaVu Sans Mono".to_owned(), 400),
                ("Liberation Mono".to_owned(), 400),
                ("Missing RPad Font".to_owned(), 400),
            ];
            if let Some(font) = font::detect() {
                fonts.push((font.family, font.weight));
            }

            for (family, weight) in fonts {
                ui.set_editor_font(family.clone().into());
                ui.set_editor_font_weight(weight);

                for size in [13, 16, 19] {
                    ui.set_font_pixels(size);

                    for wrap in [false, true] {
                        ui.set_wrap_lines(wrap);
                        let lines: Vec<String> = (1..=40)
                            .map(|line| {
                                if line % 3 == 0 {
                                    String::new()
                                } else if wrap && line % 5 == 0 {
                                    "A long wrapped line with spaces. ".repeat(12)
                                } else {
                                    format!("{line} 🦀")
                                }
                            })
                            .collect();
                        let lines = std::env::var_os("RPAD_TEST_DOCUMENT")
                            .map(|path| {
                                std::fs::read_to_string(path)
                                    .unwrap()
                                    .split('\n')
                                    .map(str::to_owned)
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or(lines);
                        let text = lines.join("\n");
                        ui.set_document_text(text.clone().into());
                        update_lines(&ui);
                        ui.invoke_select_range(0, 0);
                        ui.window().take_snapshot().unwrap();
                        let numbers: Vec<_> =
                            ElementHandle::find_by_element_id(&ui, "AppWindow::number").collect();
                        assert!(!numbers.is_empty());
                        let editor = ElementHandle::find_by_element_id(&ui, "AppWindow::editor")
                            .next()
                            .unwrap();
                        let mut offset = 0;

                        for (number, line) in numbers.iter().zip(&lines) {
                            ui.invoke_select_range(offset, offset);
                            let number_y = number.absolute_position().y;
                            let text_y = editor.absolute_position().y + ui.get_caret_y();
                            assert!((number_y - text_y).abs() < 1.1,
                            "{family}, {size}px, wrap={wrap}, offset={offset}: gutter={number_y}, text={text_y}");
                            offset += line.len() as i32 + 1;
                        }

                        let last_start = (text.len() - lines.last().unwrap().len()) as i32;
                        ui.invoke_select_range(last_start, last_start);
                        ui.window().take_snapshot().unwrap();
                        let last_number =
                            ElementHandle::find_by_element_id(&ui, "AppWindow::number")
                                .last()
                                .unwrap();
                        let last_text_y = editor.absolute_position().y + ui.get_caret_y();
                        assert!(
                            (last_number.absolute_position().y - last_text_y).abs() < 1.1,
                            "last row drift: {family}, {size}px, wrap={wrap}"
                        );
                    }
                }
            }

            slint::quit_event_loop().unwrap();
        });
        slint::run_event_loop().unwrap();

        ui.hide().unwrap();
    }

    #[test]
    #[ignore = "requires a display"]
    fn real_paragraph_cache_matches_fresh_render_after_edits_and_style_changes() {
        let ui = AppWindow::new().unwrap();
        ui.set_smooth_caret(false);
        ui.set_show_lines(false);
        ui.set_document_name("sample.lua".into());
        bind_line_metrics(&ui);
        let state = Rc::new(RefCell::new(Document::default()));
        bind_editor(&ui, &state);
        ui.show().unwrap();
        let test_ui = ui.clone_strong();
        slint::Timer::single_shot(Duration::from_millis(100), move || {
            let ui = test_ui;
            let adapter = i_slint_core::window::WindowInner::from_pub(ui.window()).window_adapter();
            let cache = adapter.renderer().text_layout_cache().unwrap();
            for font in ["Noto Sans Mono", "DejaVu Sans Mono"] {
                ui.set_editor_font(font.into());
                for size in [13, 19] {
                    ui.set_font_pixels(size);
                    for wrap in [true, false] {
                        ui.set_wrap_lines(wrap);
                        for dark in [false, true] {
                            ui.set_dark_theme(dark);
                            for prefix in ["--[[", "-- comment", "local emoji = '🦀ğ'", ""] {
                                let original = format!(
                                    "{prefix}\n\tlocal value = 42\n--]]\n{}\nنص عربي\n",
                                    "long wrapped text ".repeat(30)
                                );
                                ui.set_document_text(original.into());
                                ui.invoke_select_range(3, 20);
                                ui.window().take_snapshot().unwrap();
                                let warm = ui.window().take_snapshot().unwrap();
                                let (entries, bytes, order, _) = cache.paragraph_memo_usage();
                                assert!(entries <= 4096 && bytes <= 32 * 1024 * 1024);
                                assert!(order <= entries * 4 + 64);
                                cache.clear_all();
                                ui.window().request_redraw();
                                let fresh = ui.window().take_snapshot().unwrap();
                                assert_eq!(
                                    warm.as_slice(),
                                    fresh.as_slice(),
                                    "{font} {size} wrap={wrap} dark={dark} prefix={prefix}"
                                );
                            }
                        }
                    }
                }
            }
            // Exercise eviction beyond both the entry and estimated byte budgets.
            let large = (0..5000)
                .map(|index| format!("local cached_name_{index} = {index} -- cache budget test\n"))
                .collect::<String>();
            ui.set_document_text(large.clone().into());
            ui.window().take_snapshot().unwrap();
            ui.set_document_text(format!("-- changed prefix\n{large}").into());
            ui.invoke_select_range(0, 0);
            let warm = ui.window().take_snapshot().unwrap();
            let (entries, bytes, order, _) = cache.paragraph_memo_usage();
            assert!(entries <= 4096 && bytes <= 32 * 1024 * 1024);
            assert!(order <= entries * 4 + 64);
            cache.clear_all();
            assert_eq!(cache.paragraph_memo_usage(), (0, 0, 0, 0));
            ui.window().request_redraw();
            let fresh = ui.window().take_snapshot().unwrap();
            assert_eq!(warm.as_slice(), fresh.as_slice());
            slint::quit_event_loop().unwrap();
        });
        slint::run_event_loop().unwrap();
        ui.hide().unwrap();
    }

    #[test]
    #[ignore = "requires a display; reports full edit-to-snapshot timing"]
    fn editor_performance_sample() {
        let ui = AppWindow::new().unwrap();
        ui.set_smooth_caret(false);
        ui.set_document_name("sample.lua".into());
        ui.set_document_path("sample.lua".into());
        ui.set_show_lines(false);
        bind_line_metrics(&ui);
        let state = Rc::new(RefCell::new(Document::default()));
        bind_editor(&ui, &state);
        let count = std::env::var("RPAD_PERF_LINES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1000);
        let original = (0..count)
            .map(|line| format!("local value_{line} = {line} -- cached paragraph\n"))
            .collect::<String>();
        ui.set_document_text(original.clone().into());
        ui.invoke_history_reset();
        update_lines(&ui);
        ui.show().unwrap();
        let test_ui = ui.clone_strong();
        slint::Timer::single_shot(Duration::from_millis(100), move || {
            let ui = test_ui;
            ui.window().take_snapshot().unwrap();
            let mut samples = Vec::new();
            for round in 0..12 {
                let changed = format!("local changed = {round}\n{original}");
                let started = std::time::Instant::now();
                ui.set_document_text(changed.into());
                ui.invoke_select_range(5, 5);
                ui.invoke_edited();
                ui.window().take_snapshot().unwrap();
                samples.push(started.elapsed().as_secs_f64() * 1000.);
            }
            samples.sort_by(f64::total_cmp);
            ui.set_search_text("local".into());
            let started = std::time::Instant::now();
            for _ in 0..500 {
                ui.invoke_find_next(false);
            }
            eprintln!(
                "RPAD_PERF lines={count} edit_snapshot_median_ms={:.3} find_500_ms={:.3}",
                samples[samples.len() / 2],
                started.elapsed().as_secs_f64() * 1000.
            );
            slint::quit_event_loop().unwrap();
        });
        slint::run_event_loop().unwrap();
        ui.hide().unwrap();
    }

    #[test]
    #[ignore = "requires a display and SLINT_EMIT_DEBUG_INFO=1"]
    fn real_tabs_syntax_colors_and_block_indent() {
        let ui = AppWindow::new().unwrap();
        let original = "a\tb\nab\tc\n\tlocal value = 42\n-- comment\n";
        ui.set_document_name("test.lua".into());
        ui.set_document_path("test.lua".into());
        ui.set_document_text(original.into());
        ui.set_smooth_caret(false);
        bind_line_metrics(&ui);
        let mut document = Document::default();
        document.path = Some("test.lua".into());
        let state = Rc::new(RefCell::new(document));
        bind_editor(&ui, &state);
        update_lines(&ui);
        ui.show().unwrap();
        let test_ui = ui.clone_strong();
        slint::Timer::single_shot(Duration::from_millis(100), move || {
            let ui = test_ui;
            ui.invoke_select_range(1, 1);
            let cell = ui.get_caret_x();
            ui.invoke_select_range(2, 2);
            let stop = ui.get_caret_x();
            assert!(
                (stop - cell * 4.).abs() < 0.2,
                "tab={stop}, space cell={cell}"
            );
            ui.invoke_select_range(7, 7);
            assert!((ui.get_caret_x() - stop).abs() < 0.2);
            ui.invoke_select_range(10, 10);
            assert!((ui.get_caret_x() - stop).abs() < 0.2);
            ui.invoke_select_range(0, 0);
            let snapshot = ui.window().take_snapshot().unwrap();
            let mut cache = i_slint_core::syntax_highlight::SyntaxCache::default();
            let spans = cache.update(&original.into(), "test.lua".into(), true);
            let mut matched = std::collections::HashSet::new();
            for color in spans
                .iter()
                .flat_map(|line| line.iter().map(|span| span.color))
            {
                if snapshot.as_slice().iter().any(|pixel| {
                    pixel.r.abs_diff(color.red()) < 3
                        && pixel.g.abs_diff(color.green()) < 3
                        && pixel.b.abs_diff(color.blue()) < 3
                }) {
                    matched.insert((color.red(), color.green(), color.blue()));
                }
            }
            assert!(
                matched.len() >= 2,
                "syntax colors not rendered: {matched:?}"
            );
            assert_eq!(ui.get_document_text().as_str(), original);
            ui.invoke_select_range(0, 3);
            ui.invoke_indent_lines(false);
            assert!(ui.get_document_text().starts_with("\ta\tb"));
            ui.invoke_history_command(false);
            assert_eq!(ui.get_document_text().as_str(), original);
            ui.invoke_history_command(true);
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                    text: slint::platform::Key::Backtab.into(),
                });
            assert_eq!(ui.get_document_text().as_str(), original);
            // Tabs must paint exactly like blank spaces, not a missing-glyph box.
            ui.set_document_text("\t\t\t\t".into());
            ui.invoke_select_range(0, 0);
            let tabs = ui.window().take_snapshot().unwrap();
            ui.set_document_text("                ".into());
            ui.invoke_select_range(0, 0);
            let spaces = ui.window().take_snapshot().unwrap();
            let origin = i_slint_backend_testing::ElementHandle::find_by_element_id(
                &ui,
                "AppWindow::editor",
            )
            .next()
            .unwrap()
            .absolute_position();
            let scale = ui.window().scale_factor();
            let left = (origin.x * scale) as usize + 3;
            let top = (origin.y * scale) as usize;
            let width = tabs.width() as usize;
            for y in top..top + (ui.get_font_pixels() as f32 * 1.5 * scale) as usize {
                for x in left..left + (cell * 16. * scale) as usize - 4 {
                    assert_eq!(
                        tabs.as_slice()[y * width + x],
                        spaces.as_slice()[y * width + x],
                        "tab paints ink at {x},{y}"
                    );
                }
            }
            ui.set_document_text("\t\t\t\t".into());
            ui.invoke_select_range(2, 4);
            let selected_tabs = ui.window().take_snapshot().unwrap();
            assert_eq!(ui.get_document_text().as_str(), "\t\t\t\t");
            ui.set_document_text("                ".into());
            ui.invoke_select_range(8, 16);
            let selected_spaces = ui.window().take_snapshot().unwrap();
            for tab in 0..4 {
                let mut different = 0;
                let start = (origin.x * scale + cell * 4. * tab as f32 * scale) as usize + 1;
                let end = (origin.x * scale + cell * 4. * (tab + 1) as f32 * scale) as usize - 1;
                for y in top..top + (ui.get_font_pixels() as f32 * 1.5 * scale) as usize {
                    for x in start..end {
                        different += usize::from(
                            selected_tabs.as_slice()[y * width + x]
                                != selected_spaces.as_slice()[y * width + x],
                        );
                    }
                }
                if tab < 2 {
                    assert_eq!(different, 0, "unselected tab {tab} is marked");
                } else {
                    assert!(
                        different > 8,
                        "selected tab {tab} has no arrow: {different}"
                    );
                }
            }
            ui.set_document_text("\t\t\t\t".into());
            ui.invoke_select_range(4, 2);
            let backwards = ui.window().take_snapshot().unwrap();
            for y in top..top + (ui.get_font_pixels() as f32 * 1.5 * scale) as usize {
                for x in left..left + (cell * 16. * scale) as usize - 4 {
                    assert_eq!(
                        backwards.as_slice()[y * width + x],
                        selected_tabs.as_slice()[y * width + x]
                    );
                }
            }
            if let Some(path) = std::env::var_os("RPAD_TAB_MARKER_CAPTURE") {
                let mut image =
                    format!("P6\n{} {}\n255\n", backwards.width(), backwards.height()).into_bytes();
                for pixel in backwards.as_slice() {
                    image.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
                }
                std::fs::write(path, image).unwrap();
            }
            slint::quit_event_loop().unwrap();
        });
        slint::run_event_loop().unwrap();
        ui.hide().unwrap();
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
    fn caret_advances_on_frames_shorter_than_eight_milliseconds() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = AppWindow::new().unwrap();
        ui.show().unwrap();
        ui.invoke_focus_editor();
        advance(1);
        ui.invoke_move_caret(100., 50.);
        let before = ui.get_smooth_x();

        advance(5);

        assert!(ui.get_smooth_x() > before);
        assert!(ui.get_smooth_x() < 100.);
        ui.set_smooth_caret(false);
        advance(1);
        assert_eq!(ui.get_smooth_x(), 100.);
        ui.hide().unwrap();
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
        replace(&ui, true, &mut search::SearchCache::default());
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
        replace(&ui, true, &mut search::SearchCache::default());
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
