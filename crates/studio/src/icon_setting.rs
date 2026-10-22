//! The app icon the user picked in the settings: kept between launches in
//! a small text file, shown in the Dock and the Finder on macOS, and as
//! the window icon elsewhere.

use std::path::{Path, PathBuf};

use eframe::egui;

use crate::app_icon::{self, Layout, Tint};

/// The per-user settings folder, except in screenshot runs, which neither
/// read nor change the choice.
pub fn store() -> Option<PathBuf> {
    if std::env::var_os("CODE2FORM_SCREENSHOT").is_some() {
        return None;
    }
    Some(crate::recent::settings_dir()?.join("app-icon.txt"))
}

/// The tint kept in `store`; Graphite when there is none or it is unknown.
pub fn load(store: Option<&Path>) -> Tint {
    store.and_then(|s| std::fs::read_to_string(s).ok()).and_then(|id| Tint::from_id(&id)).unwrap_or_default()
}

pub fn save(store: Option<&Path>, tint: Tint) -> std::io::Result<()> {
    let Some(store) = store else { return Ok(()) };
    if let Some(dir) = store.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(store, format!("{}\n", tint.id()))
}

/// The icon for eframe to show from the first frame: the Dock's on macOS,
/// the window's elsewhere.
pub fn icon_data(tint: Tint) -> egui::IconData {
    let (size, layout) = if cfg!(target_os = "macos") { (1024, Layout::MacIcon) } else { (256, Layout::Full) };
    let image = app_icon::render(tint, size, layout);
    egui::IconData { rgba: image.unmultiplied(), width: size as u32, height: size as u32 }
}

/// Shows `tint` from now on. On macOS the Dock shows it while the app runs
/// and the Finder (and the Dock, once the app quits) from the app bundle,
/// which only works when the app runs from a bundle it may change.
pub fn apply(ctx: &egui::Context, tint: Tint) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let _ = ctx;
        mac::show_in_dock(tint);
        mac::show_in_finder(tint)
    }
    #[cfg(not(target_os = "macos"))]
    {
        ctx.send_viewport_cmd(egui::ViewportCommand::Icon(Some(std::sync::Arc::new(icon_data(tint)))));
        Ok(())
    }
}

/// Gives the app bundle the icon picked earlier, as when a new version of
/// the app replaced the bundle. eframe shows it in the Dock.
pub fn restore(tint: Tint) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if tint != Tint::default() {
        return mac::show_in_finder(tint);
    }
    let _ = tint;
    Ok(())
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::rc::Retained;
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{
        NSApplication, NSBitmapImageRep, NSDeviceRGBColorSpace, NSImage, NSWorkspace, NSWorkspaceIconCreationOptions,
    };
    use objc2_foundation::{NSBundle, NSSize};

    use crate::app_icon::{self, Layout, Tint};

    /// The icon as an image macOS takes, 512 points of 1024 pixels.
    fn image(tint: Tint) -> Option<Retained<NSImage>> {
        let image = app_icon::render(tint, 1024, Layout::MacIcon);
        let size = image.size as isize;
        // SAFETY: no planes are given, so the representation allocates its
        // own buffer, of `size` rows of `size * 4` bytes, which is filled
        // below.
        let rep = unsafe {
            NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
                NSBitmapImageRep::alloc(),
                std::ptr::null_mut(),
                size,
                size,
                8,
                4,
                true,
                false,
                NSDeviceRGBColorSpace,
                size * 4,
                32,
            )
        }?;
        let data = rep.bitmapData();
        if data.is_null() {
            return None;
        }
        // SAFETY: the buffer holds `size * size * 4` bytes, as many as the
        // picture, premultiplied as the representation expects.
        unsafe { std::ptr::copy_nonoverlapping(image.premultiplied.as_ptr(), data, image.premultiplied.len()) };
        let points = image.size as f64 / 2.0;
        let icon = NSImage::initWithSize(NSImage::alloc(), NSSize::new(points, points));
        icon.addRepresentation(&rep);
        Some(icon)
    }

    pub fn show_in_dock(tint: Tint) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(icon) = image(tint) else { return };
        // SAFETY: an image is given, on the main thread.
        unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&icon)) };
    }

    /// Gives the app bundle `tint` as its custom icon, or takes the custom
    /// icon away for the default, so that the bundle's own shows again.
    pub fn show_in_finder(tint: Tint) -> Result<(), String> {
        let bundle = NSBundle::mainBundle().bundlePath();
        if !bundle.to_string().ends_with(".app") {
            // Not run from an app bundle, as with `cargo run`.
            return Ok(());
        }
        let icon = if tint == Tint::default() { None } else { Some(image(tint).ok_or("cannot draw the icon")?) };
        let done = NSWorkspace::sharedWorkspace().setIcon_forFile_options(
            icon.as_deref(),
            &bundle,
            NSWorkspaceIconCreationOptions(0),
        );
        if done { Ok(()) } else { Err(format!("cannot change the icon of {bundle}")) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn survives_a_save_and_load() {
        let dir = std::env::temp_dir().join(format!("code2form-icon-store-{}", std::process::id()));
        let store = dir.join("settings").join("app-icon.txt");
        assert_eq!(load(Some(&store)), Tint::Graphite, "no file yet");
        save(Some(&store), Tint::Teal).unwrap();
        assert_eq!(load(Some(&store)), Tint::Teal);
        std::fs::write(&store, "plaid\n").unwrap();
        assert_eq!(load(Some(&store)), Tint::Graphite, "an unknown tint");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn no_store_keeps_the_default() {
        assert_eq!(load(None), Tint::Graphite);
        save(None, Tint::Red).unwrap();
    }
}
