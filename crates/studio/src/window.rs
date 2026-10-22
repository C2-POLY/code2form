//! The window's size and place, kept between launches. A window left on a
//! screen that is no longer there opens centered on the main screen.
//!
//! eframe opens the window once winit's event loop runs, which is also the
//! first time winit lists the screens: [`ScreenAware`] notes them just
//! before, for the hook that places the window.

use std::sync::{Arc, OnceLock};

use eframe::UserEvent;
use eframe::egui::{self, Pos2, Rect, Vec2};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, StartCause, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::window::WindowId;

use crate::settings::Placement;

/// How much of the window's top must be on a screen, in points, for it to
/// count as shown: enough to grab it by the title bar.
const GRIP: Vec2 = Vec2::new(120.0, 24.0);

/// The screens' areas in points, the main screen first, once known.
pub type Screens = Arc<OnceLock<Vec<Rect>>>;

/// eframe's application, with the screens noted before it opens the window.
pub struct ScreenAware<A> {
    app: A,
    screens: Screens,
}

impl<A> ScreenAware<A> {
    pub fn new(app: A, screens: Screens) -> Self {
        ScreenAware { app, screens }
    }

    fn note_screens(&self, event_loop: &ActiveEventLoop) {
        self.screens.get_or_init(|| {
            let main = event_loop.primary_monitor();
            let others: Vec<_> = event_loop.available_monitors().filter(|m| main.as_ref() != Some(m)).collect();
            main.into_iter()
                .chain(others)
                .map(|monitor| {
                    let scale = monitor.scale_factor();
                    let position = monitor.position().to_logical::<f32>(scale);
                    let size = monitor.size().to_logical::<f32>(scale);
                    Rect::from_min_size(Pos2::new(position.x, position.y), Vec2::new(size.width, size.height))
                })
                .collect()
        });
    }
}

impl<A: ApplicationHandler<UserEvent>> ApplicationHandler<UserEvent> for ScreenAware<A> {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        if cause == StartCause::Init {
            self.note_screens(event_loop);
        }
        self.app.new_events(event_loop, cause);
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // eframe opens the window here.
        self.note_screens(event_loop);
        self.app.resumed(event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        self.app.user_event(event_loop, event);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        self.app.window_event(event_loop, window_id, event);
    }

    fn device_event(&mut self, event_loop: &ActiveEventLoop, device_id: DeviceId, event: DeviceEvent) {
        self.app.device_event(event_loop, device_id, event);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.app.about_to_wait(event_loop);
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.app.suspended(event_loop);
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        self.app.exiting(event_loop);
    }

    fn memory_warning(&mut self, event_loop: &ActiveEventLoop) {
        self.app.memory_warning(event_loop);
    }
}

/// Where to open a window last left at `saved`: there when enough of its
/// top is on one of `screens`, otherwise centered on the main screen. The
/// size never exceeds the screen. With no screens known, as left.
pub fn place(saved: Placement, screens: &[Rect]) -> Placement {
    let size = Vec2::from(saved.size);
    let top = Rect::from_min_size(Pos2::from(saved.position), Vec2::new(size.x, GRIP.y));
    let shown = |screen: &&Rect| {
        let part = screen.intersect(top);
        part.width() >= GRIP.x.min(size.x) && part.height() >= GRIP.y
    };
    let Some(main) = screens.first() else { return saved };
    if let Some(screen) = screens.iter().find(shown) {
        return Placement { size: size.min(screen.size()).into(), ..saved };
    }
    let size = size.min(main.size());
    let corner = (main.center() - size / 2.0).max(main.min);
    Placement { position: corner.into(), size: size.into() }
}

/// The window as it is now, in the screen's points, and whether it is
/// maximized; `None` while minimized or full screen, whose places are not
/// kept. macOS tells whether a window is maximized only as it opens, so
/// there it never counts as maximized and its size and place are kept.
///
/// The sizes are measured at the start of a frame with the zoom of the
/// frame before: right after the zoom changes, they are off.
pub fn current(ctx: &egui::Context) -> Option<(Placement, bool)> {
    let zoom = ctx.zoom_factor();
    ctx.input(|i| {
        let viewport = i.viewport();
        if viewport.minimized == Some(true) || viewport.fullscreen == Some(true) {
            return None;
        }
        let inner = viewport.inner_rect?;
        let corner = viewport.outer_rect?.min;
        let round = |v: Vec2| (v * zoom).round().into();
        let placement = Placement { position: round(corner.to_vec2()), size: round(inner.size()) };
        let maximized = !cfg!(target_os = "macos") && viewport.maximized == Some(true);
        Some((placement, maximized))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect::from_min_size(Pos2::new(x, y), Vec2::new(width, height))
    }

    fn placement(x: f32, y: f32, width: f32, height: f32) -> Placement {
        Placement { position: [x, y], size: [width, height] }
    }

    /// A laptop screen, and a larger one to its left, a little higher.
    fn screens() -> [Rect; 2] {
        [screen(0.0, 0.0, 1512.0, 982.0), screen(-2560.0, -200.0, 2560.0, 1440.0)]
    }

    #[test]
    fn a_window_on_a_screen_stays_there() {
        for saved in [placement(100.0, 50.0, 1360.0, 860.0), placement(-2000.0, -150.0, 1800.0, 1200.0)] {
            assert_eq!(place(saved, &screens()), saved);
        }
        // Mostly off the right edge, but its title bar can still be grabbed.
        let saved = placement(1300.0, 600.0, 1360.0, 860.0);
        assert_eq!(place(saved, &screens()), saved);
    }

    #[test]
    fn a_window_off_every_screen_comes_back_on_the_main_one() {
        let main = screens()[0];
        for saved in [
            // On a screen since unplugged.
            placement(3000.0, 100.0, 1360.0, 860.0),
            // Its title bar above the top of every screen.
            placement(200.0, -400.0, 1360.0, 860.0),
            // A sliver of its top left on the right edge.
            placement(1450.0, 100.0, 1360.0, 860.0),
        ] {
            let placed = place(saved, &screens());
            assert_eq!(placed.size, saved.size);
            let window = Rect::from_min_size(Pos2::from(placed.position), Vec2::from(placed.size));
            assert!(main.contains_rect(window), "{saved:?} placed at {placed:?}");
            assert_eq!(window.center(), main.center(), "centered");
        }
    }

    #[test]
    fn a_window_larger_than_its_screen_shrinks_to_it() {
        let saved = placement(4000.0, 0.0, 3000.0, 2000.0);
        assert_eq!(place(saved, &screens()), placement(0.0, 0.0, 1512.0, 982.0));
        let saved = placement(10.0, 10.0, 3000.0, 600.0);
        assert_eq!(place(saved, &screens()).size, [1512.0, 600.0]);
    }

    #[test]
    fn with_no_screens_known_the_window_is_left_alone() {
        let saved = placement(9000.0, 9000.0, 800.0, 600.0);
        assert_eq!(place(saved, &[]), saved);
    }
}
