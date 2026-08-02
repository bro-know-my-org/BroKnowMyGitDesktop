use crate::app_error::AppError;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[cfg(target_os = "linux")]
use std::path::PathBuf;

#[cfg(target_os = "linux")]
use std::{
    env, fs,
    io::Read,
    os::fd::AsRawFd,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

#[cfg(target_os = "macos")]
use std::ffi::{c_char, c_void, CStr};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemFontFamily {
    family: String,
    monospaced: bool,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemFontPreferences {
    interface_family: Option<String>,
    code_family: Option<String>,
    interface_size: Option<f64>,
    code_size: Option<f64>,
    interface_weight: Option<u16>,
    code_weight: Option<u16>,
}

#[derive(Debug)]
struct FontPreference {
    family: String,
    size: Option<f64>,
    weight: Option<u16>,
}

#[tauri::command]
pub async fn get_system_font_preferences() -> Result<SystemFontPreferences, AppError> {
    tauri::async_runtime::spawn_blocking(platform_font_preferences)
        .await
        .map_err(|_| AppError::new("command_failed", "系统字体偏好读取任务未能完成。"))
}

#[tauri::command]
pub async fn list_system_fonts() -> Result<Vec<SystemFontFamily>, AppError> {
    tauri::async_runtime::spawn_blocking(collect_system_fonts)
        .await
        .map_err(|_| AppError::new("command_failed", "系统字体列表读取任务未能完成。"))
}

fn collect_system_fonts() -> Vec<SystemFontFamily> {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();

    let mut families = BTreeMap::<String, (String, bool)>::new();
    for face in database.faces() {
        for (family, _) in &face.families {
            let family = family.trim();
            if family.is_empty() {
                continue;
            }
            let key = family.to_lowercase();
            let entry = families
                .entry(key)
                .or_insert_with(|| (family.to_string(), false));
            entry.1 |= face.monospaced;
        }
    }

    let mut result = families
        .into_values()
        .map(|(family, monospaced)| SystemFontFamily { family, monospaced })
        .collect::<Vec<_>>();
    result.sort_by(|left, right| {
        left.family
            .to_lowercase()
            .cmp(&right.family.to_lowercase())
            .then_with(|| left.family.cmp(&right.family))
    });
    result
}

#[cfg(target_os = "linux")]
fn platform_font_preferences() -> SystemFontPreferences {
    let desktop = env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let kde_session = env::var_os("KDE_FULL_SESSION").is_some()
        || desktop
            .split(':')
            .any(|name| name.eq_ignore_ascii_case("kde"));

    if kde_session {
        if let Some(preferences) = kde_font_preferences() {
            return preferences;
        }
    }

    let installed_families = installed_font_family_keys();
    let interface = gsettings_font_preference("font-name", &installed_families);
    let code = gsettings_font_preference("monospace-font-name", &installed_families);
    SystemFontPreferences {
        interface_family: interface
            .as_ref()
            .map(|preference| preference.family.clone())
            .or_else(|| fontconfig_family("sans-serif")),
        code_family: code
            .as_ref()
            .map(|preference| preference.family.clone())
            .or_else(|| fontconfig_family("monospace")),
        interface_size: interface.as_ref().and_then(|preference| preference.size),
        code_size: code.as_ref().and_then(|preference| preference.size),
        interface_weight: interface.as_ref().and_then(|preference| preference.weight),
        code_weight: code.as_ref().and_then(|preference| preference.weight),
    }
}

#[cfg(target_os = "windows")]
fn platform_font_preferences() -> SystemFontPreferences {
    use std::{ffi::c_void, mem::size_of};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS,
    };

    let mut metrics = NONCLIENTMETRICSW::default();
    metrics.cbSize = size_of::<NONCLIENTMETRICSW>() as u32;
    let loaded = unsafe {
        SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            (&mut metrics as *mut NONCLIENTMETRICSW).cast::<c_void>(),
            0,
        ) != 0
    };
    let interface_family = loaded
        .then(|| utf16_font_family(&metrics.lfMessageFont.lfFaceName))
        .flatten()
        .or_else(|| Some("Segoe UI".to_string()));
    SystemFontPreferences {
        interface_family,
        // Windows has no user-configurable global fixed-pitch UI font.
        // Consolas is the native system code-font fallback available across
        // all currently supported Windows versions.
        code_family: Some("Consolas".to_string()),
        interface_size: loaded.then_some(metrics.lfMessageFont.lfHeight.unsigned_abs() as f64),
        code_size: Some(13.0),
        interface_weight: loaded.then_some(metrics.lfMessageFont.lfWeight.clamp(100, 900) as u16),
        code_weight: Some(400),
    }
}

#[cfg(target_os = "windows")]
fn utf16_font_family(value: &[u16]) -> Option<String> {
    let length = value.iter().position(|character| *character == 0)?;
    let family = String::from_utf16(&value[..length]).ok()?;
    sanitize_font_family(&family)
}

#[cfg(target_os = "macos")]
fn platform_font_preferences() -> SystemFontPreferences {
    const USER_FIXED_PITCH_FONT: u32 = 1;
    const SYSTEM_FONT: u32 = 2;

    let native_interface_family = core_text_ui_font_family(SYSTEM_FONT);
    let interface_family = native_interface_family.map(|family| {
        if family.starts_with('.') {
            // CoreText intentionally exposes the system UI family through a
            // private name. WebKit's public token is the supported way to use it.
            "-apple-system".to_string()
        } else {
            family
        }
    });
    SystemFontPreferences {
        interface_family,
        code_family: core_text_ui_font_family(USER_FIXED_PITCH_FONT),
        interface_size: core_text_ui_font_size(SYSTEM_FONT),
        code_size: core_text_ui_font_size(USER_FIXED_PITCH_FONT),
        interface_weight: Some(400),
        code_weight: Some(400),
    }
}

#[cfg(target_os = "macos")]
fn core_text_ui_font_size(font_type: u32) -> Option<f64> {
    type CoreFoundationRef = *const c_void;

    #[link(name = "CoreText", kind = "framework")]
    unsafe extern "C" {
        fn CTFontCreateUIFontForLanguage(
            font_type: u32,
            size: f64,
            language: CoreFoundationRef,
        ) -> CoreFoundationRef;
        fn CTFontGetSize(font: CoreFoundationRef) -> f64;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(value: CoreFoundationRef);
    }

    let font = unsafe { CTFontCreateUIFontForLanguage(font_type, 0.0, std::ptr::null()) };
    if font.is_null() {
        return None;
    }
    let size = unsafe { CTFontGetSize(font) };
    unsafe { CFRelease(font) };
    (size > 0.0).then_some(size)
}

#[cfg(target_os = "macos")]
fn core_text_ui_font_family(font_type: u32) -> Option<String> {
    type CoreFoundationRef = *const c_void;

    #[link(name = "CoreText", kind = "framework")]
    unsafe extern "C" {
        fn CTFontCreateUIFontForLanguage(
            font_type: u32,
            size: f64,
            language: CoreFoundationRef,
        ) -> CoreFoundationRef;
        fn CTFontCopyFamilyName(font: CoreFoundationRef) -> CoreFoundationRef;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(value: CoreFoundationRef);
        fn CFStringGetLength(value: CoreFoundationRef) -> isize;
        fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
        fn CFStringGetCString(
            value: CoreFoundationRef,
            buffer: *mut c_char,
            buffer_size: isize,
            encoding: u32,
        ) -> u8;
    }

    const UTF8_ENCODING: u32 = 0x0800_0100;
    let font = unsafe { CTFontCreateUIFontForLanguage(font_type, 0.0, std::ptr::null()) };
    if font.is_null() {
        return None;
    }
    let family = unsafe { CTFontCopyFamilyName(font) };
    if family.is_null() {
        unsafe { CFRelease(font) };
        return None;
    }
    let length = unsafe { CFStringGetLength(family) };
    let maximum_size = unsafe { CFStringGetMaximumSizeForEncoding(length, UTF8_ENCODING) };
    let result = if maximum_size < 0 || maximum_size > 4096 {
        None
    } else {
        let mut buffer = vec![0_u8; maximum_size as usize + 1];
        let copied = unsafe {
            CFStringGetCString(
                family,
                buffer.as_mut_ptr().cast::<c_char>(),
                buffer.len() as isize,
                UTF8_ENCODING,
            )
        };
        if copied == 0 {
            None
        } else {
            let value = unsafe { CStr::from_ptr(buffer.as_ptr().cast::<c_char>()) };
            value.to_str().ok().and_then(sanitize_font_family)
        }
    };
    unsafe {
        CFRelease(family);
        CFRelease(font);
    }
    result
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn platform_font_preferences() -> SystemFontPreferences {
    SystemFontPreferences::default()
}

#[cfg(target_os = "linux")]
fn kde_font_preferences() -> Option<SystemFontPreferences> {
    let config_path = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?
        .join("kdeglobals");
    const MAX_KDEGLOBALS_BYTES: u64 = 1024 * 1024;
    let mut contents = String::new();
    let mut file = fs::File::open(config_path).ok()?;
    file.by_ref()
        .take(MAX_KDEGLOBALS_BYTES + 1)
        .read_to_string(&mut contents)
        .ok()?;
    if contents.len() as u64 > MAX_KDEGLOBALS_BYTES {
        return None;
    }
    let interface = kde_general_font_preference(&contents, "font");
    let code = kde_general_font_preference(&contents, "fixed");
    if interface.is_none() && code.is_none() {
        return None;
    }
    Some(SystemFontPreferences {
        interface_family: interface
            .as_ref()
            .map(|preference| preference.family.clone())
            .or_else(|| fontconfig_family("sans-serif")),
        code_family: code
            .as_ref()
            .map(|preference| preference.family.clone())
            .or_else(|| fontconfig_family("monospace")),
        interface_size: interface.as_ref().and_then(|preference| preference.size),
        code_size: code.as_ref().and_then(|preference| preference.size),
        interface_weight: interface.as_ref().and_then(|preference| preference.weight),
        code_weight: code.as_ref().and_then(|preference| preference.weight),
    })
}

#[cfg(target_os = "linux")]
fn kde_general_font_preference(contents: &str, key: &str) -> Option<FontPreference> {
    let mut in_general_section = false;
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_general_section = line.eq_ignore_ascii_case("[General]");
            continue;
        }
        if !in_general_section || line.starts_with('#') {
            continue;
        }
        let Some((candidate_key, value)) = line.split_once('=') else {
            continue;
        };
        if candidate_key.trim() == key {
            let fields = value.split(',').map(str::trim).collect::<Vec<_>>();
            let family = sanitize_font_family(fields.first().copied().unwrap_or_default())?;
            let size = fields
                .get(1)
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|size| *size > 0.0)
                .map(|points| points * 4.0 / 3.0);
            let weight = fields
                .get(4)
                .and_then(|value| value.parse::<u16>().ok())
                .map(css_weight_from_qt);
            return Some(FontPreference {
                family,
                size,
                weight,
            });
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn gsettings_font_preference(
    key: &str,
    installed_families: &BTreeSet<String>,
) -> Option<FontPreference> {
    let mut command = Command::new("gsettings");
    command.args(["get", "org.gnome.desktop.interface", key]);
    let output = command_output_with_timeout(command, Duration::from_secs(2))?;
    if !output.status.success() || output.stdout.len() > 4096 {
        return None;
    }
    let raw = String::from_utf8(output.stdout).ok()?;
    let unquoted = raw
        .trim()
        .trim_matches(|character| character == '\'' || character == '"');
    let (description, size) = unquoted
        .rsplit_once(' ')
        .filter(|(_, size)| size.parse::<f64>().is_ok())
        .map_or((unquoted, None), |(description, size)| {
            (
                description,
                size.parse::<f64>().ok().map(|points| points * 4.0 / 3.0),
            )
        });
    let (family, style) = split_pango_family_and_style(description, installed_families);
    let style = style.to_ascii_lowercase();
    let weight = if style.contains("extra bold") || style.contains("extrabold") {
        800
    } else if style.contains("black") || style.contains("heavy") {
        900
    } else if style.contains("semi bold") || style.contains("demi bold") {
        600
    } else if style.contains("bold") {
        700
    } else if style.contains("medium") {
        500
    } else if style.contains("light") {
        300
    } else if style.contains("thin") {
        100
    } else {
        400
    };
    Some(FontPreference {
        family: sanitize_font_family(family)?,
        size,
        weight: Some(weight),
    })
}

#[cfg(target_os = "linux")]
fn css_weight_from_qt(weight: u16) -> u16 {
    match weight {
        0..=6 => 100,
        7..=18 => 200,
        19..=37 => 300,
        38..=53 => 400,
        54..=60 => 500,
        61..=68 => 600,
        69..=78 => 700,
        79..=84 => 800,
        _ => 900,
    }
}

#[cfg(target_os = "linux")]
fn fontconfig_family(generic: &str) -> Option<String> {
    let mut command = Command::new("fc-match");
    command.args(["--format=%{family}", generic]);
    let output = command_output_with_timeout(command, Duration::from_secs(2))?;
    if !output.status.success() || output.stdout.len() > 4096 {
        return None;
    }
    let raw = String::from_utf8(output.stdout).ok()?;
    sanitize_font_family(raw.split(',').next().unwrap_or_default())
}

#[cfg(target_os = "linux")]
fn command_output_with_timeout(mut command: Command, timeout: Duration) -> Option<Output> {
    const MAX_OUTPUT_BYTES: usize = 4096;
    command.stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let descriptor = stdout.as_raw_fd();
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    let mut output = Vec::new();
    let deadline = Instant::now() + timeout;
    loop {
        if !read_bounded_nonblocking(&mut stdout, &mut output, MAX_OUTPUT_BYTES) {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if !read_bounded_nonblocking(&mut stdout, &mut output, MAX_OUTPUT_BYTES) {
                    return None;
                }
                return Some(Output {
                    status,
                    stdout: output,
                    stderr: Vec::new(),
                });
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn read_bounded_nonblocking(
    reader: &mut impl Read,
    output: &mut Vec<u8>,
    maximum_bytes: usize,
) -> bool {
    let mut buffer = [0_u8; 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return true,
            Ok(length) => {
                if output.len() + length > maximum_bytes {
                    return false;
                }
                output.extend_from_slice(&buffer[..length]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return true,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return false,
        }
    }
}

#[cfg(target_os = "linux")]
fn split_pango_family_and_style<'a>(
    description: &'a str,
    installed_families: &BTreeSet<String>,
) -> (&'a str, String) {
    if let Some((family, style)) = description.split_once(',') {
        return (family.trim(), style.trim().to_string());
    }

    let mut family = description.trim();
    if installed_families.contains(&family.to_lowercase()) {
        return (family, String::new());
    }
    let mut styles = Vec::new();
    const STYLE_SUFFIXES: &[&str] = &[
        "Extra Bold",
        "Ultra Bold",
        "Semi Bold",
        "Demi Bold",
        "Extra Light",
        "Ultra Light",
        "Semi Light",
        "Demi Light",
        "ExtraBold",
        "UltraBold",
        "SemiBold",
        "DemiBold",
        "Condensed",
        "Expanded",
        "Oblique",
        "Italic",
        "Thin",
        "Light",
        "Book",
        "Regular",
        "Normal",
        "Medium",
        "Bold",
        "Heavy",
        "Black",
    ];
    loop {
        let lowercase_family = family.to_ascii_lowercase();
        let Some(style) = STYLE_SUFFIXES.iter().find(|style| {
            family.len() > style.len()
                && lowercase_family.ends_with(&style.to_ascii_lowercase())
                && family.as_bytes()[family.len() - style.len() - 1].is_ascii_whitespace()
        }) else {
            break;
        };
        styles.push(*style);
        family = family[..family.len() - style.len()].trim_end();
        if installed_families.contains(&family.to_lowercase()) {
            styles.reverse();
            return (family, styles.join(" "));
        }
    }
    (description.trim(), String::new())
}

#[cfg(target_os = "linux")]
fn installed_font_family_keys() -> BTreeSet<String> {
    let mut database = fontdb::Database::new();
    database.load_system_fonts();
    database
        .faces()
        .flat_map(|face| face.families.iter())
        .filter_map(|(family, _)| sanitize_font_family(family))
        .map(|family| family.to_lowercase())
        .collect()
}

fn sanitize_font_family(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 200 || value.chars().any(char::is_control) {
        return None;
    }
    Some(value.to_string())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::{kde_general_font_preference, split_pango_family_and_style};
    use std::collections::BTreeSet;

    #[test]
    fn reads_kde_font_families_from_general_section() {
        let config = r#"
[Icons]
font=Wrong Font,9
[General]
font=JetBrains Mono,10,-1,5,81,0,0,0,0,0,ExtraBold
fixed=Noto Sans Mono,11,-1,5,50,0,0,0,0,0
"#;

        let interface = kde_general_font_preference(config, "font").unwrap();
        assert_eq!(interface.family, "JetBrains Mono");
        assert_eq!(interface.size, Some(40.0 / 3.0));
        assert_eq!(interface.weight, Some(800));

        let code = kde_general_font_preference(config, "fixed").unwrap();
        assert_eq!(code.family, "Noto Sans Mono");
        assert_eq!(code.size, Some(44.0 / 3.0));
        assert_eq!(code.weight, Some(400));
    }

    #[test]
    fn separates_space_delimited_pango_weight_and_style() {
        let installed = BTreeSet::from([
            "arial black".to_string(),
            "cantarell".to_string(),
            "jetbrains mono".to_string(),
            "noto sans".to_string(),
        ]);
        assert_eq!(
            split_pango_family_and_style("Cantarell Bold Italic", &installed),
            ("Cantarell", "Bold Italic".to_string())
        );
        assert_eq!(
            split_pango_family_and_style("JetBrains Mono Extra Bold", &installed),
            ("JetBrains Mono", "Extra Bold".to_string())
        );
        assert_eq!(
            split_pango_family_and_style("Noto Sans", &installed),
            ("Noto Sans", String::new())
        );
        assert_eq!(
            split_pango_family_and_style("Arial Black", &installed),
            ("Arial Black", String::new())
        );
    }
}
