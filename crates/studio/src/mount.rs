//! The assembly mode: a circuit wired one wire at a time, in the order the
//! file lists them, each one picked out with its two pins, and checked off
//! once done.
//!
//! Progress is saved next to the circuit, as the customizer saves presets
//! next to a design: `grogu.c2fc` keeps it in `grogu.mount.json`. Wires are
//! named there by their ends, so that progress survives wires moved in the
//! file.

use std::collections::BTreeSet;
use std::f32::consts::{PI, TAU};
use std::path::{Path, PathBuf};

use c2f_circuit::Circuit;
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::camera::Camera;
use crate::inspect;

const VERSION: u32 = 1;

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct File {
    version: u32,
    /// The wire on screen, from 0.
    step: usize,
    /// The wires done, by name.
    done: BTreeSet<String>,
}

#[derive(Debug, Default)]
pub struct Mount {
    /// The assembly mode is on.
    pub active: bool,
    /// The wire on screen, from 0.
    pub step: usize,
    /// The wires done, by name.
    done: BTreeSet<String>,
    /// Where progress is saved: none for a circuit not saved yet.
    path: Option<PathBuf>,
    /// The last problem reading or saving it.
    pub problem: Option<String>,
}

/// The file that keeps the progress of the circuit at `circuit`.
pub fn progress_path(circuit: &Path) -> PathBuf {
    circuit.with_extension("mount.json")
}

/// A name for each wire, from its ends, as `xiao.D3 -> amp.BCLK`; a wire
/// repeated between the same pins gets `#2`, `#3`…
pub fn wire_names(circuit: &Circuit) -> Vec<String> {
    let mut names: Vec<String> = Vec::with_capacity(circuit.wires.len());
    for wire in &circuit.wires {
        let base = format!("{} -> {}", circuit.end_name(wire.from), circuit.end_name(wire.to));
        let count = names.iter().filter(|n| **n == base || n.starts_with(&format!("{base} #"))).count();
        names.push(if count == 0 { base } else { format!("{base} #{}", count + 1) });
    }
    names
}

impl Mount {
    /// Reads the progress of the circuit at `path`, or starts afresh.
    pub fn open(&mut self, path: Option<&Path>) {
        self.active = false;
        self.step = 0;
        self.done.clear();
        self.problem = None;
        self.path = path.map(progress_path);
        let Some(file) = &self.path else { return };
        match std::fs::read_to_string(file) {
            Ok(text) => match serde_json::from_str::<File>(&text) {
                Ok(read) => {
                    self.step = read.step;
                    self.done = read.done;
                }
                Err(err) => self.problem = Some(format!("Cannot read {}: {err}", file.display())),
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => self.problem = Some(format!("Cannot read {}: {err}", file.display())),
        }
    }

    /// The circuit was saved under a new name: its progress goes along.
    pub fn moved(&mut self, path: &Path) {
        self.path = Some(progress_path(path));
        self.save();
    }

    pub fn is_done(&self, names: &[String], index: usize) -> bool {
        names.get(index).is_some_and(|name| self.done.contains(name))
    }

    /// How many of the wires are done.
    pub fn done_count(&self, names: &[String]) -> usize {
        names.iter().filter(|name| self.done.contains(*name)).count()
    }

    pub fn set_done(&mut self, names: &[String], index: usize, done: bool) {
        let Some(name) = names.get(index) else { return };
        let changed = if done { self.done.insert(name.clone()) } else { self.done.remove(name) };
        if changed {
            self.save();
        }
    }

    /// Goes to wire `step`, within the `count` wires.
    pub fn go(&mut self, step: usize, count: usize) {
        let step = step.min(count.saturating_sub(1));
        if step != self.step {
            self.step = step;
            self.save();
        }
    }

    /// The wire on screen is done: check it off and go to the next.
    pub fn next(&mut self, names: &[String]) {
        self.set_done(names, self.step, true);
        self.go(self.step + 1, names.len());
    }

    pub fn previous(&mut self, count: usize) {
        self.go(self.step.saturating_sub(1), count);
    }

    /// Back to the first wire, none done.
    pub fn restart(&mut self) {
        self.done.clear();
        self.step = 0;
        self.save();
    }

    fn save(&mut self) {
        let Some(path) = &self.path else { return };
        let file = File { version: VERSION, step: self.step, done: self.done.clone() };
        let text = serde_json::to_string_pretty(&file).expect("progress serializes") + "\n";
        let temp = path.with_extension("json.tmp");
        self.problem = std::fs::write(&temp, text)
            .and_then(|()| std::fs::rename(&temp, path))
            .err()
            .map(|err| format!("Cannot save the assembly progress to {}: {err}", path.display()));
    }
}

/// What to do at wire `index`: "Blue wire: xiao D3 (GPIO4, A3) ➡ amp BCLK
/// (bit clock)".
pub fn instruction(circuit: &Circuit, index: usize) -> String {
    let wire = &circuit.wires[index];
    let color = match &wire.color {
        Some(color) => {
            let mut chars = color.chars();
            chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect::<String>()) + " wire"
        }
        None => "Wire".to_string(),
    };
    format!("{color}: {} ➡ {}", inspect::end_text(circuit, wire.from), inspect::end_text(circuit, wire.to))
}

/// A camera looking at both ends of a wire from above, across the line
/// between them, turned as little as it can be from `from`.
pub fn framing(a: Vec3, b: Vec3, from: &Camera, aspect: f32) -> Camera {
    let chord = b - a;
    let mut camera = *from;
    if chord.truncate().length() > 1e-3 {
        // Square to the chord, on the side nearer the camera's yaw.
        let across = chord.y.atan2(chord.x) + PI / 2.0;
        let turn = |yaw: f32| (yaw - from.yaw + PI).rem_euclid(TAU) - PI;
        let (left, right) = (turn(across), turn(across + PI));
        camera.yaw = from.yaw + if left.abs() <= right.abs() { left } else { right };
    }
    camera.pitch = 50f32.to_radians();
    // The pins, with room around them for the parts they sit on.
    let reach = Vec3::splat((chord.length() * 0.25).max(12.0));
    camera.frame(a.min(b) - reach, a.max(b) + reach, aspect);
    camera
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wiring::tests::circuit;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("code2form-mount-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("test.c2fc")
    }

    #[test]
    fn progress_is_saved_next_to_the_circuit_and_read_back() {
        let path = temp("progress");
        let _ = std::fs::remove_file(progress_path(&path));
        let names = wire_names(&circuit());
        let mut mount = Mount::default();
        mount.open(Some(&path));
        assert_eq!((mount.step, mount.done_count(&names)), (0, 0));
        mount.next(&names);
        mount.next(&names);
        mount.previous(names.len());
        assert_eq!(mount.step, 1);
        mount.next(&names);
        mount.next(&names);
        // The last wire done, the step stays on it.
        assert_eq!((mount.step, mount.done_count(&names)), (2, 3));
        mount.set_done(&names, 1, false);
        assert_eq!(progress_path(&path).file_name().unwrap(), "test.mount.json");

        let mut again = Mount::default();
        again.open(Some(&path));
        assert_eq!(again.problem, None);
        assert_eq!(again.step, 2);
        assert!(again.is_done(&names, 0) && !again.is_done(&names, 1) && again.is_done(&names, 2));
        assert!(!again.active, "the mode starts off");
        again.restart();
        again.open(Some(&path));
        assert_eq!((again.step, again.done_count(&names)), (0, 0));

        std::fs::write(progress_path(&path), "not json").unwrap();
        again.open(Some(&path));
        assert!(again.problem.as_deref().is_some_and(|p| p.starts_with("Cannot read")));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn wires_are_named_by_their_ends_and_explained() {
        let circuit = circuit();
        assert_eq!(wire_names(&circuit), ["a.A -> b.A", "a.A -> b.B", "a.B -> b.B"]);
        let mut twice = (*circuit).clone();
        twice.wires.push(twice.wires[0].clone());
        assert_eq!(wire_names(&twice)[3], "a.A -> b.A #2");
        assert_eq!(instruction(&circuit, 0), "Red wire: a A (first) ➡ b A (first)");
        assert_eq!(instruction(&circuit, 2), "Wire: a B ➡ b B");
    }

    #[test]
    fn the_camera_turns_square_to_the_wire_and_frames_both_pins() {
        let from = Camera { yaw: 0.3, ..Camera::default() };
        let (a, b) = (Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 40.0, 0.0));
        let camera = framing(a, b, &from, 1.5);
        // Along y: the camera looks from +x or -x, +x being nearer.
        assert!(camera.yaw.abs() < 1e-5, "{}", camera.yaw);
        assert!((camera.target - Vec3::new(0.0, 20.0, 0.0)).length() < 1e-4);
        let far = Camera { yaw: 3.0, ..from };
        assert!((framing(a, b, &far, 1.5).yaw - std::f32::consts::PI).abs() < 1e-4);
        // Both pins in front of the camera, within its field of view.
        for pin in [a, b] {
            let to = (pin - camera.eye()).normalize();
            assert!(to.dot(-camera.back()) > (camera.fov_y * 0.5).cos());
        }
    }
}
