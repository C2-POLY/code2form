//! Code2Form: an editor for 3D designs written as code, with a live
//! 3D preview.

#[macro_use]
mod i18n;
mod agent_confirm;
mod agent_markdown;
mod agent_panel;
mod agent_setup;
mod animation;
mod app;
mod app_icon;
mod camera;
mod catalogue;
mod circuit;
mod code_edit;
mod console;
mod customizer;
mod editor;
mod examples;
mod fillet;
mod find;
mod gizmo;
mod highlight;
mod icon_setting;
mod icons;
mod insert;
mod inspect;
mod mount;
mod outline;
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
mod shortcuts;
mod sketch_view;
mod tabs;
mod theme;
mod viewer;
mod window;
mod wiring;
mod worker;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};

use c2f_agent::guard::Policy;
use eframe::egui;

const USAGE: &str = "\
usage: code2form [file.c2f | file.scad | circuit.c2fc]...

Opens each file in a tab, the last one shown, in an editor with a live 3D
preview; the tabs open when the app was last closed come back first.

Set CODE2FORM_SCREENSHOT=out.png to save a picture of the window once
the first render is shown, then quit; the window opens without taking
focus.

The Open Recent list is kept in the user's settings folder, or in the
file CODE2FORM_RECENT_FILES names; screenshot runs without that variable
neither read nor change it. The settings and the open tabs are kept in
that folder too; screenshot runs leave them aside, and show the theme
CODE2FORM_APPEARANCE names (system, light or dark), the system's by
default, in the language CODE2FORM_LANGUAGE names (en or fr), English by
default, and the workspace CODE2FORM_WORKSPACE names (model or code), the
Code one by default.";

fn main() -> ExitCode {
    // An agent run by the studio (or by anyone, with the app's executable
    // and no c2f installed) serves the tools there, the live ones too, on
    // stdin and stdout, without a window.
    // Not `args()`, which panics on an argument that is not UTF-8.
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if let Some(rest) = studio_server_args(&args) {
        return mcp(rest);
    }
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
        let workspace = std::env::var("CODE2FORM_WORKSPACE").ok().and_then(|w| settings::Workspace::from_id(&w));
        settings.workspace = workspace.unwrap_or_default();
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

/// The arguments after `mcp --studio`, when the command line starts so.
fn studio_server_args(args: &[std::ffi::OsString]) -> Option<&[std::ffi::OsString]> {
    match args {
        [mcp, studio, rest @ ..] if mcp == "mcp" && studio == "--studio" => Some(rest),
        _ => None,
    }
}

/// What the server lets the agent do in `folder`: the `--policy` JSON the
/// engine gives (`args`), else `CODE2FORM_AGENT_POLICY`, else the settings
/// as `settings.txt` keeps them, as `c2f mcp --studio` reads them.
fn server_policy(args: &[std::ffi::OsString], folder: &Path) -> Policy {
    let given = args.iter().position(|a| a == "--policy").and_then(|at| args.get(at + 1)?.to_str().map(str::to_string));
    let spec = given.or_else(|| std::env::var("CODE2FORM_AGENT_POLICY").ok());
    match spec.and_then(|spec| agent_setup::AgentSettings::from_policy_json(&spec)) {
        Some((agent, libraries)) => agent.policy(folder, &libraries),
        None => Policy::from_settings_file(c2f_agent::guard::settings_file().as_deref(), Some(folder)),
    }
}

/// `code2form mcp --studio`: `c2f mcp --studio`, from the app itself.
fn mcp(args: &[std::ffi::OsString]) -> ExitCode {
    eprintln!("code2form mcp: Code2Form {} serving on stdin and stdout", env!("CARGO_PKG_VERSION"));
    let tools = match c2f_agent::Registry::builtin().with(Box::new(c2f_agent::studio_link::StudioProxy::new())) {
        Ok(tools) => tools,
        Err(err) => {
            eprintln!("code2form mcp: {err}");
            return ExitCode::FAILURE;
        }
    };
    // What the user lets the agent do: Claude Code starts this server in
    // the active file's folder (a scratch one for an untitled tab), its
    // scope; never "anywhere".
    let folder = std::env::current_dir().unwrap_or_else(|_| {
        let scratch = agent_setup::scratch_folder("default");
        let _ = std::fs::create_dir_all(&scratch);
        scratch
    });
    let tools = c2f_agent::guard::Guarded::registry(tools, server_policy(args, &folder));
    match c2f_agent::Server::new(tools).serve(std::io::stdin().lock(), std::io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("code2form mcp: {err}");
            ExitCode::FAILURE
        }
    }
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

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    #[test]
    fn the_server_s_arguments_may_be_any_bytes() {
        use std::ffi::OsString;
        let args = |list: &[&str]| list.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(super::studio_server_args(&args(&["mcp", "--studio"])), Some(&[][..]));
        let with = args(&["mcp", "--studio", "--policy", "{}"]);
        assert_eq!(super::studio_server_args(&with).map(<[_]>::len), Some(2));
        assert!(super::studio_server_args(&args(&["box.c2f"])).is_none());
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            // Not UTF-8: a file to open, no panic.
            let odd = vec![OsString::from_vec(vec![0x66, 0xff, 0x6f])];
            assert!(super::studio_server_args(&odd).is_none());
        }
    }

    #[test]
    fn the_server_takes_the_policy_it_is_given() {
        use std::ffi::OsString;
        let agent = crate::agent_setup::AgentSettings { allow_export: false, ask_first: true, ..Default::default() };
        // Absolute on every system: `/libs` has no drive on Windows.
        let (libs, work) = (std::env::temp_dir().join("libs"), std::env::temp_dir().join("work"));
        let json = agent.policy_json(std::slice::from_ref(&libs));
        let args = [OsString::from("--policy"), OsString::from(json)];
        let policy = super::server_policy(&args, &work);
        assert!(!policy.export && policy.ask_first);
        let scope = policy.scope.unwrap();
        assert_eq!((&scope.folder, &scope.read_only[0]), (&work, &libs));
    }

    /// The studio's agent calls the very tools `c2f mcp` serves.
    #[test]
    fn the_agent_s_tools_render_code() {
        let tools = c2f_agent::Registry::builtin();
        let listed = tools.list();
        assert!(listed.iter().any(|tool| tool["name"] == "render" && tool["inputSchema"]["properties"]["code"].is_object()));
        let content = tools.call("render", &json!({ "code": "cube([10, 20, 5]);" })).expect("the render tool").expect("a report");
        let report: Value = serde_json::from_str(content[0]["text"].as_str().expect("a text")).expect("JSON");
        assert_eq!(report["bounding_box"]["size"], json!([10.0, 20.0, 5.0]));
    }
}
