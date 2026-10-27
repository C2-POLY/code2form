//! The whole window driven as a user would: the buttons found by the names
//! screen readers get, the view clicked at the model's points, every frame
//! laid out as the app lays it out.

use std::time::{Duration, Instant};

use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Pos2, Rect};
use glam::Vec3;

use super::{Stores, StudioApp};
use crate::recent::RecentFiles;
use crate::settings::{Settings, Workspace};

/// The window's size, in points.
const SCREEN: egui::Vec2 = egui::vec2(1400.0, 900.0);

/// What a screen reader is told of a widget: its name, where it is and
/// whether it can be used.
#[derive(Clone, Debug)]
struct Named {
    label: String,
    rect: Rect,
    enabled: bool,
    /// A label's text rather than a control's name.
    text: bool,
}

/// The app in a window of its own, without the user's files.
struct Window {
    ctx: egui::Context,
    app: StudioApp,
    time: f64,
    /// The named widgets of the last frame.
    named: Vec<Named>,
}

impl Window {
    /// The app on `code`, in `workspace`, once it rendered.
    fn new(code: &str, workspace: Workspace) -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let stores = Stores { session: None, settings: None, conversations: None };
        let mut settings = Settings::load(None);
        settings.workspace = workspace;
        let app = StudioApp::with(&ctx, None, stores, RecentFiles::default(), settings, Vec::new());
        let mut window = Window { ctx, app, time: 0.0, named: Vec::new() };
        // A first launch shows a new file's start screen over the view.
        window.app.doc.start = None;
        window.app.doc.text = code.into();
        window.app.render_now();
        window.rendered();
        window
    }

    /// One frame with `events`, `modifiers` held.
    fn frame(&mut self, events: Vec<Event>, modifiers: Modifiers) {
        // A quarter of a second a frame: no two clicks make a double-click,
        // and the camera's flights end in a few frames.
        self.time += 0.25;
        let input = egui::RawInput {
            events,
            modifiers,
            time: Some(self.time),
            predicted_dt: 1.0 / 60.0,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)),
            ..Default::default()
        };
        let app = &mut self.app;
        let output = self.ctx.run(input, |ctx| {
            app.receive();
            app.shortcuts(ctx);
            app.panels(ctx);
        });
        if let Some(update) = output.platform_output.accesskit_update {
            // What the macOS screen reader checks first, and dies of: the
            // widget with the keyboard is among the nodes.
            assert!(
                update.nodes.iter().any(|(id, _)| *id == update.focus),
                "the keyboard is on a widget out of sight"
            );
            self.named = update
                .nodes
                .iter()
                .filter_map(|(_, node)| {
                    // A control's name, or a label's text.
                    let text = node.label().is_none();
                    let label = node.label().or(node.value())?.to_string();
                    let b = node.bounds()?;
                    let rect = Rect::from_min_max(Pos2::new(b.x0 as f32, b.y0 as f32), Pos2::new(b.x1 as f32, b.y1 as f32));
                    Some(Named { label, rect, enabled: !node.is_disabled(), text })
                })
                .collect();
        }
    }

    /// A few frames without input, for the camera to settle.
    fn settle(&mut self) {
        for _ in 0..8 {
            self.frame(Vec::new(), Modifiers::NONE);
        }
    }

    /// Waits for the render under way, then shows it.
    fn rendered(&mut self) {
        let start = Instant::now();
        while self.app.doc.busy() {
            assert!(start.elapsed() < Duration::from_secs(30), "no reply");
            std::thread::sleep(Duration::from_millis(5));
            self.app.receive();
        }
        assert!(!self.app.doc.failed, "the code does not render:\n{}", self.app.doc.text);
        self.settle();
    }

    /// A click of `button` at `at`, `modifiers` held: the move, the press and
    /// the release, each in a frame.
    fn click_at(&mut self, at: Pos2, button: PointerButton, modifiers: Modifiers) {
        let press = |pressed| Event::PointerButton { pos: at, button, pressed, modifiers };
        self.frame(vec![Event::PointerMoved(at)], modifiers);
        self.frame(vec![press(true)], modifiers);
        self.frame(vec![press(false)], modifiers);
        // A menu opened by the click is measured in a first frame, greyed,
        // and shows in the next.
        self.frame(Vec::new(), Modifiers::NONE);
    }

    /// The widget named `label` (a menu entry's name ends with its keys),
    /// the last laid out of that name: a menu over the window comes after.
    fn named(&self, label: &str) -> Named {
        let is = |n: &&Named| n.label == label || n.label.starts_with(&format!("{label} "));
        // A control of that name before a mere text.
        let found = self.named.iter().rev().filter(|n| !n.text).find(is).or_else(|| self.named.iter().rev().find(is));
        found.cloned().unwrap_or_else(|| {
            let names: Vec<&str> = self.named.iter().map(|n| n.label.as_str()).collect();
            panic!("no widget named {label:?}; the window has {names:?}")
        })
    }

    /// Clicks the widget named `label`, which must be enabled.
    fn click(&mut self, label: &str) {
        let widget = self.named(label);
        assert!(widget.enabled, "{label:?} is disabled");
        self.click_at(widget.rect.center(), PointerButton::Primary, Modifiers::NONE);
    }

    /// Clicks the control named `label` lowest in the window: of two of a
    /// name, the panel's tab rather than the top bar's button.
    fn click_lowest(&mut self, label: &str) {
        let is = |n: &&Named| !n.text && (n.label == label || n.label.starts_with(&format!("{label} ")));
        let lowest = self.named.iter().filter(is).max_by(|a, b| a.rect.top().total_cmp(&b.rect.top()));
        let at = lowest.unwrap_or_else(|| panic!("no control named {label:?}")).rect.center();
        self.click_at(at, PointerButton::Primary, Modifiers::NONE);
    }

    fn has(&self, label: &str) -> bool {
        self.named.iter().any(|n| n.label == label || n.label.starts_with(&format!("{label} ")))
    }

    /// Where the point `p` of the model shows.
    fn screen(&self, p: [f32; 3]) -> Pos2 {
        self.app.doc.viewer.to_screen(self.app.view_rect, Vec3::from_array(p)).expect("in front of the eye")
    }

    /// The code of the statements selected.
    fn selected(&self) -> Vec<&str> {
        self.app.selected_ranges().unwrap_or_default().into_iter().map(|r| &self.app.doc.text[r]).collect()
    }
}

const TWO_CUBES: &str = "cube(10);\ntranslate([20, 0, 0]) cube(10);\n";

#[test]
fn a_right_click_in_the_view_selects_and_deletes() {
    let mut w = Window::new(TWO_CUBES, Workspace::Model);
    let top = w.screen([25.0, 5.0, 10.0]);
    w.click_at(top, PointerButton::Secondary, Modifiers::NONE);
    assert_eq!(w.selected(), ["cube(10);"]);
    assert!(w.app.selected_ranges().unwrap()[0].start > 0, "the second cube");
    w.click("Delete");
    assert_eq!(w.app.doc.text, "cube(10);\n");
}

#[test]
fn a_right_click_in_the_tree_duplicates() {
    let mut w = Window::new(TWO_CUBES, Workspace::Model);
    let row = w.named("translate").rect.center();
    w.click_at(row, PointerButton::Secondary, Modifiers::NONE);
    assert_eq!(w.selected(), ["translate([20, 0, 0]) cube(10);"]);
    w.click("Duplicate");
    assert_eq!(w.app.doc.text, "cube(10);\ntranslate([20, 0, 0]) cube(10);\ntranslate([20, 0, 0]) cube(10);\n");
}

#[test]
fn the_toolbar_rounds_a_selected_cube() {
    let mut w = Window::new("cube(10);\n", Workspace::Model);
    assert!(!w.named("Fillet").enabled, "nothing selected yet");
    let top = w.screen([5.0, 5.0, 10.0]);
    w.click_at(top, PointerButton::Primary, Modifiers::NONE);
    assert_eq!(w.selected(), ["cube(10);"]);
    w.click("Fillet");
    w.click("Apply");
    assert!(w.app.doc.text.contains("rounded_cube("), "{}", w.app.doc.text);
}

#[test]
fn a_rectangle_sketched_and_extruded() {
    let mut w = Window::new("", Workspace::Model);
    w.click("Sketch");
    w.click("XY");
    assert!(w.app.doc.sketch.active(), "the sketch mode");
    w.settle();
    w.click("Rectangle");
    let (a, b) = (w.screen([0.0, 0.0, 0.0]), w.screen([20.0, 10.0, 0.0]));
    w.click_at(a, PointerButton::Primary, Modifiers::NONE);
    w.click_at(b, PointerButton::Primary, Modifiers::NONE);
    w.click("Extrude…");
    assert!(w.has("Extrude"), "the toolbar's Extrude");
    w.settle();
    w.click("Extrude");
    assert!(!w.app.doc.sketch.active(), "the sketch is written");
    let code = &w.app.doc.text;
    assert!(code.contains("module sketch_profile1()") && code.contains("linear_extrude"), "{code}");
    w.rendered();
}

#[test]
fn esc_leaves_a_sketch_and_the_model_toolbar_comes_back() {
    let mut w = Window::new(TWO_CUBES, Workspace::Model);
    w.click("Sketch");
    w.click("XY");
    assert!(w.has("Finish sketch"));
    w.click("Cancel");
    assert!(!w.app.doc.sketch.active());
    assert!(w.has("Cube") && !w.has("Finish sketch"));
    // Escape too, from a new sketch.
    w.click("Sketch");
    w.click("XY");
    w.frame(vec![Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }], Modifiers::NONE);
    w.settle();
    assert!(!w.app.doc.sketch.active(), "Esc leaves the sketch");
    assert_eq!(w.app.doc.text, TWO_CUBES);
}

fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
}

#[test]
fn a_cube_put_down_from_the_toolbar_and_undone() {
    let mut w = Window::new("", Workspace::Model);
    w.click("Cube");
    let at = w.screen([10.0, 10.0, 0.0]);
    w.click_at(at, PointerButton::Primary, Modifiers::NONE);
    assert_eq!(w.app.doc.text, "translate([10, 10, 5]) cube(10, center = true);\n");
    w.rendered();
    assert_eq!(w.selected().len(), 1);
    // Cmd+Z, with the code hidden: the cube goes.
    w.frame(vec![key(Key::Z, Modifiers::COMMAND)], Modifiers::COMMAND);
    assert_eq!(w.app.doc.text, "");
    w.frame(vec![key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT)], Modifiers::COMMAND | Modifiers::SHIFT);
    assert_eq!(w.app.doc.text, "translate([10, 10, 5]) cube(10, center = true);\n");
}

#[test]
fn show_in_code_goes_to_the_object_line() {
    let mut w = Window::new(TWO_CUBES, Workspace::Model);
    let top = w.screen([25.0, 5.0, 10.0]);
    w.click_at(top, PointerButton::Secondary, Modifiers::NONE);
    w.click("Show in Code");
    assert_eq!(w.app.settings.workspace, Workspace::Code);
    w.settle();
    assert_eq!(w.app.doc.editor.cursor.map(|(line, _)| line), Some(1), "on the second line");
}

#[test]
fn the_code_workspace_tree_tab_selects() {
    let mut w = Window::new(TWO_CUBES, Workspace::Code);
    w.click("Tree");
    let row = w.named("translate").rect.center();
    w.click_at(row, PointerButton::Primary, Modifiers::NONE);
    assert_eq!(w.selected(), ["translate([20, 0, 0]) cube(10);"]);
    w.click_lowest("Code");
    assert_eq!(w.app.panels.code_tab, super::workspace::CodeTab::Code, "back on the code");
}

#[test]
fn the_inspector_shows_parameters_and_comes_back_to_properties() {
    let mut w = Window::new("size = 10; // [1:50]\ncube(size);\n", Workspace::Model);
    w.click("Parameters");
    assert!(w.has("size"), "the parameter's name");
    let top = w.screen([5.0, 5.0, 10.0]);
    w.click_at(top, PointerButton::Primary, Modifiers::NONE);
    assert_eq!(w.selected(), ["cube(size);"]);
    assert_eq!(w.app.panels.inspector, super::workspace::InspectorTab::Properties);
}

#[test]
fn a_plane_picked_in_the_view() {
    let mut w = Window::new("", Workspace::Model);
    // From the top, the XZ and YZ squares are seen edge on: the XY one.
    let &(_, yaw, pitch) = crate::viewer::PRESETS.iter().find(|p| p.0 == "Top").unwrap();
    w.app.doc.viewer.look_from(yaw, pitch);
    w.settle();
    w.click("Sketch");
    assert!(w.has("Sketch on"), "the planes' bar");
    assert!(!w.app.doc.sketch.active());
    let at = w.screen([5.0, 5.0, 0.0]);
    w.click_at(at, PointerButton::Primary, Modifiers::NONE);
    let (plane, _) = w.app.doc.sketch.plane_and_cut().expect("a sketch");
    assert_eq!(plane, c2f_sketch::Plane::Xy);
}

#[test]
fn a_sketch_on_a_face_cuts_its_object() {
    let mut w = Window::new(TWO_CUBES, Workspace::Model);
    w.click("Sketch");
    let top = w.screen([25.0, 5.0, 10.0]);
    w.click_at(top, PointerButton::Primary, Modifiers::NONE);
    let (plane, cut) = w.app.doc.sketch.plane_and_cut().expect("a sketch");
    let c2f_sketch::Plane::Face { origin, normal, .. } = plane else { panic!("{plane:?}") };
    assert_eq!((origin[2], normal), (10.0, [0.0, 0.0, 1.0]));
    let cut = cut.expect("the cube clicked");
    assert_eq!(&w.app.doc.text[cut], "cube(10);");
}

#[test]
fn esc_gives_up_choosing_a_plane() {
    let mut w = Window::new(TWO_CUBES, Workspace::Model);
    w.click("Sketch");
    assert!(w.has("Sketch on"));
    w.frame(vec![key(Key::Escape, Modifiers::NONE)], Modifiers::NONE);
    w.settle();
    assert!(!w.has("Sketch on") && !w.app.doc.sketch.active());
    // Sketch twice gives up too.
    w.click("Sketch");
    w.click("Sketch");
    assert!(!w.has("Sketch on"));
}

#[test]
fn a_new_file_with_the_code_hidden() {
    // In the Model workspace, and on the Code workspace's Tree tab.
    let mut w = Window::new(TWO_CUBES, Workspace::Model);
    w.click("+");
    assert_eq!(w.app.shelf.len(), 2);
    let mut w = Window::new(TWO_CUBES, Workspace::Code);
    w.click("Tree");
    w.click("+");
    assert_eq!(w.app.shelf.len(), 2);
    // With the code on screen, the new file's editor takes the keyboard
    // (from Cmd+N: a click on + is a click away from it, which egui takes
    // as leaving it).
    w.click_lowest("Code");
    assert_eq!(w.app.panels.code_tab, super::workspace::CodeTab::Code);
    w.frame(vec![key(Key::N, Modifiers::COMMAND)], Modifiers::COMMAND);
    w.frame(Vec::new(), Modifiers::NONE);
    assert_eq!(w.app.shelf.len(), 3);
    assert_eq!(w.ctx.memory(|m| m.focused()), Some(w.app.doc.editor.id()));
}
