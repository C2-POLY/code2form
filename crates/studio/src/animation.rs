//! Playing a design that reads `$t`, the animation's time, as the
//! reference's View > Animate does: `steps` frames from `$t = 0` up to just
//! before 1, shown `fps` times a second. Each frame is a render of its own;
//! the frames rendered stay in memory, so that the second time round plays
//! at full speed however heavy the model. An edit to the code or a value
//! throws them away.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use c2f_eval::customizer::Literal;

use crate::worker::Scene;

pub const FPS: std::ops::RangeInclusive<u32> = 1..=60;
pub const STEPS: std::ops::RangeInclusive<u32> = 2..=1000;

/// What the frames rendered were rendered from: the code and the
/// customizer values.
type Source = (String, Vec<(String, Literal)>);

pub struct Animation {
    /// The last render read `$t`: the bar shows.
    pub animated: bool,
    pub playing: bool,
    pub fps: u32,
    pub steps: u32,
    /// The frame on screen, or about to be.
    pub frame: u32,
    /// When the next frame is due, while playing.
    next_at: Option<Instant>,
    /// The frames rendered, by number, and what they were rendered from.
    frames: HashMap<u32, Scene>,
    source: Option<Source>,
    /// The render on its way, and the frame it is of.
    pending: Option<(u64, u32)>,
}

impl Default for Animation {
    fn default() -> Self {
        Animation {
            animated: false,
            playing: false,
            fps: 15,
            steps: 30,
            frame: 0,
            next_at: None,
            frames: HashMap::new(),
            source: None,
            pending: None,
        }
    }
}

/// What the app does next for the animation.
#[derive(Debug, PartialEq)]
pub enum Step {
    /// Nothing until `Duration` has passed.
    Wait(Duration),
    /// Shows this frame, rendered before.
    Show(Box<Scene>),
    /// Renders the current frame.
    Render,
}

impl Animation {
    /// `$t` for the current frame.
    pub fn t(&self) -> f64 {
        f64::from(self.frame) / f64::from(self.steps)
    }

    /// A render of the current frame was asked for, as `generation`.
    pub fn sent(&mut self, generation: u64) {
        self.pending = Some((generation, self.frame));
    }

    /// Takes the render `generation` of `source`: keeps its scene when it
    /// is a frame of the animation. Frames of another source are dropped.
    /// A failed render (no `scene`) leaves the animation as it is, as an
    /// unfinished line would. Returns whether the render is of the frame
    /// on screen: a frame asked for before a seek to a frame in memory is
    /// kept, not shown.
    pub fn rendered(&mut self, generation: u64, animated: bool, source: &Source, scene: Option<&Scene>) -> bool {
        let Some((sent, frame)) = self.pending else { return true };
        if generation < sent {
            return true;
        }
        self.pending = None;
        let Some(scene) = scene else { return true };
        self.animated = animated;
        if !animated {
            self.stop();
            self.frames.clear();
            return true;
        }
        if self.source.as_ref() != Some(source) {
            self.frames.clear();
            self.source = Some(source.clone());
        }
        self.frames.insert(frame, scene.clone());
        frame == self.frame
    }

    /// Drops the frames in memory: a file they were rendered from changed.
    pub fn forget(&mut self) {
        self.frames.clear();
    }

    /// What to do now, at `now`, while showing a render of `source` and
    /// with a render on its way when `busy`.
    pub fn tick(&mut self, now: Instant, busy: bool, source: &Source) -> Option<Step> {
        if !self.playing || !self.animated {
            return None;
        }
        let due = *self.next_at.get_or_insert(now);
        if now < due {
            return Some(Step::Wait(due - now));
        }
        // The frame on screen is still on its way.
        if busy {
            return None;
        }
        self.frame = (self.frame + 1) % self.steps;
        // Late frames are not caught up with: the next one is a frame's
        // time from now.
        self.next_at = Some(now.max(due) + self.period());
        Some(self.show(source))
    }

    /// Goes to `frame`: shows it when rendered, otherwise renders it.
    pub fn seek(&mut self, frame: u32, source: &Source) -> Step {
        self.frame = frame.min(self.steps - 1);
        self.show(source)
    }

    fn show(&self, source: &Source) -> Step {
        match self.frames.get(&self.frame) {
            Some(scene) if self.source.as_ref() == Some(source) => Step::Show(Box::new(scene.clone())),
            _ => Step::Render,
        }
    }

    pub fn play(&mut self) {
        self.playing = true;
        self.next_at = None;
    }

    pub fn stop(&mut self) {
        self.playing = false;
        self.next_at = None;
    }

    /// Sets how many frames make a turn: the frames rendered stand for
    /// other times, so they go.
    pub fn set_steps(&mut self, steps: u32) {
        let steps = steps.clamp(*STEPS.start(), *STEPS.end());
        if steps != self.steps {
            // The same time, as near as the new steps fall.
            self.frame = ((self.t() * f64::from(steps)).round() as u32) % steps;
            self.steps = steps;
            self.frames.clear();
        }
    }

    pub fn set_fps(&mut self, fps: u32) {
        self.fps = fps.clamp(*FPS.start(), *FPS.end());
    }

    /// How many frames are kept, for the bar.
    pub fn cached(&self) -> usize {
        self.frames.len()
    }

    fn period(&self) -> Duration {
        Duration::from_secs_f64(1.0 / f64::from(self.fps))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(text: &str) -> Source {
        (text.to_string(), Vec::new())
    }

    fn scene(triangles: usize) -> Scene {
        let mut scene = Scene::default();
        scene.stats.triangles = triangles;
        scene
    }

    /// The animation playing `src`, frame 0 rendered as generation 1.
    fn playing(src: &Source) -> Animation {
        let mut animation = Animation { steps: 4, ..Animation::default() };
        animation.sent(1);
        animation.rendered(1, true, src, Some(&scene(0)));
        animation.play();
        animation
    }

    #[test]
    fn frames_render_once_then_come_from_memory() {
        let src = source("rotate(360 * $t) cube(1);");
        let mut animation = playing(&src);
        let start = Instant::now();
        let period = animation.period();
        let at = |frames: u32| start + period * frames;
        // The first tick goes to frame 1 at once: it is not rendered yet.
        assert_eq!(animation.tick(at(0), false, &src), Some(Step::Render));
        assert_eq!((animation.frame, animation.t()), (1, 0.25));
        animation.sent(2);
        // Until it is back, nothing moves.
        assert_eq!(animation.tick(at(1), true, &src), None);
        animation.rendered(2, true, &src, Some(&scene(1)));
        assert_eq!(animation.tick(at(0), false, &src), Some(Step::Wait(period)));
        for (generation, frame) in [(3, 2), (4, 3)] {
            assert_eq!(animation.tick(at(frame - 1), false, &src), Some(Step::Render));
            assert_eq!(animation.frame, frame);
            animation.sent(generation);
            animation.rendered(generation, true, &src, Some(&scene(frame as usize)));
        }
        // Round again: every frame is in memory.
        assert_eq!(animation.tick(at(3), false, &src), Some(Step::Show(Box::new(scene(0)))));
        assert_eq!(animation.tick(at(4), false, &src), Some(Step::Show(Box::new(scene(1)))));
        assert_eq!(animation.cached(), 4);
    }

    #[test]
    fn an_edit_throws_the_frames_away() {
        let src = source("rotate(360 * $t) cube(1);");
        let mut animation = playing(&src);
        let edited = source("rotate(360 * $t) cube(2);");
        // Frame 0 was rendered from the code before the edit.
        assert_eq!(animation.seek(0, &edited), Step::Render);
        animation.sent(2);
        animation.rendered(2, true, &edited, Some(&scene(5)));
        assert_eq!(animation.cached(), 1);
        assert_eq!(animation.seek(0, &edited), Step::Show(Box::new(scene(5))));
    }

    #[test]
    fn a_design_that_stops_reading_t_stops_playing() {
        let src = source("rotate(360 * $t) cube(1);");
        let mut animation = playing(&src);
        animation.sent(2);
        animation.rendered(2, false, &source("cube(1);"), Some(&scene(0)));
        assert!(!animation.animated && !animation.playing);
        assert_eq!(animation.tick(Instant::now(), false, &src), None);
    }

    #[test]
    fn stale_renders_are_not_frames() {
        let src = source("rotate(360 * $t) cube(1);");
        let mut animation = playing(&src);
        animation.seek(2, &src);
        animation.sent(5);
        animation.rendered(4, true, &src, Some(&scene(9)));
        assert_eq!(animation.cached(), 1, "an older render is not frame 2");
    }

    #[test]
    fn a_frame_asked_for_before_a_seek_is_kept_not_shown() {
        let src = source("rotate(360 * $t) cube(1);");
        let mut animation = playing(&src);
        animation.stop();
        assert_eq!(animation.seek(2, &src), Step::Render);
        animation.sent(2);
        // Back to frame 0, in memory, before frame 2 comes.
        assert_eq!(animation.seek(0, &src), Step::Show(Box::new(scene(0))));
        assert!(!animation.rendered(2, true, &src, Some(&scene(2))));
        assert_eq!(animation.cached(), 2);
    }

    #[test]
    fn a_failed_render_leaves_the_animation_as_it_is() {
        let src = source("rotate(360 * $t) cube(1);");
        let mut animation = playing(&src);
        animation.sent(2);
        assert!(animation.rendered(2, false, &src, None));
        assert!(animation.animated && animation.playing);
        assert_eq!(animation.cached(), 1);
    }

    #[test]
    fn new_steps_keep_the_time() {
        let src = source("x = $t;");
        let mut animation = playing(&src);
        animation.seek(2, &src);
        animation.set_steps(10);
        assert_eq!((animation.frame, animation.steps, animation.cached()), (5, 10, 0));
        animation.set_steps(0);
        assert_eq!(animation.steps, 2);
    }
}
