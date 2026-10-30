//! Writes the app icon's pictures as an iconset folder, for `iconutil` to
//! turn into the bundle's AppIcon.icns (see scripts/make-icon.sh), or as
//! the iPhone and iPad app's one picture, 1024 pixels square and opaque,
//! when given a `.png` file (see scripts/ios-icon.sh):
//!
//!   cargo run -p code2form --example app-icon -- AppIcon.iconset [tint]
//!   cargo run -p code2form --example app-icon -- AppIcon.png [tint]
//!
//! The tint is one of the settings' (graphite, the default, green, red…).
//! Every size is drawn at its own resolution rather than scaled down.

#[path = "../src/app_icon.rs"]
#[allow(dead_code)]
mod app_icon;

use std::path::Path;
use std::process::ExitCode;

use app_icon::{Layout, Tint};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (dir, tint) = match args.as_slice() {
        [dir] => (dir, Some(Tint::default())),
        [dir, tint] => (dir, Tint::from_id(tint)),
        _ => (&String::new(), None),
    };
    let Some(tint) = tint.filter(|_| !dir.is_empty()) else {
        let tints: Vec<&str> = Tint::ALL.iter().map(|t| t.id()).collect();
        eprintln!("usage: app-icon <folder.iconset | ios.png> [{}]", tints.join("|"));
        return ExitCode::from(2);
    };
    let dir = Path::new(dir);
    let written = if dir.extension().is_some_and(|e| e == "png") { write_ios(dir, tint) } else { write(dir, tint) };
    if let Err(err) = written {
        eprintln!("error: {}: {err}", dir.display());
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// The iOS icon: one picture, which Xcode scales for every device. The App
/// Store refuses one with an alpha channel, so it is written as RGB.
fn write_ios(file: &Path, tint: Tint) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(dir) = file.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let image = app_icon::render(tint, 1024, Layout::IosIcon);
    let rgb: Vec<u8> = image.unmultiplied().chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(file)?), 1024, 1024);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    encoder.write_header()?.write_image_data(&rgb)?;
    Ok(())
}

fn write(dir: &Path, tint: Tint) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir)?;
    for base in [16, 32, 128, 256, 512] {
        for (scale, suffix) in [(1, ""), (2, "@2x")] {
            let size = base * scale;
            let image = app_icon::render(tint, size, Layout::MacIcon);
            let file = std::fs::File::create(dir.join(format!("icon_{base}x{base}{suffix}.png")))?;
            let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size as u32, size as u32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
            encoder.write_header()?.write_image_data(&image.unmultiplied())?;
        }
    }
    Ok(())
}
