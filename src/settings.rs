use crate::AppWindow;
use slint::ComponentHandle;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    wrap: bool,
    lines: bool,
    smooth: bool,
    font_size: i32,
    window_width: u32,
    window_height: u32,
}

impl Settings {
    pub fn capture(ui: &AppWindow) -> Self {
        let size = ui.window().size().to_logical(ui.window().scale_factor());

        Self {
            window_width: if size.width > 0. {
                (size.width.round() as u32).clamp(540, 16384)
            } else {
                980
            },
            window_height: if size.height > 0. {
                (size.height.round() as u32).clamp(360, 16384)
            } else {
                700
            },
            wrap: ui.get_wrap_lines(),
            lines: ui.get_show_lines(),
            smooth: ui.get_smooth_caret(),
            font_size: ui.get_font_pixels(),
        }
    }

    pub fn capture_preserving_window(ui: &AppWindow, previous: &Self) -> Self {
        let mut current = Self::capture(ui);

        if ui.window().is_fullscreen() || ui.window().is_maximized() {
            current.window_width = previous.window_width;
            current.window_height = previous.window_height;
        }

        current
    }

    pub fn apply(&self, ui: &AppWindow) {
        ui.window().set_size(slint::LogicalSize::new(
            self.window_width as f32,
            self.window_height as f32,
        ));
        ui.set_wrap_lines(self.wrap);
        ui.set_show_lines(self.lines);
        ui.set_smooth_caret(self.smooth);
        ui.set_font_pixels(self.font_size);
    }

    pub fn load(path: &Path, mut defaults: Self) -> Self {
        let Ok(text) = fs::read_to_string(path) else {
            return defaults;
        };

        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };

            match key.trim() {
                "wrap" => {
                    if let Ok(value) = value.trim().parse() {
                        defaults.wrap = value;
                    }
                }
                "lines" => {
                    if let Ok(value) = value.trim().parse() {
                        defaults.lines = value;
                    }
                }
                "smooth" => {
                    if let Ok(value) = value.trim().parse() {
                        defaults.smooth = value;
                    }
                }
                "font_size" => {
                    if let Ok(value) = value.trim().parse::<i32>() {
                        defaults.font_size = value.clamp(10, 36);
                    }
                }
                "window_width" => {
                    if let Ok(value) = value.trim().parse::<u32>() {
                        defaults.window_width = value.clamp(540, 16384);
                    }
                }
                "window_height" => {
                    if let Ok(value) = value.trim().parse::<u32>() {
                        defaults.window_height = value.clamp(360, 16384);
                    }
                }
                _ => {}
            }
        }

        defaults
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let directory = path
            .parent()
            .ok_or_else(|| std::io::Error::other("No configuration directory"))?;
        fs::create_dir_all(directory)?;

        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        writeln!(
            file,
            "wrap={}\nlines={}\nsmooth={}\nfont_size={}\nwindow_width={}\nwindow_height={}",
            self.wrap,
            self.lines,
            self.smooth,
            self.font_size,
            self.window_width,
            self.window_height
        )?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|error| error.error)?;

        Ok(())
    }
}

pub fn path() -> PathBuf {
    // Elevated runs must not create root-owned settings in the invoking user's home.
    #[cfg(unix)]
    if unsafe { libc::geteuid() } == 0 && std::env::var_os("SUDO_UID").is_some() {
        return PathBuf::from("/root/.config/rpad/settings.conf");
    }

    #[cfg(windows)]
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(crate::theme::user_home);

    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::theme::user_home().join(".config"));

    base.join("rpad/settings.conf")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fullscreen_and_maximized_keep_previous_window_dimensions() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = AppWindow::new().unwrap();
        ui.window().set_size(slint::LogicalSize::new(1100., 780.));
        let normal = Settings::capture(&ui);

        for fullscreen in [false, true] {
            ui.window().set_fullscreen(fullscreen);
            ui.window().set_maximized(!fullscreen);
            ui.window().set_size(slint::LogicalSize::new(1920., 1080.));
            ui.set_font_pixels(24);
            let captured = Settings::capture_preserving_window(&ui, &normal);
            assert_eq!((captured.window_width, captured.window_height), (1100, 780));
            assert_eq!(captured.font_size, 24);
        }
        ui.window().set_fullscreen(false);
        ui.window().set_maximized(false);
        ui.window().set_size(slint::LogicalSize::new(1200., 850.));
        let restored = Settings::capture_preserving_window(&ui, &normal);
        assert_eq!((restored.window_width, restored.window_height), (1200, 850));
    }

    #[test]
    fn window_dimensions_restore_and_survive_hiding() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = AppWindow::new().unwrap();
        ui.show().unwrap();
        ui.window().set_size(slint::LogicalSize::new(1130., 810.));

        let expected = Settings::capture(&ui);
        assert_eq!((expected.window_width, expected.window_height), (1130, 810));
        ui.hide().unwrap();
        let closed = Settings::capture(&ui);
        assert_eq!((closed.window_width, closed.window_height), (1130, 810));

        let restored = AppWindow::new().unwrap();
        expected.apply(&restored);
        restored.show().unwrap();
        let reopened = Settings::capture(&restored);
        assert_eq!((reopened.window_width, reopened.window_height), (1130, 810));
        restored.hide().unwrap();
    }

    #[test]
    fn round_trip_and_invalid_values_preserve_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rpad/settings.conf");
        let expected = Settings {
            wrap: true,
            lines: false,
            smooth: false,
            font_size: 23,
            window_width: 1140,
            window_height: 820,
        };
        expected.save(&path).unwrap();
        let defaults = Settings {
            wrap: false,
            lines: true,
            smooth: true,
            font_size: 16,
            window_width: 980,
            window_height: 700,
        };
        assert_eq!(Settings::load(&path, defaults), expected);

        fs::write(
            &path,
            "wrap=invalid\nfont_size=999\nwindow_width=1\nwindow_height=oops\nunknown=true",
        )
        .unwrap();
        let restored = Settings::load(&path, expected.clone());
        assert_eq!(restored.font_size, 36);
        assert_eq!(restored.window_width, 540);
        assert_eq!(restored.window_height, 820);
        assert_eq!(restored.wrap, expected.wrap);
        assert_eq!(restored.lines, expected.lines);
    }
}
