//! The tray icon's image: the policy's `branding.trayIcon`, else its logo, else furnish's own
//! armchair. One-color icons are template images on macOS (the menu bar tints them); on Windows
//! and Linux they turn black on a light taskbar and white on a dark one.

use std::{fs, io::Read, path::Path};

use anyhow::{Context, Result, bail};
use base64::Engine;
use resvg::{tiny_skia, usvg};

/// furnish's own glyph, the same one on every system (rendered here, so no PNGs to keep in sync).
const OWN: &[u8] = include_bytes!("../icons/tray-template.svg");

/// Pixels per side: Windows shows 16 to 32 (100 to 200 % scaling), macOS 22 points at 2x, Linux
/// panels about 22 to 24, at 2x on HiDPI screens.
pub const SIZE: u32 = if cfg!(windows) { 32 } else { 48 };

/// Larger files aren't read: the collection decides what the file is.
const LIMIT: u64 = 1024 * 1024;

#[derive(Debug, PartialEq)]
pub struct Icon {
    /// SIZE x SIZE, RGBA, not premultiplied.
    pub rgba: Vec<u8>,
    /// macOS: a template image, which the menu bar colors itself.
    pub template: bool,
}

/// The icon for these branding files. `template`: one-color icons become template images (macOS);
/// otherwise they turn white (`dark_bar`) or black. A file that can't be shown is logged and skipped.
pub fn choose(tray_icon: Option<&Path>, logo: Option<&Path>, template: bool, dark_bar: bool) -> Icon {
    let render = |kind: &str, path: &Path| {
        render_file(path, SIZE)
            .inspect_err(|err| eprintln!("warning: tray icon: can't show the branding {kind} {}: {err:#}", path.display()))
            .ok()
    };
    let one_color = |rgba: Vec<u8>| match template {
        true => Icon { rgba, template: true },
        false => Icon { rgba: tint(rgba, if dark_bar { 255 } else { 0 }), template: false },
    };
    if let Some(rgba) = tray_icon.and_then(|p| render("trayIcon", p)) {
        return one_color(rgba);
    }
    if let Some(rgba) = logo.and_then(|p| render("logo", p)) {
        return Icon { rgba, template: false };
    }
    one_color(render_svg(OWN, SIZE).expect("furnish's tray icon renders"))
}

/// An SVG, PNG, JPEG or WebP file as a `size` x `size` RGBA image, fitted and centered.
pub fn render_file(path: &Path, size: u32) -> Result<Vec<u8>> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    let mime = match ext.as_str() {
        "svg" => None,
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        _ => bail!("use SVG, PNG, JPEG or WebP"),
    };
    let mut bytes = Vec::new();
    fs::File::open(path)?.take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        bail!("larger than 1 MB");
    }
    match mime {
        None => render_svg(&bytes, size),
        // The SVG renderer decodes and scales pictures too.
        Some(mime) => {
            let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{size}" height="{size}"><image width="{size}" height="{size}" preserveAspectRatio="xMidYMid meet" xlink:href="data:{mime};base64,{data}"/></svg>"#
            );
            render_svg(svg.as_bytes(), size)
        }
    }
}

fn render_svg(data: &[u8], size: u32) -> Result<Vec<u8>> {
    let tree = usvg::Tree::from_data(data, &usvg::Options::default())?;
    let (w, h) = (tree.size().width(), tree.size().height());
    let side = size as f32;
    let scale = (side / w).min(side / h);
    let transform = tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, (side - w * scale) / 2.0, (side - h * scale) / 2.0);
    let mut pixmap = tiny_skia::Pixmap::new(size, size).context("no image of that size")?;
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let rgba: Vec<u8> = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    if rgba.chunks_exact(4).all(|p| p[3] == 0) {
        bail!("nothing to see in it");
    }
    Ok(rgba)
}

/// Every pixel in one gray (0 black, 255 white), keeping its transparency.
fn tint(mut rgba: Vec<u8>, gray: u8) -> Vec<u8> {
    for p in rgba.chunks_exact_mut(4) {
        p[..3].fill(gray);
    }
    rgba
}

/// Windows: the taskbar is dark unless `SystemUsesLightTheme` (HKCU\…\Themes\Personalize) is
/// set and not 0. Without it (Windows 10 before 1903) the taskbar is dark.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn windows_bar_is_dark(system_uses_light_theme: Option<u32>) -> bool {
    system_uses_light_theme.is_none_or(|v| v == 0)
}

/// Linux: GNOME's top bar is always dark. Elsewhere the freedesktop portal's `color-scheme`
/// (1 dark, 2 light) follows the desktop's colors; without one, assume a dark panel, the default
/// of most desktops.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn linux_bar_is_dark(current_desktop: &str, color_scheme: Option<u32>) -> bool {
    current_desktop.split(':').any(|d| d.eq_ignore_ascii_case("gnome")) || color_scheme != Some(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&rgba.repeat((w * h) as usize)).unwrap();
        writer.finish().unwrap();
        out
    }

    fn pixel(rgba: &[u8], x: u32, y: u32) -> [u8; 4] {
        let i = ((y * SIZE + x) * 4) as usize;
        rgba[i..i + 4].try_into().unwrap()
    }

    const RED_SQUARE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#f00"/></svg>"##;

    #[test]
    fn renders_svg_at_the_tray_size() {
        let dir = tempfile::tempdir().unwrap();
        let rgba = render_file(&write(dir.path(), "logo.svg", RED_SQUARE), SIZE).unwrap();
        assert_eq!(rgba.len(), (SIZE * SIZE * 4) as usize);
        assert_eq!(pixel(&rgba, SIZE / 2, SIZE / 2), [255, 0, 0, 255]);
    }

    #[test]
    fn fits_and_centers_wide_images() {
        let dir = tempfile::tempdir().unwrap();
        let wide = write(dir.path(), "wide.png", &png(40, 10, [0, 0, 255, 255]));
        let rgba = render_file(&wide, SIZE).unwrap();
        assert_eq!(rgba.len(), (SIZE * SIZE * 4) as usize);
        assert_eq!(pixel(&rgba, SIZE / 2, SIZE / 2), [0, 0, 255, 255]);
        assert_eq!(pixel(&rgba, SIZE / 2, 0)[3], 0, "transparent above");
        assert_eq!(pixel(&rgba, SIZE / 2, SIZE - 1)[3], 0, "transparent below");
    }

    #[test]
    fn refuses_what_it_cant_show() {
        let dir = tempfile::tempdir().unwrap();
        assert!(render_file(&write(dir.path(), "logo.gif", b"GIF89a"), SIZE).is_err());
        assert!(render_file(&write(dir.path(), "logo.svg", b"not svg"), SIZE).is_err());
        assert!(render_file(&write(dir.path(), "empty.svg", br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>"#), SIZE).is_err());
        assert!(render_file(&dir.path().join("missing.png"), SIZE).is_err());
        let big = vec![b' '; LIMIT as usize + 1];
        assert!(render_file(&write(dir.path(), "big.svg", &big), SIZE).is_err());
    }

    #[test]
    fn own_icon_is_one_color_and_tinted_for_the_taskbar() {
        let light = choose(None, None, false, false);
        let dark = choose(None, None, false, true);
        assert!(!light.template && !dark.template);
        let opaque = |icon: &Icon| icon.rgba.chunks_exact(4).filter(|p| p[3] > 0).map(|p| [p[0], p[1], p[2]]).collect::<Vec<_>>();
        assert!(!opaque(&light).is_empty());
        assert!(opaque(&light).iter().all(|p| *p == [0, 0, 0]));
        assert!(opaque(&dark).iter().all(|p| *p == [255, 255, 255]));
        // Same glyph: same transparency.
        let alpha = |icon: &Icon| icon.rgba.chunks_exact(4).map(|p| p[3]).collect::<Vec<_>>();
        assert_eq!(alpha(&light), alpha(&dark));
        assert!(choose(None, None, true, false).template);
    }

    #[test]
    fn tray_icon_before_logo_before_own() {
        let dir = tempfile::tempdir().unwrap();
        let logo = write(dir.path(), "logo.svg", RED_SQUARE);
        let tray = write(dir.path(), "tray.png", &png(8, 8, [0, 0, 0, 255]));
        let broken = write(dir.path(), "broken.svg", b"<svg");

        // The tray icon: one color, tinted (or a template on macOS).
        let icon = choose(Some(&tray), Some(&logo), false, true);
        assert_eq!(pixel(&icon.rgba, SIZE / 2, SIZE / 2), [255, 255, 255, 255]);
        assert!(choose(Some(&tray), Some(&logo), true, true).template);
        // The logo: in its colors, never a template.
        let icon = choose(None, Some(&logo), true, true);
        assert_eq!((pixel(&icon.rgba, SIZE / 2, SIZE / 2), icon.template), ([255, 0, 0, 255], false));
        // Unreadable files fall through to the next.
        assert_eq!(choose(Some(&broken), Some(&logo), true, true), choose(None, Some(&logo), true, true));
        assert_eq!(choose(Some(&broken), Some(&broken), false, false), choose(None, None, false, false));
    }

    #[test]
    fn taskbar_theme() {
        assert!(!windows_bar_is_dark(Some(1)));
        assert!(windows_bar_is_dark(Some(0)));
        assert!(windows_bar_is_dark(None));
        assert!(!linux_bar_is_dark("KDE", Some(2)));
        assert!(linux_bar_is_dark("KDE", Some(1)));
        assert!(linux_bar_is_dark("XFCE", None));
        assert!(linux_bar_is_dark("XFCE", Some(0)));
        assert!(linux_bar_is_dark("ubuntu:GNOME", Some(2)));
    }
}
