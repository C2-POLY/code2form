//! The geometry of the move, turn and scale handles (C2F-39), apart from
//! the app: where each handle shows on screen, which one the pointer is
//! over, and what a drag of it means in the model's space.
//!
//! The handles sit at the centre of the selection's box, a constant size on
//! screen: each length below is in points and becomes world units through
//! [`Gizmo::unit`], the size of one point at that centre. Along each axis of
//! the gizmo's frame (the world's, or the object's own once turned) are an
//! arrow to move, a ring to turn around it, a square on the far side to
//! scale, and between each pair of axes a small square to move in their
//! plane; a dot at the centre scales evenly.
//!
//! A drag becomes a [`Change`]: a move by a vector, a turn by an angle
//! around an axis, or scale factors along the object's own axes. How a
//! change is written in the code is up to the app; here is only what it
//! does to the points of the model, for the preview.
//!
//! Turns are OpenSCAD's: `rotate([x, y, z])` turns around X, then Y, then
//! Z, all fixed axes, so its matrix is `Rz · Ry · Rx` ([`rotation`],
//! [`euler`] going back).

use eframe::egui::Pos2;
use glam::{DMat3, DMat4, DVec3, Mat4, Vec2, Vec3};

/// One of the handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    /// Moves along axis `0..3` of the frame.
    Move(usize),
    /// Moves in the plane across axis `0..3` (the other two).
    Plane(usize),
    /// Turns around axis `0..3` of the frame.
    Turn(usize),
    /// Scales along the object's axis `0..3`.
    Scale(usize),
    /// Scales evenly.
    ScaleAll,
}

impl Handle {
    /// Its name, for a hint.
    pub fn describe(self) -> String {
        const NAMES: [&str; 3] = ["X", "Y", "Z"];
        match self {
            Handle::Move(k) => format!("Move along {}", NAMES[k]),
            Handle::Plane(k) => format!("Move in {}", ["YZ", "XZ", "XY"][k]),
            Handle::Turn(k) => format!("Turn around {}", NAMES[k]),
            Handle::Scale(k) => format!("Scale along {}", NAMES[k]),
            Handle::ScaleAll => "Scale evenly".to_string(),
        }
    }

    /// The axis it follows or turns around, if any.
    pub fn axis(self) -> Option<usize> {
        match self {
            Handle::Move(k) | Handle::Plane(k) | Handle::Turn(k) | Handle::Scale(k) => Some(k),
            Handle::ScaleAll => None,
        }
    }
}

// The layout, in points from the centre.
const ARROW_FROM: f32 = 16.0;
const ARROW_BASE: f32 = 92.0;
const ARROW_TIP: f32 = 110.0;
/// Where the scale squares are, on the negative side of each axis.
pub const SCALE_AT: f32 = 58.0;
const PLANE_FROM: f32 = 22.0;
const PLANE_TO: f32 = 40.0;
const RING: f32 = 76.0;
const RING_SEGMENTS: usize = 64;
/// How near, in points, the pointer must be to a handle to grab it.
pub const REACH: f32 = 7.0;

/// What a handle looks like on screen.
#[derive(Clone, Debug, PartialEq)]
pub enum Mark {
    /// A shaft and the triangle at its tip.
    Arrow { from: Pos2, base: Pos2, tip: Pos2 },
    /// A closed line.
    Ring(Vec<Pos2>),
    /// A square in a plane, as four corners.
    Quad([Pos2; 4]),
    /// A small square facing the eye, joined to `anchor` by a thin line.
    Knob { at: Pos2, anchor: Pos2 },
    /// A dot facing the eye.
    Dot(Pos2),
}

impl Mark {
    /// The distance from `p` to the mark on screen; 0 inside a square.
    pub fn distance(&self, p: Pos2) -> f32 {
        match self {
            Mark::Arrow { from, tip, .. } => segment_distance(p, *from, *tip),
            Mark::Ring(points) => {
                let n = points.len();
                (0..n).map(|i| segment_distance(p, points[i], points[(i + 1) % n])).fold(f32::INFINITY, f32::min)
            }
            Mark::Quad(corners) => {
                if inside(p, corners) {
                    0.0
                } else {
                    (0..4).map(|i| segment_distance(p, corners[i], corners[(i + 1) % 4])).fold(f32::INFINITY, f32::min)
                }
            }
            Mark::Knob { at, .. } => (p.distance(*at) - 5.0).max(0.0),
            Mark::Dot(at) => (p.distance(*at) - 6.0).max(0.0),
        }
    }
}

/// The handles on an object, in the model's space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gizmo {
    /// The centre of the selection's box.
    pub origin: Vec3,
    /// The frame's axes: the world's, or the object's own (unit vectors).
    pub axes: [Vec3; 3],
    /// The object's own axes, which scaling follows whatever the frame.
    pub own_axes: [Vec3; 3],
    /// How long one point of the screen is at `origin`, in model units.
    pub unit: f32,
}

impl Gizmo {
    /// The handles at `origin`, `unit` being the size of a point there.
    pub fn new(origin: Vec3, axes: [Vec3; 3], own_axes: [Vec3; 3], unit: f32) -> Self {
        Gizmo { origin, axes, own_axes, unit }
    }

    /// Whether scaling along each axis has a handle: only when the frame's
    /// axes are the object's own, as a scale along a world axis of a turned
    /// object would shear it, which `scale` cannot write.
    fn scales_per_axis(&self) -> bool {
        self.axes.iter().zip(&self.own_axes).all(|(a, b)| a.dot(*b) > 0.9999)
    }

    /// Every handle that shows, with its mark, as `project` puts points of
    /// the model on screen. An arrow pointing at the eye, or a plane seen
    /// edge-on, is left out: it could not be dragged sensibly.
    pub fn marks(&self, project: impl Fn(Vec3) -> Option<Pos2>) -> Vec<(Handle, Mark)> {
        let Some(center) = project(self.origin) else { return Vec::new() };
        let at = |axis: Vec3, points: f32| project(self.origin + axis * (points * self.unit));
        let mut out = Vec::new();
        for k in 0..3 {
            let (b, c) = (self.axes[(k + 1) % 3], self.axes[(k + 2) % 3]);
            let ring: Option<Vec<Pos2>> = (0..RING_SEGMENTS)
                .map(|i| {
                    let t = i as f32 / RING_SEGMENTS as f32 * std::f32::consts::TAU;
                    project(self.origin + (b * t.cos() + c * t.sin()) * (RING * self.unit))
                })
                .collect();
            if let Some(ring) = ring {
                out.push((Handle::Turn(k), Mark::Ring(ring)));
            }
        }
        for k in 0..3 {
            let (b, c) = (self.axes[(k + 1) % 3], self.axes[(k + 2) % 3]);
            let corner = |u: f32, v: f32| project(self.origin + (b * u + c * v) * self.unit);
            if let (Some(p0), Some(p1), Some(p2), Some(p3)) = (
                corner(PLANE_FROM, PLANE_FROM),
                corner(PLANE_TO, PLANE_FROM),
                corner(PLANE_TO, PLANE_TO),
                corner(PLANE_FROM, PLANE_TO),
            ) {
                let quad = [p0, p1, p2, p3];
                let side = PLANE_TO - PLANE_FROM;
                if area(&quad) > 0.2 * side * side {
                    out.push((Handle::Plane(k), Mark::Quad(quad)));
                }
            }
        }
        for k in 0..3 {
            let axis = self.axes[k];
            if let (Some(from), Some(base), Some(tip)) =
                (at(axis, ARROW_FROM), at(axis, ARROW_BASE), at(axis, ARROW_TIP))
            {
                if tip.distance(center) > 0.25 * ARROW_TIP {
                    out.push((Handle::Move(k), Mark::Arrow { from, base, tip }));
                }
            }
            if self.scales_per_axis() {
                if let (Some(at), Some(anchor)) = (at(-axis, SCALE_AT), at(-axis, ARROW_FROM)) {
                    if at.distance(center) > 0.25 * SCALE_AT {
                        out.push((Handle::Scale(k), Mark::Knob { at, anchor }));
                    }
                }
            }
        }
        out.push((Handle::ScaleAll, Mark::Dot(center)));
        out
    }

    /// The handle under `pointer`: the even scale at the centre first, then
    /// arrows and scale squares, plane squares, and rings last, the nearest
    /// within reach in each.
    pub fn pick(&self, project: impl Fn(Vec3) -> Option<Pos2>, pointer: Pos2) -> Option<Handle> {
        let marks = self.marks(project);
        let rank = |h: &Handle| match h {
            Handle::ScaleAll => 0,
            Handle::Move(_) | Handle::Scale(_) => 1,
            Handle::Plane(_) => 2,
            Handle::Turn(_) => 3,
        };
        marks
            .iter()
            .map(|(h, m)| (*h, m.distance(pointer)))
            .filter(|&(_, d)| d <= REACH)
            .min_by(|a, b| rank(&a.0).cmp(&rank(&b.0)).then(a.1.total_cmp(&b.1)))
            .map(|(h, _)| h)
    }
}

/// How long one point of the screen is at `p`, in model units: `project`
/// puts points on screen, `right` is the eye's right direction.
pub fn unit_at(project: impl Fn(Vec3) -> Option<Pos2>, p: Vec3, right: Vec3) -> Option<f32> {
    let (a, b) = (project(p)?, project(p + right)?);
    let points = a.distance(b);
    (points > 1e-6).then(|| 1.0 / points)
}

// ---- dragging ----

/// Where along the line through `origin` in direction `axis` (a unit
/// vector) the ray from `eye` in direction `dir` passes nearest; `None`
/// when they are nearly parallel.
pub fn axis_param(eye: Vec3, dir: Vec3, origin: Vec3, axis: Vec3) -> Option<f32> {
    let dir = dir.normalize_or_zero();
    let w = origin - eye;
    let b = axis.dot(dir);
    let denom = 1.0 - b * b;
    if denom < 1e-5 {
        return None;
    }
    let (d, e) = (axis.dot(w), dir.dot(w));
    Some((b * e - d) / denom)
}

/// Where the ray from `eye` in direction `dir` meets the plane through
/// `origin` across `normal`; `None` when it runs along it or away.
pub fn plane_point(eye: Vec3, dir: Vec3, origin: Vec3, normal: Vec3) -> Option<Vec3> {
    let along = dir.dot(normal);
    if along.abs() < 1e-4 {
        return None;
    }
    let t = (origin - eye).dot(normal) / along;
    (t > 0.0).then(|| eye + dir * t)
}

/// The angle from `from` to `to` around `center` on screen, in degrees,
/// clockwise as seen (the screen's y goes down), between -180 and 180.
pub fn screen_angle(center: Pos2, from: Pos2, to: Pos2) -> f32 {
    let (a, b) = (Vec2::new(from.x - center.x, from.y - center.y), Vec2::new(to.x - center.x, to.y - center.y));
    a.perp_dot(b).atan2(a.dot(b)).to_degrees()
}

/// A turn around `axis` as a drag going `clockwise` degrees round the
/// gizmo's centre on screen shows it: the right-hand turn around an axis
/// pointing at the eye goes anticlockwise.
pub fn turn_from_screen(clockwise: f32, axis: Vec3, toward_eye: Vec3) -> f32 {
    if axis.dot(toward_eye) > 0.0 { -clockwise } else { clockwise }
}

/// The angle from `a` to `b` around `axis` (a unit vector), in degrees,
/// right-handed, between -180 and 180.
pub fn angle_around(axis: Vec3, a: Vec3, b: Vec3) -> f32 {
    axis.dot(a.cross(b)).atan2(a.dot(b)).to_degrees()
}

/// `value` to the nearest multiple of `step`; as is for no step.
pub fn snap(value: f64, step: f64) -> f64 {
    if step > 0.0 { (value / step).round() * step } else { value }
}

/// A scale factor as a drag from `start` to `now` points away from the
/// centre makes it, kept above a twentieth.
pub fn scale_factor(start: f32, now: f32) -> f64 {
    if start.abs() < 1e-6 {
        return 1.0;
    }
    f64::from(now / start).max(0.05)
}

// ---- turns ----

/// The matrix of OpenSCAD's `rotate([x, y, z])` (degrees): around X, then
/// Y, then Z.
pub fn rotation(degrees: [f64; 3]) -> DMat3 {
    let [x, y, z] = degrees.map(f64::to_radians);
    DMat3::from_rotation_z(z) * DMat3::from_rotation_y(y) * DMat3::from_rotation_x(x)
}

/// The angles `rotate([x, y, z])` takes to turn as `m` does, x and z
/// between -180 and 180, y between -90 and 90; at y = ±90, where X and Z
/// turn alike, all of it goes to z. Values within a billionth of a whole
/// number of degrees are rounded to it, for clean code.
pub fn euler(m: DMat3) -> [f64; 3] {
    // m = Rz · Ry · Rx; row r, column c is m.col(c)[r].
    let at = |r: usize, c: usize| m.col(c)[r];
    let sy = (-at(2, 0)).clamp(-1.0, 1.0);
    let y = sy.asin();
    let (x, z) = if sy.abs() > 1.0 - 1e-12 {
        (0.0, (-at(0, 1)).atan2(at(1, 1)))
    } else {
        (at(2, 1).atan2(at(2, 2)), at(1, 0).atan2(at(0, 0)))
    };
    [x, y, z].map(|a| {
        let d = a.to_degrees();
        let whole = d.round();
        let d = if (d - whole).abs() < 1e-9 { whole } else { d };
        if d == 0.0 { 0.0 } else { d }
    })
}

/// A turn of `degrees` around `axis` (a unit vector), right-handed.
pub fn axis_rotation(axis: DVec3, degrees: f64) -> DMat3 {
    DMat3::from_axis_angle(axis.normalize_or_zero(), degrees.to_radians())
}

// ---- changes ----

/// What a drag does to the object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Change {
    /// Moves it by a vector of the model's space.
    Move(DVec3),
    /// Turns it around an axis of the model's space (a unit vector) by an
    /// angle in degrees, about the gizmo's centre.
    Turn { axis: DVec3, degrees: f64 },
    /// Scales it along its own axes, about the gizmo's centre.
    Scale([f64; 3]),
}

impl Change {
    /// Whether it changes nothing.
    pub fn is_identity(&self) -> bool {
        match *self {
            Change::Move(v) => v.length() < 1e-9,
            Change::Turn { degrees, .. } => degrees.abs() < 1e-9,
            Change::Scale(f) => f.iter().all(|x| (x - 1.0).abs() < 1e-9),
        }
    }

    /// What it does to the points of the model, about `pivot`, for an
    /// object turned as `own` says (its own axes, which scaling follows).
    pub fn matrix(&self, pivot: DVec3, own: DMat3) -> DMat4 {
        let about = |m: DMat3| DMat4::from_translation(pivot) * DMat4::from_mat3(m) * DMat4::from_translation(-pivot);
        match *self {
            Change::Move(v) => DMat4::from_translation(v),
            Change::Turn { axis, degrees } => about(axis_rotation(axis, degrees)),
            Change::Scale(f) => about(own * DMat3::from_diagonal(DVec3::from(f)) * own.transpose()),
        }
    }

    /// [`Change::matrix`] in single precision, for the view.
    pub fn preview(&self, pivot: DVec3, own: DMat3) -> Mat4 {
        self.matrix(pivot, own).as_mat4()
    }
}

// ---- screen geometry ----

fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let (ab, ap) = (b - a, p - a);
    let len2 = ab.length_sq();
    let t = if len2 > 0.0 { (ap.dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

fn cross(o: Pos2, a: Pos2, b: Pos2) -> f32 {
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
}

/// Whether `p` is inside the convex quad, whichever way it winds.
fn inside(p: Pos2, q: &[Pos2; 4]) -> bool {
    let signs: Vec<f32> = (0..4).map(|i| cross(q[i], q[(i + 1) % 4], p)).collect();
    signs.iter().all(|&s| s >= 0.0) || signs.iter().all(|&s| s <= 0.0)
}

fn area(q: &[Pos2; 4]) -> f32 {
    (cross(q[0], q[1], q[2]) + cross(q[0], q[2], q[3])).abs() / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A camera looking down at the XY plane from +Z, 100 points per unit,
    /// the origin at (400, 300) on screen and +Y going up it.
    fn top(p: Vec3) -> Option<Pos2> {
        Some(Pos2::new(400.0 + p.x * 100.0, 300.0 - p.y * 100.0))
    }

    const WORLD: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn a_point_on_screen_is_a_hundredth_of_a_unit_from_the_top() {
        assert_eq!(unit_at(top, Vec3::ZERO, Vec3::X), Some(0.01));
    }

    #[test]
    fn the_pointer_picks_the_handle_it_is_on() {
        let gizmo = Gizmo::new(Vec3::ZERO, WORLD, WORLD, 0.01);
        let pick = |x: f32, y: f32| gizmo.pick(top, Pos2::new(x, y));
        assert_eq!(pick(400.0, 300.0), Some(Handle::ScaleAll));
        // Along +X, the arrow; along +Y (up the screen), its arrow.
        assert_eq!(pick(400.0 + 60.0, 302.0), Some(Handle::Move(0)));
        assert_eq!(pick(399.0, 300.0 - 100.0), Some(Handle::Move(1)));
        // Z points at the eye: no arrow, but its ring turns around it.
        assert!(!gizmo.marks(top).iter().any(|(h, _)| *h == Handle::Move(2)));
        let on_ring =
            Pos2::new(400.0 + RING * std::f32::consts::FRAC_1_SQRT_2, 300.0 - RING * std::f32::consts::FRAC_1_SQRT_2);
        assert_eq!(pick(on_ring.x, on_ring.y), Some(Handle::Turn(2)));
        // Between X and Y, the plane square across Z.
        assert_eq!(pick(431.0, 269.0), Some(Handle::Plane(2)));
        // On the far side of X, its scale square.
        assert_eq!(pick(400.0 - SCALE_AT, 300.0), Some(Handle::Scale(0)));
        assert_eq!(pick(700.0, 100.0), None);
    }

    #[test]
    fn a_turned_frame_has_no_scale_along_world_axes() {
        let turned = rotation([0.0, 0.0, 30.0]);
        let own = [0, 1, 2].map(|k| turned.col(k).as_vec3());
        let world = Gizmo::new(Vec3::ZERO, WORLD, own, 0.01);
        assert!(!world.marks(top).iter().any(|(h, _)| matches!(h, Handle::Scale(_))));
        let local = Gizmo::new(Vec3::ZERO, own, own, 0.01);
        assert!(local.marks(top).iter().any(|(h, _)| *h == Handle::Scale(0)));
        // The local X arrow points 30° up from the screen's right.
        let tip = local.marks(top).into_iter().find_map(|(h, m)| match (h, m) {
            (Handle::Move(0), Mark::Arrow { tip, .. }) => Some(tip),
            _ => None,
        });
        let tip = tip.unwrap();
        let angle = (300.0 - tip.y).atan2(tip.x - 400.0).to_degrees();
        assert!((angle - 30.0).abs() < 0.01, "{angle}");
    }

    #[test]
    fn a_ray_across_an_axis_gives_the_point_along_it() {
        // Looking down from (3, 2, 10): straight over x = 3 of the X axis.
        let t = axis_param(Vec3::new(3.0, 2.0, 10.0), -Vec3::Z, Vec3::ZERO, Vec3::X).unwrap();
        assert!((t - 3.0).abs() < 1e-5);
        // From an axis through (1, 0, 0) the same ray is 2 along.
        let t = axis_param(Vec3::new(3.0, 2.0, 10.0), -Vec3::Z * 4.0, Vec3::X, Vec3::X).unwrap();
        assert!((t - 2.0).abs() < 1e-5);
        // Looking along the axis, nothing.
        assert_eq!(axis_param(Vec3::new(0.0, 0.0, 10.0), -Vec3::Z, Vec3::ZERO, Vec3::Z), None);
        let p = plane_point(Vec3::new(1.0, 2.0, 5.0), Vec3::new(0.0, 0.0, -1.0), Vec3::ZERO, Vec3::Z).unwrap();
        assert_eq!(p, Vec3::new(1.0, 2.0, 0.0));
        assert_eq!(plane_point(Vec3::new(1.0, 2.0, 5.0), Vec3::Z, Vec3::ZERO, Vec3::Z), None);
    }

    #[test]
    fn snapping_rounds_to_the_step() {
        assert_eq!(snap(12.4, 1.0), 12.0);
        assert_eq!(snap(-7.6, 1.0), -8.0);
        assert_eq!(snap(52.0, 15.0), 45.0);
        assert_eq!(snap(53.0, 15.0), 60.0);
        assert_eq!(snap(1.234, 0.0), 1.234);
        assert_eq!(scale_factor(50.0, 75.0), 1.5);
        assert_eq!(scale_factor(50.0, -10.0), 0.05);
    }

    #[test]
    fn a_drag_round_a_ring_turns_by_its_angle() {
        let c = Pos2::new(400.0, 300.0);
        // From the right to the top of the screen: a quarter, anticlockwise.
        let clockwise = screen_angle(c, Pos2::new(500.0, 300.0), Pos2::new(400.0, 200.0));
        assert!((clockwise + 90.0).abs() < 1e-4);
        // Seen from above, that is +90° around Z (X turns to Y)...
        assert_eq!(turn_from_screen(clockwise, Vec3::Z, Vec3::Z), 90.0);
        // ...and from below, -90°.
        assert_eq!(turn_from_screen(clockwise, Vec3::Z, -Vec3::Z), -90.0);
        let m = rotation([0.0, 0.0, 90.0]);
        assert!((m * DVec3::X - DVec3::Y).length() < 1e-12);
        // In the ring's plane, from X to Y is +90° around Z.
        assert!((angle_around(Vec3::Z, Vec3::X, Vec3::Y) - 90.0).abs() < 1e-4);
        assert!((angle_around(-Vec3::Z, Vec3::X, Vec3::Y) + 90.0).abs() < 1e-4);
    }

    #[test]
    fn euler_angles_go_back_to_the_rotation() {
        for angles in [[0.0, 0.0, 45.0], [90.0, 0.0, 0.0], [10.0, 20.0, 30.0], [-120.0, 45.0, 170.0], [30.0, 90.0, 0.0]]
        {
            let m = rotation(angles);
            let back = rotation(euler(m));
            for k in 0..3 {
                assert!((m.col(k) - back.col(k)).length() < 1e-9, "{angles:?} → {:?}", euler(m));
            }
        }
        assert_eq!(euler(rotation([0.0, 0.0, 45.0])), [0.0, 0.0, 45.0]);
        assert_eq!(euler(rotation([10.0, 20.0, 30.0])), [10.0, 20.0, 30.0]);
        assert_eq!(euler(DMat3::IDENTITY), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn local_and_world_turns_compose_on_either_side() {
        // An object turned 90° around Z; its own X is the world's Y.
        let own = rotation([0.0, 0.0, 90.0]);
        // Turning it 90° around its own X is turning it around the world's Y.
        let local = own * axis_rotation(DVec3::X, 90.0);
        let world = axis_rotation(own * DVec3::X, 90.0) * own;
        for k in 0..3 {
            assert!((local.col(k) - world.col(k)).length() < 1e-12);
        }
        let e = euler(local);
        assert!(close(e[0], 90.0) && close(e[1], 0.0) && close(e[2], 90.0), "{e:?}");
    }

    #[test]
    fn changes_move_the_points_about_the_pivot() {
        let pivot = DVec3::new(5.0, 5.0, 5.0);
        let m = Change::Move(DVec3::new(1.0, 2.0, 3.0)).matrix(pivot, DMat3::IDENTITY);
        assert_eq!(m.transform_point3(DVec3::ZERO), DVec3::new(1.0, 2.0, 3.0));
        let m = Change::Turn { axis: DVec3::Z, degrees: 90.0 }.matrix(pivot, DMat3::IDENTITY);
        // The pivot stays; the corner at the origin swings round it.
        assert!((m.transform_point3(pivot) - pivot).length() < 1e-12);
        assert!((m.transform_point3(DVec3::ZERO) - DVec3::new(10.0, 0.0, 0.0)).length() < 1e-12);
        // Twice as long along the object's own X, turned to the world's Y.
        let own = rotation([0.0, 0.0, 90.0]);
        let m = Change::Scale([2.0, 1.0, 1.0]).matrix(pivot, own);
        assert!((m.transform_point3(pivot + DVec3::Y) - (pivot + DVec3::Y * 2.0)).length() < 1e-12);
        assert!((m.transform_point3(pivot + DVec3::X) - (pivot + DVec3::X)).length() < 1e-12);
        assert!(Change::Scale([1.0; 3]).is_identity());
    }
}
