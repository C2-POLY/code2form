//! The app icon the user picked in the settings: shown in the Dock and
//! the Finder on macOS, and as the window icon elsewhere.

use eframe::egui;

use crate::app_icon::{self, Layout, Tint};
use crate::i18n::tr;

impl Tint {
    /// As the settings list it, in the interface's language.
    pub fn name(self) -> &'static str {
        match self {
            Tint::Graphite => tr("Graphite", "Graphite"),
            Tint::Green => tr("Green", "Vert"),
            Tint::Red => tr("Red", "Rouge"),
            Tint::Orange => tr("Orange", "Orange"),
            Tint::Amber => tr("Amber", "Ambre"),
            Tint::Teal => tr("Teal", "Sarcelle"),
            Tint::Cyan => tr("Cyan", "Cyan"),
            Tint::Indigo => tr("Indigo", "Indigo"),
            Tint::Violet => tr("Violet", "Violet"),
            Tint::Pink => tr("Pink", "Rose"),
        }
    }
}

/// The icon for eframe to show from the first frame: the Dock's on macOS,
/// the window's elsewhere.
pub fn icon_data(tint: Tint) -> egui::IconData {
    let (size, layout) = if cfg!(target_os = "macos") { (1024, Layout::MacIcon) } else { (256, Layout::Full) };
    let image = app_icon::render(tint, size, layout);
    egui::IconData { rgba: image.unmultiplied(), width: image.size as u32, height: image.size as u32 }
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
    use super::tr;

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
        let icon = if tint == Tint::default() { None } else { Some(image(tint).ok_or(tr("cannot draw the icon", "impossible de dessiner l’icône"))?) };
        let done = NSWorkspace::sharedWorkspace().setIcon_forFile_options(
            icon.as_deref(),
            &bundle,
            NSWorkspaceIconCreationOptions(0),
        );
        if done { Ok(()) } else { Err(tr!("cannot change the icon of {}", "impossible de changer l’icône de {}", bundle)) }
    }
}
