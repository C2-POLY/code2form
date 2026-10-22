//! Code2Form: an editor for 3D designs written as code, with a live
//! 3D preview.

mod app;
mod app_icon;
mod camera;
mod catalogue;
mod circuit;
mod code_edit;
mod console;
mod customizer;
mod editor;
mod fillet;
mod find;
mod gizmo;
mod highlight;
mod icon_setting;
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
mod select;
mod settings;
mod sketch_view;
mod tabs;
mod theme;
mod viewer;
mod wiring;
mod worker;

use std::path::PathBuf;
use std::process::ExitCode;

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
that folder too; screenshot runs leave them aside.";

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
    let settings = settings::Settings::load(settings::store().as_deref());

    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Code2Form")
            .with_app_id("code2form")
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([760.0, 480.0])
            .with_active(!background)
            // Also keeps eframe from showing its own logo in the Dock.
            .with_icon(icon_setting::icon_data(settings.icon)),
        renderer: eframe::Renderer::Glow,
        multisampling: 4,
        depth_buffer: 24,
        ..Default::default()
    };
    #[cfg(target_os = "macos")]
    if background {
        options.event_loop_builder = Some(Box::new(|builder| {
            use winit::platform::macos::EventLoopBuilderExtMacOS;
            builder.with_activate_ignoring_other_apps(false);
        }));
    }
    open_events::install();
    let result = eframe::run_native(
        "Code2Form",
        options,
        Box::new(move |cc| Ok(Box::new(app::StudioApp::new(cc, paths, settings)))),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
