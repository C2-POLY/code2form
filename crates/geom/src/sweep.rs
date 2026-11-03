//! Whether a mechanism's parts hit each other as it moves: each joint that
//! moves goes, alone, from its rest over its whole travel, both ways, a
//! step at a time; at each step the part and what it holds are checked
//! against the rest of the model. The first position each way where they
//! share more volume than they did at rest is where the joint is stopped:
//! a pivot's pin in its hole, two parts glued face to face, overlap at
//! rest and go on doing so, which is no collision.
//!
//! One joint at a time, the others at rest: two joints moved together can
//! hit where neither does alone. A joint rests at 0, or at the nearest of
//! its limits when 0 is outside them.

use std::path::Path;

use c2f_eval::node::{JointKind, Matrix, Node};
use manifold_rust::manifold::Manifold;

use crate::collide::{TOUCHING, place};
use crate::joints::{self, Link};
use crate::pieces::pieces;

/// How far a joint goes at each step, unless told otherwise.
pub const STEP_DEGREES: f64 = 2.0;
pub const STEP_MM: f64 = 0.5;

/// A shared volume grown by less than this share of what it was at rest
/// is the same contact, faceted differently as it turns.
const GROWTH: f64 = 0.02;

#[derive(Clone, Copy, Debug)]
pub struct SweepOptions {
    pub step_degrees: f64,
    pub step_mm: f64,
}

impl Default for SweepOptions {
    fn default() -> Self {
        SweepOptions { step_degrees: STEP_DEGREES, step_mm: STEP_MM }
    }
}

/// Where a joint stops, one way.
#[derive(Clone, Debug, PartialEq)]
pub struct Stop {
    /// The joint's position: degrees or millimeters.
    pub position: f64,
    /// The part that hits, the joint's or one it holds, and the one it
    /// hits, by name (`base` for what is outside any part).
    pub moving: String,
    pub hit: String,
    /// The volume they share there, in cubic millimeters, more than at rest.
    pub volume: f64,
}

/// One joint's travel.
#[derive(Clone, Debug, PartialEq)]
pub struct JointSweep {
    /// Its part's name.
    pub name: String,
    pub kind: JointKind,
    /// Lowest and highest position, and where it starts from: 0, or the
    /// nearest end when 0 is outside them.
    pub range: [f64; 2],
    pub rest: f64,
    pub step: f64,
    /// The first stop going down from rest, and going up.
    pub down: Option<Stop>,
    pub up: Option<Stop>,
}

impl JointSweep {
    /// Whether the joint goes all the way, both ways.
    pub fn free(&self) -> bool {
        self.down.is_none() && self.up.is_none()
    }

    /// One line on it, in English, for the command line and the agent.
    pub fn describe(&self) -> String {
        let unit = if self.kind == JointKind::Prismatic { " mm" } else { "°" };
        let at = |v: f64| format!("{}{unit}", trim(v));
        let travel = format!("{} to {}", at(self.range[0]), at(self.range[1]));
        if self.free() {
            return format!("{}: free over its travel, {travel}", self.name);
        }
        let stop = |s: &Stop| format!("{} hits {} at {} ({} mm³)", s.moving, s.hit, at(s.position), trim(s.volume));
        let ways: Vec<String> = [("going down", &self.down), ("going up", &self.up)]
            .iter()
            .map(|(way, s)| match s {
                Some(s) => format!("{way}, {}", stop(s)),
                None => format!("{way}, free"),
            })
            .collect();
        format!("{} ({travel}, from {}): {}", self.name, at(self.rest), ways.join("; "))
    }
}

fn trim(v: f64) -> String {
    let s = format!("{:.2}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The travel of every joint of the model that moves, in the order the
/// model writes their parts. Data files are read relative to `base_dir`.
pub fn sweep(nodes: &[Node], base_dir: &Path, options: SweepOptions) -> Vec<JointSweep> {
    let split = pieces(nodes, base_dir, "base");
    let pieces = &split.pieces;
    let links: Vec<Link> =
        pieces.iter().map(|p| Link { parent: p.parent, matrix: p.matrix, joint: p.joint.as_ref() }).collect();
    let rest_pose: Vec<f64> = pieces.iter().map(|p| p.joint.as_ref().map_or(0.0, joints::rest)).collect();
    let rest_worlds = joints::worlds(&links, &rest_pose);
    let placed = |i: usize, world: &Matrix| place(&pieces[i].model, world);
    let at_rest: Vec<Option<Manifold>> = (0..pieces.len()).map(|i| placed(i, &rest_worlds[i])).collect();
    let mut out = Vec::new();
    for (index, piece) in pieces.iter().enumerate() {
        let Some(joint) = piece.joint.as_ref() else { continue };
        // What the joint carries: its part and every part written in it.
        let mut carried = vec![false; pieces.len()];
        carried[index] = true;
        for i in index + 1..pieces.len() {
            carried[i] = pieces[i].parent.is_some_and(|p| carried[p]);
        }
        // The box around it and what it holds, as the exploded view takes it.
        let around = carried.iter().zip(&at_rest).filter_map(|(&c, m)| m.as_ref().filter(|_| c)).map(Manifold::bounding_box).reduce(
            |a, b| {
                let mut u = a;
                u.min.x = a.min.x.min(b.min.x);
                u.min.y = a.min.y.min(b.min.y);
                u.min.z = a.min.z.min(b.min.z);
                u.max.x = a.max.x.max(b.max.x);
                u.max.y = a.max.y.max(b.max.y);
                u.max.z = a.max.z.max(b.max.z);
                u
            },
        );
        let size = around.map_or(0.0, |b| {
            ((b.max.x - b.min.x).powi(2) + (b.max.y - b.min.y).powi(2) + (b.max.z - b.min.z).powi(2)).sqrt()
        });
        let Some(range) = joints::range(joint, size) else { continue };
        let step = if joint.kind == JointKind::Prismatic { options.step_mm } else { options.step_degrees }.max(1e-6);
        let rest = rest_pose[index];
        // The pairs that may hit, and what they share at rest.
        let pairs: Vec<(usize, usize, f64)> = (0..pieces.len())
            .filter(|&m| carried[m])
            .flat_map(|m| (0..pieces.len()).filter(|&s| !carried[s]).map(move |s| (m, s)))
            .filter_map(|(m, s)| {
                let (a, b) = (at_rest[m].as_ref()?, at_rest[s].as_ref()?);
                Some((m, s, shared(a, b)))
            })
            .collect();
        let stop_at = |position: f64| -> Option<Stop> {
            let mut pose = rest_pose.clone();
            pose[index] = position;
            let worlds = joints::worlds(&links, &pose);
            let mut moved: Vec<Option<Option<Manifold>>> = vec![None; pieces.len()];
            let mut worst: Option<Stop> = None;
            for &(m, s, before) in &pairs {
                let a = moved[m].get_or_insert_with(|| placed(m, &worlds[m]));
                let (Some(a), Some(b)) = (a.as_ref(), at_rest[s].as_ref()) else { continue };
                let now = shared(a, b);
                let grown = now - before;
                if grown > TOUCHING.max(GROWTH * before) && worst.as_ref().is_none_or(|w| grown > w.volume) {
                    worst = Some(Stop {
                        position,
                        moving: pieces[m].name.clone(),
                        hit: pieces[s].name.clone(),
                        volume: grown,
                    });
                }
            }
            worst
        };
        // Each way from rest, a step at a time, the end itself last.
        let walk = |end: f64| -> Option<Stop> {
            let distance = (end - rest).abs();
            let steps = (distance / step).ceil() as usize;
            (1..=steps).map(|k| rest + (end - rest) * (k as f64 * step).min(distance) / distance.max(1e-12)).find_map(stop_at)
        };
        out.push(JointSweep {
            name: piece.name.clone(),
            kind: joint.kind,
            range,
            rest,
            step,
            down: walk(range[0]),
            up: walk(range[1]),
        });
    }
    out
}

/// The volume two solids share.
fn shared(a: &Manifold, b: &Manifold) -> f64 {
    let (ba, bb) = (a.bounding_box(), b.bounding_box());
    let apart = ba.max.x <= bb.min.x
        || bb.max.x <= ba.min.x
        || ba.max.y <= bb.min.y
        || bb.max.y <= ba.min.y
        || ba.max.z <= bb.min.z
        || bb.max.z <= ba.min.z;
    if apart { 0.0 } else { a.intersection(b).volume() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sweep_src(src: &str) -> Vec<JointSweep> {
        let program = c2f_eval::Program::load(src, Path::new("t.scad"), &[]).expect("parses");
        sweep(&c2f_eval::evaluate(&program).nodes, Path::new(""), SweepOptions::default())
    }

    #[test]
    fn an_arm_that_swings_into_a_post_stops_there() {
        // An arm 20 long along X from a pivot at the origin, turning about
        // Z; a post at 45°, 15 out, in its way going up only.
        let found = sweep_src(
            "rotate([0, 0, 45]) translate([14, -1, 0]) cube([2, 2, 5]);
            part(\"arm\", joint = \"revolute\", limits = [-90, 90]) translate([0, -1, 0]) cube([20, 2, 2]);",
        );
        assert_eq!(found.len(), 1);
        let arm = &found[0];
        assert!(arm.down.is_none(), "{arm:?}");
        let up = arm.up.as_ref().expect("a stop going up");
        // The arm's side, 1 wide, meets the post's 1 wide side a few
        // degrees before 45.
        assert!((30.0..45.0).contains(&up.position), "{}", arm.describe());
        assert_eq!((up.moving.as_str(), up.hit.as_str()), ("arm", "base"));
        assert!(arm.describe().contains("going up, arm hits base at"), "{}", arm.describe());
    }

    #[test]
    fn a_pin_in_its_hole_is_no_collision() {
        // The arm's hub overlaps the base at rest and turns in it: free.
        let found = sweep_src(
            "cylinder(r = 5, h = 2, $fn = 24);
            part(\"arm\", joint = \"continuous\") { cylinder(r = 3, h = 4, $fn = 24); translate([0, -1, 3]) cube([20, 2, 1]); }",
        );
        assert!(found[0].free(), "{}", found[0].describe());
        assert_eq!(found[0].range, [-180.0, 180.0]);
    }

    #[test]
    fn a_slide_stops_at_a_wall_and_carries_what_it_holds() {
        // A carriage sliding along X carries a finger that hits a wall at
        // x = 10: the finger's end is at 6, so 4 mm in.
        let found = sweep_src(
            "translate([10, -5, 0]) cube([1, 10, 10]);
            part(\"carriage\", joint = \"prismatic\", axis = [1, 0, 0], limits = [-3, 8]) {
                cube([2, 2, 2]);
                translate([2, 0, 0]) part(\"finger\") cube([4, 1, 1]);
            }",
        );
        let carriage = &found[0];
        let up = carriage.up.as_ref().expect("a stop");
        assert_eq!((up.moving.as_str(), up.hit.as_str()), ("finger", "base"));
        assert!((4.0..=4.5).contains(&up.position), "{}", carriage.describe());
        assert!(carriage.down.is_none());
        assert!(carriage.describe().contains("mm"), "{}", carriage.describe());
    }

    #[test]
    fn a_joint_whose_limits_leave_out_zero_starts_from_its_nearest_one() {
        // At 0 the arm lies in one post, out of its travel (10° to 90°); a
        // second post at 60° stops it. Measured from 0, the first overlap
        // would hide the second.
        let found = sweep_src(
            "translate([14, -1, 0]) cube([2, 2, 5]);
            rotate([0, 0, 60]) translate([14, -1, 0]) cube([2, 2, 5]);
            part(\"arm\", joint = \"revolute\", limits = [10, 90]) translate([0, -1, 0]) cube([20, 2, 2]);",
        );
        let arm = &found[0];
        assert_eq!(arm.rest, 10.0);
        assert!(arm.down.is_none(), "{}", arm.describe());
        let up = arm.up.as_ref().expect("a stop going up");
        assert!((45.0..60.0).contains(&up.position), "{}", arm.describe());
    }

    #[test]
    fn fixed_parts_and_models_without_joints_have_no_travel() {
        assert!(sweep_src("cube(1); part(\"lid\") cube(1);").is_empty());
        assert!(sweep_src("part(\"lid\", joint = \"fixed\") cube(1);").is_empty());
    }
}
