use crate::{AppWindow, Theme};
use slint::{Color, ComponentHandle};
#[cfg(any(not(windows), test))]
use std::collections::HashMap;
use std::{fs, path::PathBuf, process::Command};

type Rgb = (u8, u8, u8);
#[derive(Clone, Debug, PartialEq)]
pub struct Colors {
    surface: Rgb,
    container: Rgb,
    elevated: Rgb,
    text: Rgb,
    muted: Rgb,
    accent: Rgb,
    on_accent: Rgb,
    outline: Rgb,
    pub dark: bool,
}

#[cfg_attr(windows, allow(dead_code))]
pub fn user_home() -> PathBuf {
    if let Ok(uid) = std::env::var("SUDO_UID") {
        if let Ok(passwd) = fs::read_to_string("/etc/passwd") {
            for line in passwd.lines() {
                let fields: Vec<_> = line.split(':').collect();
                if fields.len() > 5 && fields[2] == uid {
                    return fields[5].into();
                }
            }
        }
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp".into())
}

#[cfg(unix)]
pub fn prepare_sudo_display() {
    // sudo normally drops XDG_RUNTIME_DIR; use the invoking desktop session.
    if unsafe { libc::geteuid() } != 0 {
        return;
    }
    if let Ok(uid) = std::env::var("SUDO_UID") {
        let runtime = PathBuf::from(format!("/run/user/{uid}"));
        if runtime.is_dir() {
            if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
                std::env::set_var("XDG_RUNTIME_DIR", &runtime);
            }
            if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none()
                && runtime.join("bus").exists()
            {
                std::env::set_var(
                    "DBUS_SESSION_BUS_ADDRESS",
                    format!("unix:path={}/bus", runtime.display()),
                );
            }
            if std::env::var_os("WAYLAND_DISPLAY").is_none() {
                if let Ok(entries) = fs::read_dir(&runtime) {
                    use std::os::unix::fs::FileTypeExt;
                    let mut sockets: Vec<_> = entries
                        .flatten()
                        .filter(|e| {
                            e.file_name().to_string_lossy().starts_with("wayland-")
                                && e.file_type().is_ok_and(|t| t.is_socket())
                        })
                        .map(|e| e.file_name())
                        .collect();
                    sockets.sort();
                    if let Some(socket) = sockets.first() {
                        std::env::set_var("WAYLAND_DISPLAY", socket);
                    }
                }
            }
        }
        if std::env::var_os("XAUTHORITY").is_none() {
            let auth = user_home().join(".Xauthority");
            if auth.is_file() {
                std::env::set_var("XAUTHORITY", auth);
            } else if let Ok(entries) = fs::read_dir(&runtime) {
                let mut candidates: Vec<_> = entries
                    .flatten()
                    .filter(|e| {
                        e.file_name().to_string_lossy().starts_with("xauth_")
                            && e.file_type().is_ok_and(|t| t.is_file())
                    })
                    .map(|e| e.path())
                    .collect();
                candidates.sort();
                if let Some(auth) = candidates.first() {
                    std::env::set_var("XAUTHORITY", auth);
                }
            }
        }
    }
}

#[cfg(not(unix))]
pub fn prepare_sudo_display() {}

#[cfg(any(not(windows), test))]
fn parse_color(value: &str) -> Option<Rgb> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        if hex.len() == 6 {
            return Some((
                u8::from_str_radix(&hex[0..2], 16).ok()?,
                u8::from_str_radix(&hex[2..4], 16).ok()?,
                u8::from_str_radix(&hex[4..6], 16).ok()?,
            ));
        }
        return None;
    }
    let parts: Vec<u8> = value
        .split(',')
        .map(str::trim)
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    if parts.len() >= 3 {
        Some((parts[0], parts[1], parts[2]))
    } else {
        None
    }
}
fn light(c: Rgb) -> f32 {
    (c.0 as f32 * 0.2126 + c.1 as f32 * 0.7152 + c.2 as f32 * 0.0722) / 255.
}
fn mix(a: Rgb, b: Rgb, factor: f32) -> Rgb {
    let blend = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * factor).round() as u8;
    (blend(a.0, b.0), blend(a.1, b.1), blend(a.2, b.2))
}
fn contrast(accent: Rgb) -> Rgb {
    if light(accent) > 0.6 {
        (32, 20, 18)
    } else {
        (255, 255, 255)
    }
}
fn fallback(dark: bool, accent: Option<Rgb>) -> Colors {
    let accent = accent.unwrap_or(if dark { (255, 180, 168) } else { (180, 43, 35) });
    let base = if dark { (20, 18, 18) } else { (255, 249, 247) };
    let surface = mix(base, accent, 0.025);
    let container = mix(base, accent, if dark { 0.07 } else { 0.065 });
    Colors {
        surface,
        container,
        elevated: mix(base, accent, 0.13),
        text: if dark { (241, 225, 221) } else { (35, 25, 23) },
        muted: if dark { (211, 190, 185) } else { (106, 82, 77) },
        accent,
        on_accent: contrast(accent),
        outline: mix(base, accent, 0.25),
        dark,
    }
}
#[cfg(any(not(windows), test))]
fn kde_colors(input: &str) -> Option<Colors> {
    let mut section = "";
    let mut entries = HashMap::new();
    for line in input.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            section = &line[1..line.len() - 1];
        } else if let Some((key, value)) = line.split_once('=') {
            entries.insert((section, key.trim()), value.trim());
        }
    }
    let get = |section, key| entries.get(&(section, key)).and_then(|v| parse_color(v));
    let surface = get("Colors:View", "BackgroundNormal")?;
    let dark = light(surface) < 0.5;
    let mut colors = fallback(
        dark,
        get("General", "AccentColor").or_else(|| get("Colors:Selection", "BackgroundNormal")),
    );
    colors.surface = surface;
    colors.container = get("Colors:Window", "BackgroundNormal").unwrap_or(colors.container);
    colors.elevated = get("Colors:Button", "BackgroundNormal").unwrap_or(colors.elevated);
    colors.text = get("Colors:View", "ForegroundNormal").unwrap_or(colors.text);
    colors.muted = get("Colors:Window", "ForegroundNormal")
        .map(|c| mix(c, colors.container, 0.25))
        .unwrap_or(colors.muted);
    Some(colors)
}
#[cfg(not(windows))]
fn portal(key: &str) -> Option<String> {
    // Bounded call, made only on the theme worker, never on the UI thread.
    let result = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--timeout",
            "1",
            "--dest",
            "org.freedesktop.portal.Desktop",
            "--object-path",
            "/org/freedesktop/portal/desktop",
            "--method",
            "org.freedesktop.portal.Settings.Read",
            "org.freedesktop.appearance",
            key,
        ])
        .output()
        .ok()?;
    if result.status.success() {
        String::from_utf8(result.stdout).ok()
    } else {
        None
    }
}
#[cfg(windows)]
pub fn read() -> Colors {
    use std::os::windows::process::CommandExt;

    let output=Command::new("powershell.exe").args(["-NoProfile","-NonInteractive","-Command",
        r"$p=Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize' -ErrorAction SilentlyContinue; $d=Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\DWM' -ErrorAction SilentlyContinue; Write-Output $p.AppsUseLightTheme; Write-Output $d.AccentColor"
    ]).creation_flags(0x08000000).output().ok();
    let values = output
        .and_then(|v| String::from_utf8(v.stdout).ok())
        .unwrap_or_default();
    let mut lines = values.lines();
    let dark = lines.next().and_then(|v| v.trim().parse::<u32>().ok()) != Some(1);
    let accent = lines
        .next()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .map(|v| {
            (
                (v & 255) as u8,
                ((v >> 8) & 255) as u8,
                ((v >> 16) & 255) as u8,
            )
        });
    fallback(dark, accent)
}

#[cfg(not(windows))]
pub fn read() -> Colors {
    let config = if std::env::var_os("SUDO_UID").is_some() {
        user_home().join(".config")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| user_home().join(".config"))
    };
    if let Ok(input) = fs::read_to_string(config.join("kdeglobals")) {
        if let Some(colors) = kde_colors(&input) {
            return colors;
        }
    }
    let scheme = portal("color-scheme");
    let dark = !scheme.as_deref().is_some_and(|v| v.contains("uint32 2"));
    let accent = portal("accent-color").and_then(|v| {
        let numbers: Vec<f32> = v
            .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect();
        if numbers.len() == 3 && numbers.iter().all(|x| (0.0..=1.0).contains(x)) {
            Some((
                (numbers[0] * 255.) as u8,
                (numbers[1] * 255.) as u8,
                (numbers[2] * 255.) as u8,
            ))
        } else {
            None
        }
    });
    fallback(dark, accent)
}
pub fn apply(ui: &AppWindow, c: &Colors) {
    let rgb = |v: Rgb| Color::from_rgb_u8(v.0, v.1, v.2);
    let theme = ui.global::<Theme>();
    theme.set_surface(rgb(c.surface));
    theme.set_container(rgb(c.container));
    theme.set_elevated(rgb(c.elevated));
    theme.set_text(rgb(c.text));
    theme.set_muted(rgb(c.muted));
    theme.set_accent(rgb(c.accent));
    theme.set_on_accent(rgb(c.on_accent));
    theme.set_outline(rgb(c.outline));
    ui.set_dark_theme(c.dark);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kde_and_red_fallback() {
        let c=kde_colors("[Colors:View]\nBackgroundNormal=#1a1110\nForegroundNormal=241,223,220\n[Colors:Selection]\nBackgroundNormal=#ffb4a8\n[Colors:Window]\nBackgroundNormal=#271d1c\n").unwrap();
        assert_eq!(c.surface, (26, 17, 16));
        assert_eq!(c.accent, (255, 180, 168));
        assert!(c.dark);
        assert!(fallback(true, None).accent.0 > fallback(true, None).accent.1);
        assert!(kde_colors("[General]\nColorScheme=Breeze").is_none());
    }
}
