//! Code2Form: an editor for 3D designs written as code, with a live
//! 3D preview.

#[macro_use]
mod i18n;
mod animation;
mod app;
mod app_icon;
mod camera;
mod catalogue;
mod circuit;
mod console;
mod customizer;
mod editor;
mod examples;
mod find;
mod highlight;
mod icon_setting;
mod icons;
mod inspect;
mod mount;
mod open_events;
mod part_edit;
mod pause_edit;
mod pick;
mod prefs;
mod recent;
mod renderer;
mod rewire;
mod screenshot;
mod settings;
mod shortcuts;
mod tabs;
mod theme;
mod viewer;
mod window;
mod wiring;
mod worker;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui;

const USAGE: &str = "\
usage: code2form [file.c2f | file.scad | circuit.c2fc]...

Opens each file in a tab, the last one shown, in an editor with a live 3D
preview; the tabs open when the app was last closed come back first.

Set CODE2FORM_SCREENSHOT=out.png to save a picture of the window once
the first render is shown; the window then opens without taking focus.

The Open Recent list is kept in the user's settings folder, or in the
file CODE2FORM_RECENT_FILES names; screenshot runs without that variable
neither read nor change it. The settings and the open tabs are kept in
that folder too; screenshot runs leave them aside, and show the theme
CODE2FORM_APPEARANCE names (system, light or dark), the system's by
default, in the language CODE2FORM_LANGUAGE names (en or fr), English by
default.";

fn main() -> ExitCode {
    // Old macOS versions add a process serial number to apps the Finder starts.
    let paths: Vec<PathBuf> =
        std::env::args_os().skip(1).filter(|a| !a.to_string_lossy().starts_with("-psn_")).map(PathBuf::from).collect();
    if paths.iter().any(|p| p.as_os_str() == "-h" || p.as_os_str() == "--help") {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    // Scripted screenshots should not steal the keyboard from the user.
    let background = std::env::var_os("CODE2FORM_SCREENSHOT").is_some();
    let mut settings = settings::Settings::load(settings::store().as_deref());
    if background {
        let appearance = std::env::var("CODE2FORM_APPEARANCE").ok().and_then(|a| theme::Appearance::from_id(&a));
        settings.appearance = appearance.unwrap_or_default();
        let language = std::env::var("CODE2FORM_LANGUAGE").ok().and_then(|l| i18n::Language::from_id(&l));
        settings.language = language.unwrap_or_default();
    }
    i18n::set_language(settings.language);

    let saved = settings.window;
    let maximized = settings.window_maximized;
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Code2Form")
        .with_app_id("code2form")
        .with_inner_size(saved.map_or([1360.0, 860.0], |w| w.size))
        .with_min_inner_size(settings::MIN_WINDOW)
        .with_active(!background)
        // Also keeps eframe from showing its own logo in the Dock.
        .with_icon(icon_setting::icon_data(settings.icon));
    // macOS has no maximized windows, only large ones.
    if maximized && !cfg!(target_os = "macos") {
        viewport = viewport.with_maximized(true);
    }
    let screens = window::Screens::default();
    let known = screens.clone();
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        multisampling: 4,
        depth_buffer: 24,
        // Run once winit lists the screens, as the window opens.
        window_builder: Some(Box::new(move |viewport| match saved {
            Some(saved) => {
                let placed = window::place(saved, known.get().map_or(&[], Vec::as_slice));
                viewport.with_position(placed.position).with_inner_size(placed.size)
            }
            None => viewport,
        })),
        ..Default::default()
    };

    let mut builder = winit::event_loop::EventLoop::with_user_event();
    #[cfg(target_os = "macos")]
    if background {
        use winit::platform::macos::EventLoopBuilderExtMacOS;
        builder.with_activate_ignoring_other_apps(false);
    }
    let event_loop = match builder.build() {
        Ok(event_loop) => event_loop,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::FAILURE;
        }
    };
    // eframe tells of the errors that end it in its log.
    if log::set_logger(&ERRORS).is_ok() {
        log::set_max_level(log::LevelFilter::Error);
    }
    open_events::install();
    let app = eframe::create_native(
        "Code2Form",
        options,
        Box::new(move |cc| Ok(Box::new(app::StudioApp::new(cc, paths, settings)))),
        &event_loop,
    );
    let result = event_loop.run_app(&mut window::ScreenAware::new(app, screens));
    if let Err(err) = result {
        eprintln!("error: {err}");
        return ExitCode::FAILURE;
    }
    if ERRORS.failed.load(Ordering::Relaxed) { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

/// Prints eframe's errors, such as a window it could not open.
struct Errors {
    failed: AtomicBool,
}

static ERRORS: Errors = Errors { failed: AtomicBool::new(false) };

impl log::Log for Errors {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() == log::Level::Error && metadata.target().starts_with("eframe")
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("error: {}", record.args());
            self.failed.store(true, Ordering::Relaxed);
        }
    }

    fn flush(&self) {}
}
