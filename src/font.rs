#![cfg_attr(windows, allow(dead_code, unused_imports))]
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, PartialEq)]
pub struct EditorFont {
    pub family: String,
    pub weight: i32,
}
fn ini<'a>(text: &'a str, group: &str, key: &str) -> Option<&'a str> {
    let mut section = "";
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            section = &line[1..line.len() - 1];
        } else if section == group {
            if let Some((name, value)) = line.split_once('=') {
                if name.trim() == key {
                    return Some(value.trim());
                }
            }
        }
    }
    None
}
fn qt_font(value: &str) -> Option<EditorFont> {
    let fields: Vec<_> = value.split(',').collect();
    let family = fields.first()?.trim();
    if family.is_empty() {
        return None;
    }
    let weight = fields
        .get(4)
        .and_then(|v| v.parse::<i32>().ok())
        .map(|w| {
            if w <= 99 {
                match w {
                    0..=25 => 300,
                    26..=57 => 400,
                    58..=68 => 600,
                    69..=80 => 700,
                    _ => 900,
                }
            } else {
                w.clamp(100, 900)
            }
        })
        .unwrap_or(400);
    Some(EditorFont {
        family: family.into(),
        weight,
    })
}
fn konsole_profile(dir: &Path, name: &str, depth: usize) -> Option<EditorFont> {
    if depth > 8 {
        return None;
    }
    let path = Path::new(name);
    if path.components().count() != 1 {
        return None;
    }
    let path = if path.extension().is_some() {
        dir.join(path)
    } else {
        dir.join(format!("{name}.profile"))
    };
    let text = fs::read_to_string(path).ok()?;
    ini(&text, "Appearance", "Font")
        .and_then(qt_font)
        .or_else(|| {
            let parent = ini(&text, "General", "Parent")?;
            if parent == "FALLBACK/" {
                None
            } else {
                konsole_profile(dir, parent, depth + 1)
            }
        })
}
fn simple_terminal(config: &Path, terminal: &str) -> Option<EditorFont> {
    let family = match terminal {
        "foot" => {
            let text = fs::read_to_string(config.join("foot/foot.ini")).ok()?;
            ini(&text, "main", "font")?
                .split(',')
                .next()?
                .split(':')
                .next()?
                .trim()
                .to_owned()
        }
        "kitty" => {
            let text = fs::read_to_string(config.join("kitty/kitty.conf")).ok()?;
            text.lines()
                .filter_map(|line| line.trim().split_once(char::is_whitespace))
                .find_map(|(key, value)| {
                    (key == "font_family").then(|| value.trim().trim_matches('"').to_owned())
                })?
        }
        "alacritty" => {
            let text = fs::read_to_string(config.join("alacritty/alacritty.toml")).ok()?;
            ini(&text, "font.normal", "family")?
                .trim_matches(['"', '\''])
                .to_owned()
        }
        _ => return None,
    };
    if family.is_empty() {
        None
    } else {
        Some(EditorFont {
            family,
            weight: 400,
        })
    }
}
fn resolve(font: EditorFont) -> Option<EditorFont> {
    let pattern = format!("{},monospace", font.family);
    let result = Command::new("fc-match")
        .args(["-f", "%{family[0]}", &pattern])
        .output()
        .ok()?;
    if !result.status.success() {
        return None;
    }
    let family = String::from_utf8(result.stdout).ok()?.trim().to_owned();
    if family.is_empty() {
        None
    } else {
        Some(EditorFont {
            family,
            weight: font.weight,
        })
    }
}
pub fn detect_in(config: &Path, data: &Path, terminal: &str) -> Option<EditorFont> {
    // A terminal that launched rpad takes precedence over unrelated installed configs.
    for name in ["foot", "kitty", "alacritty"] {
        if terminal.to_lowercase().contains(name) {
            if let Some(font) = simple_terminal(config, name) {
                return Some(font);
            }
        }
    }
    let kde = fs::read_to_string(config.join("kdeglobals")).unwrap_or_default();
    if let Ok(konsole) = fs::read_to_string(config.join("konsolerc")) {
        if let Some(name) = ini(&konsole, "Desktop Entry", "DefaultProfile") {
            if let Some(font) = konsole_profile(&data.join("konsole"), name, 0) {
                return Some(font);
            }
            // A Konsole profile without Font inherits KDE's fixed-width font.
            if let Some(font) = ini(&kde, "General", "fixed").and_then(qt_font) {
                return Some(font);
            }
        }
    }
    for name in ["kitty", "foot", "alacritty"] {
        if let Some(font) = simple_terminal(config, name) {
            return Some(font);
        }
    }
    ini(&kde, "General", "fixed").and_then(qt_font)
}
#[cfg(windows)]
pub fn detect() -> Option<EditorFont> {
    Some(EditorFont {
        family: "Cascadia Mono".into(),
        weight: 400,
    })
}

#[cfg(not(windows))]
pub fn detect() -> Option<EditorFont> {
    let home = crate::theme::user_home();
    let sudo = std::env::var_os("SUDO_UID").is_some();
    let config = if sudo {
        home.join(".config")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
    };
    let data = if sudo {
        home.join(".local/share")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
    };
    let terminal = format!(
        "{} {} {}",
        std::env::var("TERM_PROGRAM").unwrap_or_default(),
        std::env::var("TERM").unwrap_or_default(),
        std::env::var("TERMINAL").unwrap_or_default()
    );
    let configured = detect_in(&config, &data, &terminal);
    configured.and_then(resolve).or_else(|| {
        resolve(EditorFont {
            family: "monospace".into(),
            weight: 400,
        })
    })
}
fn kde_ui_font(config: &Path) -> Option<EditorFont> {
    let text = fs::read_to_string(config.join("kdeglobals")).ok()?;

    ini(&text, "General", "font").and_then(qt_font)
}

#[cfg(not(windows))]
pub fn detect_ui() -> Option<EditorFont> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::theme::user_home().join(".config"));

    if let Some(font) = kde_ui_font(&config) {
        return Some(font);
    }

    let gtk = fs::read_to_string(config.join("gtk-3.0/settings.ini")).unwrap_or_default();
    let pango = ini(&gtk, "Settings", "gtk-font-name")
        .map(str::to_owned)
        .or_else(|| {
            let output = Command::new("gsettings")
                .args(["get", "org.gnome.desktop.interface", "font-name"])
                .output()
                .ok()?;

            if !output.status.success() {
                return None;
            }

            Some(
                String::from_utf8(output.stdout)
                    .ok()?
                    .trim()
                    .trim_matches('\'')
                    .to_owned(),
            )
        })?;
    let mut words: Vec<_> = pango.split_whitespace().collect();

    if words.last().is_some_and(|word| word.parse::<f32>().is_ok()) {
        words.pop();
    }

    let family = words.join(" ");

    (!family.is_empty()).then_some(EditorFont {
        family,
        weight: 400,
    })
}

#[cfg(windows)]
pub fn detect_ui() -> Option<EditorFont> {
    // Wine can expose the host home through HOME and its Z: drive.
    if let Ok(home) = std::env::var("HOME") {
        if home.starts_with('/') {
            if let Some(font) = kde_ui_font(&PathBuf::from(format!("Z:{home}/.config"))) {
                return Some(font);
            }
        }
    }

    #[repr(C)]
    struct LogFont {
        metrics: [i32; 5],
        attributes: [u8; 8],
        family: [u16; 32],
    }

    #[link(name = "user32")]
    extern "system" {
        fn SystemParametersInfoW(
            action: u32,
            parameter: u32,
            data: *mut std::ffi::c_void,
            flags: u32,
        ) -> i32;
    }

    let mut font = LogFont {
        metrics: [0; 5],
        attributes: [0; 8],
        family: [0; 32],
    };
    let result = unsafe {
        SystemParametersInfoW(
            0x001f,
            std::mem::size_of::<LogFont>() as u32,
            (&mut font as *mut LogFont).cast(),
            0,
        )
    };

    if result == 0 {
        return None;
    }

    let end = font
        .family
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(font.family.len());
    let family = String::from_utf16(&font.family[..end]).ok()?;

    (!family.is_empty()).then_some(EditorFont {
        family,
        weight: font.metrics[4].clamp(100, 900),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn desktop_font_uses_general_font_instead_of_terminal_font() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("kdeglobals"),
            "[General]\nfont=Inter,12,-1,5,400\nfixed=JetBrains Mono,12,-1,5,400\n",
        )
        .unwrap();
        assert_eq!(
            super::kde_ui_font(directory.path()).unwrap().family,
            "Inter"
        );
    }
    use super::*;
    #[test]
    fn konsole_inherits_kde_fixed_and_explicit_profile_wins() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("konsole")).unwrap();
        fs::write(
            root.join("konsolerc"),
            "[Desktop Entry]\nDefaultProfile=Main.profile\n",
        )
        .unwrap();
        fs::write(
            root.join("kdeglobals"),
            "[General]\nfixed=JetBrains Mono NL SemiBold,13,-1,5,600,0\n",
        )
        .unwrap();
        fs::write(
            root.join("konsole/Main.profile"),
            "[General]\nParent=FALLBACK/\n",
        )
        .unwrap();
        assert_eq!(
            detect_in(root, root, "dumb").unwrap(),
            EditorFont {
                family: "JetBrains Mono NL SemiBold".into(),
                weight: 600
            }
        );
        fs::write(
            root.join("konsole/Main.profile"),
            "[Appearance]\nFont=Fira Code,12,-1,5,400\n",
        )
        .unwrap();
        assert_eq!(detect_in(root, root, "").unwrap().family, "Fira Code");
        fs::create_dir_all(root.join("foot")).unwrap();
        fs::write(root.join("foot/foot.ini"), "[main]\nfont=Iosevka:size=12\n").unwrap();
        assert_eq!(detect_in(root, root, "foot").unwrap().family, "Iosevka");
    }
}
