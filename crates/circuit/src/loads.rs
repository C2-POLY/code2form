//! Static loads on a turning shaft: what holds it, what turns it. A watch
//! winder's cradle at the front end of a shaft, a turntable, a pan axis:
//! masses hang on a shaft that supports carry (the motor's own bearings,
//! ball bearings, printed holes) and a motor turns. From the masses and
//! where they sit, this module computes the torque the motor must give,
//! each support's reaction, how far the shaft bends at the loads, and the
//! margins against the figures of the motor's and the bearings' sheets,
//! so that an agent can decide on a front bearing, a reduction or another
//! motor with numbers.
//!
//! Engineering estimates, not a simulation: the loads are static (no
//! acceleration, no vibration), the shaft is a straight beam of one
//! diameter, the supports are points, and friction is a coefficient
//! times the load on the radius it rubs at. The margins cover what that
//! leaves out; the target margin is 2 unless the scenario gives another.
//!
//! Positions are in millimeters and masses in grams, as the designs and
//! the part sheets give them; forces come out in newtons, moments in
//! newton millimeters and torques in millinewton meters, which is the
//! same unit (1 N·mm = 1 mN·m). Gravity is along -z.
//!
//! [`analyse`] is pure; [`with_sheets`] first fills in the figures the
//! scenario leaves to its parts' `// @spec` lines ([`crate::quantity`]).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::quantity::spec_figure;

/// Standard gravity, m/s².
pub const G: f64 = 9.806_65;

/// A deep-groove ball bearing's friction coefficient, referred to its
/// bore: catalogues give 0.0010 to 0.0015 for a lightly loaded, greased
/// bearing; the higher figure allows for seals.
pub const MU_BEARING: f64 = 0.0015;

/// A steel shaft turning in a printed plastic hole (PLA, PETG), dry:
/// published pin-on-disc figures run from 0.2 to 0.4.
pub const MU_BUSHING: f64 = 0.3;

/// The output shaft of a small gear motor (N20, TT, 28BYJ-48) runs in a
/// sintered bronze or plastic sleeve, not a ball bearing: taken as a
/// bushing, which errs on the safe side.
pub const MU_MOTOR: f64 = 0.3;

/// The efficiency of one stage of spur gears or a belt, printed or not.
pub const STAGE_EFFICIENCY: f64 = 0.9;

/// The greatest reduction one stage gives; more takes another stage.
pub const STAGE_RATIO: f64 = 6.0;

/// The length over which a support alone holds the shaft against a
/// moment: the spacing of a small gear motor's two output bearings, or
/// the length of a printed hole. A support's `span_mm` sets its own.
pub const SUPPORT_SPAN_MM: f64 = 8.0;

/// A deflection one sees as wobble at the loads.
pub const VISIBLE_DEFLECTION_MM: f64 = 0.2;

/// The reductions a gearbox, a pair of gears or a belt gives, from which
/// the suggested ratio is the first that is enough.
pub const RATIOS: &[f64] = &[
    1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0, 15.0, 20.0, 25.0, 30.0, 40.0, 50.0, 60.0, 80.0, 100.0, 120.0, 150.0,
    200.0, 250.0, 300.0, 400.0, 500.0, 1000.0,
];

/// What turns, what weighs on it and what holds it, as the `loads` tool
/// and `c2f loads` read it from JSON.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub axis: Axis,
    #[serde(default)]
    pub loads: Vec<Load>,
    pub supports: Vec<Support>,
    #[serde(default)]
    pub motor: Option<Motor>,
    #[serde(default)]
    pub shaft: Shaft,
    /// Turns of the motor for one of the shaft: 1 when it drives it
    /// directly.
    #[serde(default = "one")]
    pub ratio: f64,
    /// The reduction's stages, each [`STAGE_EFFICIENCY`]; by default as
    /// many as it takes at [`STAGE_RATIO`] each.
    #[serde(default)]
    pub stages: Option<u32>,
    /// The drive's efficiency, instead of the stages'.
    #[serde(default)]
    pub efficiency: Option<f64>,
    #[serde(default = "two")]
    pub target_margin: f64,
}

fn one() -> f64 {
    1.0
}

fn two() -> f64 {
    2.0
}

/// The line the shaft turns about: through `at`, along `direction`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Axis {
    pub at: [f64; 3],
    pub direction: [f64; 3],
}

/// A mass that turns with the shaft, at its center of mass.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Load {
    pub name: String,
    pub mass_g: f64,
    pub at: [f64; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SupportKind {
    /// The motor's own output bearings.
    Motor,
    /// A ball bearing.
    Bearing,
    /// A printed hole the shaft turns in.
    Bushing,
}

/// Where the shaft is held, along the axis from its point.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Support {
    pub name: String,
    pub at_mm: f64,
    pub kind: SupportKind,
    /// A part whose sheet gives its figures: `static load`, and for the
    /// motor those of [`Motor`].
    #[serde(default)]
    pub part: Option<String>,
    /// A motor joined to the shaft by a flexible coupling, which passes
    /// the torque and no radial load: it holds nothing up.
    #[serde(default)]
    pub coupling: bool,
    /// A bearing's static load rating, C0.
    #[serde(default, rename = "static_load_N")]
    pub static_load_n: Option<f64>,
    /// The length over which it holds the shaft alone ([`SUPPORT_SPAN_MM`]).
    #[serde(default)]
    pub span_mm: Option<f64>,
}

/// The motor's figures, given here or read from its part's sheet; one
/// given here wins over the sheet's.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Motor {
    #[serde(default)]
    pub part: Option<String>,
    /// The torque it gives running, at its output shaft.
    #[serde(default, rename = "torque_mNm")]
    pub torque_mnm: Option<f64>,
    #[serde(default, rename = "stall_torque_mNm")]
    pub stall_torque_mnm: Option<f64>,
    /// A stepper's.
    #[serde(default, rename = "holding_torque_mNm")]
    pub holding_torque_mnm: Option<f64>,
    #[serde(default)]
    pub speed_rpm: Option<f64>,
    #[serde(default, rename = "max_radial_load_N")]
    pub max_radial_load_n: Option<f64>,
    #[serde(default, rename = "max_axial_load_N")]
    pub max_axial_load_n: Option<f64>,
}

/// The shaft: its diameter and what it is made of (steel when neither
/// `material` nor `E_GPa` says).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shaft {
    pub d_mm: f64,
    #[serde(default)]
    pub material: Option<String>,
    #[serde(default, rename = "E_GPa")]
    pub e_gpa: Option<f64>,
}

impl Default for Shaft {
    /// A 5 mm steel shaft, the size of a 28BYJ-48's or a 625 bearing's.
    fn default() -> Shaft {
        Shaft { d_mm: 5.0, material: None, e_gpa: None }
    }
}

/// Young's modulus of a shaft material, GPa: handbook figures, the
/// plastics' as printed (along the layers, which a printed shaft is not
/// always).
pub fn young_modulus_gpa(material: &str) -> Option<f64> {
    Some(match material.trim().to_lowercase().as_str() {
        "steel" => 210.0,
        "stainless" | "stainless steel" => 193.0,
        "brass" => 100.0,
        "aluminium" | "aluminum" => 69.0,
        "pla" => 3.5,
        "petg" => 2.1,
        _ => return None,
    })
}

/// What [`analyse`] finds.
#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub torque: Torque,
    pub supports: Vec<Reaction>,
    pub loads: Vec<LoadReport>,
    pub max_deflection_mm: f64,
    pub motor: Option<MotorReport>,
    pub margins: Margins,
    pub target_margin: f64,
    /// The exact reduction that reaches the target torque margin, with the
    /// suggested one's efficiency, when the torque falls short.
    pub ratio_needed: Option<f64>,
    /// The first of [`RATIOS`] that is enough.
    pub suggested_ratio: Option<f64>,
    pub warnings: Vec<String>,
    /// Sentences an agent can quote.
    pub summary: Vec<String>,
}

/// At the shaft.
#[derive(Clone, Debug, Serialize)]
pub struct Torque {
    /// What the loads' weight takes in the worst position of a turn.
    #[serde(rename = "static_mNm")]
    pub static_mnm: f64,
    /// In the position the scenario gives.
    #[serde(rename = "here_mNm")]
    pub here_mnm: f64,
    #[serde(rename = "friction_mNm")]
    pub friction_mnm: f64,
    /// Static and friction.
    #[serde(rename = "needed_mNm")]
    pub needed_mnm: f64,
}

/// A support's share.
#[derive(Clone, Debug, Serialize)]
pub struct Reaction {
    pub name: String,
    pub kind: SupportKind,
    pub at_mm: f64,
    /// Whether it holds the shaft up: false for a motor on a coupling.
    pub carries: bool,
    #[serde(rename = "radial_N")]
    pub radial_n: f64,
    #[serde(rename = "axial_N")]
    pub axial_n: f64,
    /// The moment it holds: the overhang's, when it holds the shaft alone.
    #[serde(rename = "moment_Nmm")]
    pub moment_nmm: f64,
    /// The radial load on its front bearing: the radial reaction, plus
    /// the moment over its span when it holds the shaft alone.
    #[serde(rename = "bearing_load_N")]
    pub bearing_load_n: f64,
    #[serde(rename = "friction_mNm")]
    pub friction_mnm: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct LoadReport {
    pub name: String,
    pub mass_g: f64,
    /// Along the axis from its point.
    pub at_mm: f64,
    /// From the axis.
    pub offset_mm: f64,
    /// How far the shaft bends under it.
    pub deflection_mm: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct MotorReport {
    pub part: Option<String>,
    /// The torque counted on, at the motor's shaft.
    #[serde(rename = "torque_mNm")]
    pub torque_mnm: Option<f64>,
    /// Which figure that is: `torque`, `half the stall torque` or
    /// `holding torque`.
    pub torque_from: Option<String>,
    /// Through the reduction, with its losses.
    #[serde(rename = "at_shaft_mNm")]
    pub at_shaft_mnm: Option<f64>,
    pub ratio: f64,
    pub efficiency: f64,
    pub speed_rpm: Option<f64>,
    pub shaft_speed_rpm: Option<f64>,
    #[serde(rename = "max_radial_load_N")]
    pub max_radial_load_n: Option<f64>,
    #[serde(rename = "max_axial_load_N")]
    pub max_axial_load_n: Option<f64>,
    /// On its front bearing.
    #[serde(rename = "radial_load_N")]
    pub radial_load_n: f64,
    #[serde(rename = "axial_load_N")]
    pub axial_load_n: f64,
}

/// Each figure available over what it must take; none when a figure is
/// missing or nothing is asked of it.
#[derive(Clone, Debug, Serialize)]
pub struct Margins {
    pub torque: Option<f64>,
    pub motor_radial: Option<f64>,
    pub motor_axial: Option<f64>,
    pub bearings: Vec<BearingMargin>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BearingMargin {
    pub name: String,
    #[serde(rename = "static_load_N")]
    pub static_load_n: f64,
    /// P0 = max(Fr, 0.6 Fr + 0.5 Fa), a deep-groove bearing's.
    #[serde(rename = "equivalent_load_N")]
    pub equivalent_load_n: f64,
    pub margin: Option<f64>,
}

/// The file `part` names: as is, then in each of `dirs`.
pub fn find_part(part: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    std::iter::once(PathBuf::from(part)).chain(dirs.iter().map(|dir| dir.join(part))).find(|path| path.is_file())
}

fn sheet(part: &str, dirs: &[PathBuf]) -> Result<Vec<c2f_lang::annotations::Spec>, String> {
    let path = find_part(part, dirs).ok_or_else(|| format!("cannot find the part `{part}`, as is, in `working_dir` or in the library folders"))?;
    let src = std::fs::read_to_string(&path).map_err(|err| format!("cannot read {}: {err}", Path::new(&path).display()))?;
    Ok(c2f_lang::annotations::read(&src).0.specs)
}

/// `scenario` with the figures it leaves out read from its parts' sheets,
/// found as is, then in `dirs`: the motor's (from `motor.part`, else the
/// motor support's `part`) and each bearing's static load.
pub fn with_sheets(scenario: &Scenario, dirs: &[PathBuf]) -> Result<Scenario, String> {
    let mut scenario = scenario.clone();
    let motor_part = scenario
        .motor
        .as_ref()
        .and_then(|m| m.part.clone())
        .or_else(|| scenario.supports.iter().find(|s| s.kind == SupportKind::Motor).and_then(|s| s.part.clone()));
    if let Some(part) = motor_part {
        let specs = sheet(&part, dirs)?;
        let motor = scenario.motor.get_or_insert_with(Motor::default);
        motor.part = Some(part);
        let fill = |field: &mut Option<f64>, key: &str| {
            if field.is_none() {
                *field = spec_figure(&specs, key);
            }
        };
        fill(&mut motor.torque_mnm, "torque");
        fill(&mut motor.stall_torque_mnm, "stall torque");
        fill(&mut motor.holding_torque_mnm, "holding torque");
        fill(&mut motor.speed_rpm, "speed");
        fill(&mut motor.max_radial_load_n, "max radial load");
        fill(&mut motor.max_axial_load_n, "max axial load");
    }
    for support in &mut scenario.supports {
        if let (Some(part), None, false) = (&support.part, support.static_load_n, support.kind == SupportKind::Motor) {
            support.static_load_n = spec_figure(&sheet(part, dirs)?, "static load");
        }
    }
    Ok(scenario)
}

type V3 = [f64; 3];

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}

/// The stages a reduction takes at [`STAGE_RATIO`] each.
pub fn stages_for(ratio: f64) -> u32 {
    if ratio <= 1.0 + 1e-9 { 0 } else { (ratio.ln() / STAGE_RATIO.ln() - 1e-9).ceil().max(1.0) as u32 }
}

/// A force on the shaft in one plane through the axis: `force` across the
/// axis at `s` along it, and a `couple` there, both in that plane.
#[derive(Clone, Copy, Debug)]
struct Event {
    s: f64,
    force: f64,
    couple: f64,
}

/// How the beam is held for its deflection.
#[derive(Clone, Copy, Debug)]
enum Held {
    /// Built in at one point: a support alone.
    Clamped(f64),
    /// Pinned at two: the outermost supports.
    Pinned(f64, f64),
}

/// The deflection at each of `at`, of a beam of stiffness `ei` (N·mm²)
/// under `events` (N, N·mm, mm), held as `held`. The bending moment is
/// linear between events, so integrating it twice from one event to the
/// next is exact: EI y'' = M, with M(x) the moment of the forces left of
/// x, F (x - s) for a force and -C for a couple.
fn deflections(events: &[Event], held: Held, ei: f64, at: &[f64]) -> Vec<f64> {
    let mut xs: Vec<f64> = events.iter().map(|e| e.s).chain(at.iter().copied()).collect();
    match held {
        Held::Clamped(c) => xs.push(c),
        Held::Pinned(a, b) => xs.extend([a, b]),
    }
    xs.sort_by(f64::total_cmp);
    xs.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    let moment = |x: f64, upto: f64| -> f64 {
        events.iter().filter(|e| e.s <= upto + 1e-9).map(|e| e.force * (x - e.s) - e.couple).sum()
    };
    // y and y' from the first point, starting level at zero.
    let (mut y, mut slope) = (vec![0.0; xs.len()], vec![0.0; xs.len()]);
    for i in 0..xs.len().saturating_sub(1) {
        let h = xs[i + 1] - xs[i];
        let (m0, m1) = (moment(xs[i], xs[i]), moment(xs[i + 1], xs[i]));
        slope[i + 1] = slope[i] + h * (m0 + m1) / (2.0 * ei);
        y[i + 1] = y[i] + h * slope[i] + h * h * (2.0 * m0 + m1) / (6.0 * ei);
    }
    let index = |x: f64| xs.iter().position(|p| (p - x).abs() < 1e-9).unwrap_or(0);
    let x0 = xs[0];
    // Add the line y0 + θ (x - x0) that meets how the beam is held.
    let (y0, theta) = match held {
        Held::Clamped(c) => {
            let i = index(c);
            let theta = -slope[i];
            (-y[i] - theta * (c - x0), theta)
        }
        Held::Pinned(a, b) => {
            let (i, j) = (index(a), index(b));
            let theta = -(y[j] - y[i]) / (b - a);
            (-y[i] - theta * (a - x0), theta)
        }
    };
    at.iter().map(|&x| y[index(x)] + y0 + theta * (x - x0)).collect()
}

/// The loads, reactions, margins and warnings of `scenario`, whose figures
/// are all given (see [`with_sheets`] for those of its parts' sheets).
///
/// - Torque: a load of mass m whose center sits at r from the axis takes
///   m g⊥ r at most in a turn, g⊥ the part of gravity across the axis;
///   the loads together take |Σ m r| g⊥, their unbalance. Friction adds,
///   at each support, μ (radial + axial) on the shaft's radius:
///   [`MU_BEARING`], [`MU_BUSHING`], [`MU_MOTOR`].
/// - Reactions: the shaft is a beam on point supports, in the two planes
///   through the axis, across and along gravity's part across it. One
///   support holds the loads' force and their moment about it; the motor
///   then carries an overhung load, on its front bearing the force plus
///   the moment over [`SUPPORT_SPAN_MM`]. Two supports are statically
///   determinate and solved exactly. With more, the shaft is taken as
///   rigid on supports of equal stiffness, which is the least-squares
///   (smallest) set of reactions in equilibrium; a support near the loads
///   still takes most. A motor on a coupling carries nothing. The thrust
///   along the axis goes to the first ball bearing, else to the motor,
///   else to the first support.
/// - Deflection: E I y'' = M along the shaft, I = π d⁴ / 64, built in at a
///   support alone or pinned at the two outermost.
/// - Margins: the motor's torque times the ratio and the drive's
///   efficiency over the torque needed; the motor's maximum radial and
///   axial loads over what its front bearing takes; a bearing's static
///   load rating over its equivalent static load.
pub fn analyse(scenario: &Scenario) -> Result<Report, String> {
    let shaft = &scenario.shaft;
    let length = norm(scenario.axis.direction);
    if !positive(length) || length < 1e-12 {
        return Err("`axis.direction` must be a vector that is not zero".into());
    }
    if !positive(shaft.d_mm) {
        return Err("`shaft.d_mm` must be more than 0".into());
    }
    if !positive(scenario.ratio) {
        return Err("`ratio` must be more than 0".into());
    }
    if !positive(scenario.target_margin) {
        return Err("`target_margin` must be more than 0".into());
    }
    if let Some(load) = scenario.loads.iter().find(|l| !(l.mass_g == 0.0 || positive(l.mass_g))) {
        return Err(format!("the load `{}` must have a mass of 0 g or more", load.name));
    }
    if let Some(e) = scenario.efficiency.filter(|e| !(positive(*e) && *e <= 1.0)) {
        return Err(format!("`efficiency` must be over 0 and at most 1, not {e}"));
    }
    if let Some(s) = scenario.supports.iter().find(|s| s.coupling && s.kind != SupportKind::Motor) {
        return Err(format!("`{}` is a {}: only a motor turns the shaft through a coupling", s.name, kind_name(s.kind)));
    }
    let e_gpa = match (&shaft.e_gpa, &shaft.material) {
        (Some(e), _) if positive(*e) => *e,
        (Some(e), _) => return Err(format!("`shaft.E_GPa` must be more than 0, not {e}")),
        (None, Some(material)) => young_modulus_gpa(material).ok_or_else(|| {
            format!("unknown shaft material `{material}`: steel, stainless, brass, aluminium, PLA, PETG, or give `E_GPa`")
        })?,
        (None, None) => young_modulus_gpa("steel").unwrap_or(210.0),
    };
    let held: Vec<usize> = (0..scenario.supports.len()).filter(|&i| !scenario.supports[i].coupling).collect();
    if held.is_empty() {
        return Err("nothing holds the shaft: give `supports`, a motor on a coupling holds nothing up".into());
    }

    let u = scale(scenario.axis.direction, 1.0 / length);
    let origin = scenario.axis.at;
    let gravity = [0.0, 0.0, -G];
    let across = sub(gravity, scale(u, dot(gravity, u)));
    let g_across = norm(across);
    // The two planes through the axis: along gravity's part across it,
    // and square to that (any pair when the axis is vertical).
    let e1 = if g_across > 1e-9 {
        scale(across, 1.0 / g_across)
    } else {
        let helper = if u[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
        let v = cross(u, helper);
        scale(v, 1.0 / norm(v))
    };
    let e2 = cross(u, e1);
    let planes = [e1, e2];

    // Each load: its weight across the axis in each plane, its moment
    // about the axis's point in each plane, and its offset.
    let (mut unbalance, mut torque_here, mut thrust) = ([0.0; 3], 0.0, 0.0);
    let mut forces = [Vec::new(), Vec::new()];
    let mut load_at = Vec::new();
    let mut offsets = Vec::new();
    for load in &scenario.loads {
        let mass = load.mass_g / 1000.0;
        let weight = scale(gravity, mass);
        let d = sub(load.at, origin);
        let s = dot(d, u);
        let r = sub(d, scale(u, s));
        unbalance = [unbalance[0] + mass * r[0], unbalance[1] + mass * r[1], unbalance[2] + mass * r[2]];
        torque_here += dot(u, cross(d, weight));
        thrust += dot(weight, u);
        for (k, e) in planes.iter().enumerate() {
            let force = dot(weight, *e);
            let moment = dot(cross(d, weight), cross(u, *e));
            forces[k].push(Event { s, force, couple: moment - s * force });
        }
        load_at.push(s);
        offsets.push(norm(r));
    }
    let static_torque = norm(unbalance) * g_across;

    // Reactions across the axis, in each plane, and the couple a support
    // alone holds.
    let at: Vec<f64> = held.iter().map(|&i| scenario.supports[i].at_mm).collect();
    let mut radial = vec![[0.0; 2]; held.len()];
    let mut couple = [0.0; 2];
    for k in 0..2 {
        let total_force: f64 = forces[k].iter().map(|e| e.force).sum();
        let total_moment: f64 = forces[k].iter().map(|e| e.s * e.force + e.couple).sum();
        if held.len() == 1 {
            radial[0][k] = -total_force;
            couple[k] = -total_moment + at[0] * total_force;
        } else {
            // The smallest reactions R with Σ R = -ΣF and Σ s R = -ΣM:
            // R = Aᵀ (A Aᵀ)⁻¹ b, A's rows 1 and s.
            let n = at.len() as f64;
            let (sum_s, sum_s2) = (at.iter().sum::<f64>(), at.iter().map(|s| s * s).sum::<f64>());
            let det = n * sum_s2 - sum_s * sum_s;
            if det.abs() < 1e-9 * (1.0 + sum_s2) {
                return Err(format!("the supports that hold the shaft are all at {} mm along it: give them distinct `at_mm`", num(at[0])));
            }
            let (b0, b1) = (-total_force, -total_moment);
            let l0 = (sum_s2 * b0 - sum_s * b1) / det;
            let l1 = (n * b1 - sum_s * b0) / det;
            for (j, s) in at.iter().enumerate() {
                radial[j][k] = l0 + l1 * s;
            }
        }
    }
    let alone_moment = if held.len() == 1 { couple[0].hypot(couple[1]) } else { 0.0 };

    // The thrust along the axis, to one support.
    let thrust_to = held
        .iter()
        .copied()
        .find(|&i| scenario.supports[i].kind == SupportKind::Bearing)
        .or_else(|| held.iter().copied().find(|&i| scenario.supports[i].kind == SupportKind::Motor))
        .unwrap_or(held[0]);

    let radius = shaft.d_mm / 2.0;
    let mut reactions = Vec::new();
    for (i, support) in scenario.supports.iter().enumerate() {
        let j = held.iter().position(|&h| h == i);
        let radial_n = j.map_or(0.0, |j| radial[j][0].hypot(radial[j][1]));
        let axial_n = if i == thrust_to { thrust.abs() } else { 0.0 };
        let moment_nmm = if j.is_some() { alone_moment } else { 0.0 };
        let span = support.span_mm.filter(|s| *s > 0.0).unwrap_or(SUPPORT_SPAN_MM);
        let (front, rear) = (radial_n + moment_nmm / span, moment_nmm / span);
        let mu = match support.kind {
            SupportKind::Bearing => MU_BEARING,
            SupportKind::Bushing => MU_BUSHING,
            SupportKind::Motor => MU_MOTOR,
        };
        reactions.push(Reaction {
            name: support.name.clone(),
            kind: support.kind,
            at_mm: support.at_mm,
            carries: j.is_some(),
            radial_n,
            axial_n,
            moment_nmm,
            bearing_load_n: front,
            friction_mnm: mu * (front + rear + axial_n) * radius,
        });
    }
    let friction: f64 = reactions.iter().map(|r| r.friction_mnm).sum();
    let needed = static_torque + friction;

    // Deflection, in each plane, then together.
    let ei = e_gpa * 1000.0 * std::f64::consts::PI * shaft.d_mm.powi(4) / 64.0;
    let hold = if held.len() == 1 {
        Held::Clamped(at[0])
    } else {
        let (lo, hi) = at.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &s| (lo.min(s), hi.max(s)));
        Held::Pinned(lo, hi)
    };
    let mut bend = [vec![0.0; load_at.len()], vec![0.0; load_at.len()]];
    for k in 0..2 {
        let mut events = forces[k].clone();
        events.extend(at.iter().zip(&radial).map(|(&s, r)| Event { s, force: r[k], couple: 0.0 }));
        if held.len() == 1 {
            events.push(Event { s: at[0], force: 0.0, couple: couple[k] });
        }
        bend[k] = deflections(&events, hold, ei, &load_at);
    }
    let loads: Vec<LoadReport> = scenario
        .loads
        .iter()
        .enumerate()
        .map(|(i, load)| LoadReport {
            name: load.name.clone(),
            mass_g: load.mass_g,
            at_mm: load_at[i],
            offset_mm: offsets[i],
            deflection_mm: bend[0][i].hypot(bend[1][i]),
        })
        .collect();
    let max_deflection = loads.iter().map(|l| l.deflection_mm).fold(0.0, f64::max);

    // The motor and the margins.
    let target = scenario.target_margin;
    let stages = scenario.stages.unwrap_or_else(|| stages_for(scenario.ratio));
    let efficiency = scenario.efficiency.unwrap_or(STAGE_EFFICIENCY.powi(stages as i32));
    let motor_support = scenario.supports.iter().position(|s| s.kind == SupportKind::Motor);
    let mut warnings = Vec::new();
    let mut summary = Vec::new();
    let motor = scenario.motor.clone().or_else(|| motor_support.map(|_| Motor::default()));
    let (mut torque_margin, mut ratio_needed, mut suggested_ratio) = (None, None, None);
    let mut motor_report = None;
    let (mut radial_margin, mut axial_margin) = (None, None);
    summary.push(format!(
        "The shaft needs {} mN·m: {} for the loads' weight in the worst position of a turn, {} of friction in its supports.",
        num(needed),
        num(static_torque),
        num(friction)
    ));
    if let Some(motor) = &motor {
        let (torque, from) = match (motor.torque_mnm, motor.stall_torque_mnm, motor.holding_torque_mnm) {
            (Some(t), _, _) => (Some(t), Some("torque")),
            // A DC motor runs at half its stall torque at best, its
            // maximum power; counting on more stalls it.
            (None, Some(stall), _) => (Some(stall / 2.0), Some("half the stall torque")),
            (None, None, Some(holding)) => (Some(holding), Some("holding torque")),
            _ => (None, None),
        };
        let at_shaft = torque.map(|t| t * scenario.ratio * efficiency);
        let (radial_load, axial_load) = motor_support.map_or((0.0, 0.0), |i| (reactions[i].bearing_load_n, reactions[i].axial_n));
        match at_shaft {
            Some(at_shaft) => {
                let mut how = String::new();
                if scenario.ratio != 1.0 {
                    how = format!(" through {}:1 at {}% efficiency", num(scenario.ratio), num(efficiency * 100.0));
                }
                if scenario.ratio != 1.0 || from != Some("torque") {
                    let figure = match from {
                        Some("torque") | None => "its rated torque",
                        Some(other) => other,
                    };
                    how.push_str(&format!(" (counting {} mN·m, {figure})", num(torque.unwrap_or(0.0))));
                }
                if needed > 0.0 {
                    let margin = at_shaft / needed;
                    torque_margin = Some(margin);
                    summary.push(format!(
                        "The motor gives {} mN·m at the shaft{how}: a torque margin of {}, for a target of {}.",
                        num(at_shaft),
                        num(margin),
                        num(target)
                    ));
                    if margin < target {
                        let t = torque.unwrap_or(0.0);
                        let found = RATIOS.iter().copied().filter(|&r| r > scenario.ratio).find(|&r| t * r * STAGE_EFFICIENCY.powi(stages_for(r) as i32) >= target * needed);
                        let mut warning = format!(
                            "the motor gives {} mN·m at the shaft for {} needed: a margin of {}, under {}",
                            num(at_shaft),
                            num(needed),
                            num(margin),
                            num(target)
                        );
                        match found {
                            Some(r) => {
                                let eff = STAGE_EFFICIENCY.powi(stages_for(r) as i32);
                                ratio_needed = Some(target * needed / (t * eff));
                                suggested_ratio = Some(r);
                                warning.push_str(&format!(
                                    "; a {}:1 reduction ({} mN·m at the shaft) or a stronger motor reaches it",
                                    num(r),
                                    num(t * r * eff)
                                ));
                                summary.push(format!(
                                    "A {}:1 reduction ({} {}) brings it to {}; the shaft then turns at {}.",
                                    num(r),
                                    stages_for(r),
                                    if stages_for(r) == 1 { "stage" } else { "stages" },
                                    num(t * r * eff / needed),
                                    motor.speed_rpm.map_or("the motor's speed over the ratio".to_string(), |s| format!("{} rpm", num(s / r)))
                                ));
                            }
                            None => warning.push_str("; no reduction up to 1000:1 does: take a stronger motor"),
                        }
                        warnings.push(warning);
                    }
                }
            }
            None => warnings.push("no motor torque: give `motor.torque_mNm` (or `stall_torque_mNm`), or a motor part whose sheet has `// @spec torque`".into()),
        }
        if let Some(i) = motor_support {
            let support = &reactions[i];
            if held.len() == 1 && held[0] == i && (support.radial_n > 1e-9 || support.moment_nmm > 1e-9) {
                let arm = if support.radial_n > 1e-9 { support.moment_nmm / support.radial_n } else { 0.0 };
                warnings.push(format!(
                    "the motor `{}` holds the shaft alone, an overhung load: {} N hanging {} mm past it, {} N·mm on its bearings, {} N on its front one; a bearing near the loads takes it off the motor",
                    support.name,
                    num(support.radial_n),
                    num(arm),
                    num(support.moment_nmm),
                    num(support.bearing_load_n)
                ));
            }
            if let Some(max) = motor.max_radial_load_n {
                if radial_load > 1e-9 {
                    radial_margin = Some(max / radial_load);
                }
            }
            if let Some(max) = motor.max_axial_load_n {
                if axial_load > 1e-9 {
                    axial_margin = Some(max / axial_load);
                }
            }
            summary.push(match motor.max_radial_load_n {
                Some(max) => format!(
                    "The motor's front bearing takes {} N radially, for a maximum of {} N{}.",
                    num(radial_load),
                    num(max),
                    radial_margin.map_or(String::new(), |m| format!(": a margin of {}", num(m)))
                ),
                None => format!("The motor's front bearing takes {} N radially (its maximum radial load is not given).", num(radial_load)),
            });
            if let Some(m) = radial_margin.filter(|m| *m < target) {
                warnings.push(format!(
                    "the motor's front bearing takes {} N, its maximum radial load is {} N: a margin of {}, under {}",
                    num(radial_load),
                    num(motor.max_radial_load_n.unwrap_or(0.0)),
                    num(m),
                    num(target)
                ));
            }
            if let Some(m) = axial_margin.filter(|m| *m < target) {
                warnings.push(format!(
                    "the motor takes {} N along its shaft, its maximum axial load is {} N: a margin of {}, under {}",
                    num(axial_load),
                    num(motor.max_axial_load_n.unwrap_or(0.0)),
                    num(m),
                    num(target)
                ));
            }
        }
        motor_report = Some(MotorReport {
            part: motor.part.clone(),
            torque_mnm: torque,
            torque_from: from.map(str::to_string),
            at_shaft_mnm: at_shaft,
            ratio: scenario.ratio,
            efficiency,
            speed_rpm: motor.speed_rpm,
            shaft_speed_rpm: motor.speed_rpm.map(|s| s / scenario.ratio),
            max_radial_load_n: motor.max_radial_load_n,
            max_axial_load_n: motor.max_axial_load_n,
            radial_load_n: radial_load,
            axial_load_n: axial_load,
        });
    } else {
        summary.push("No motor is given: only the loads on the shaft are computed.".into());
    }

    // A support other than the motor that holds the shaft alone.
    if held.len() == 1 && scenario.supports[held[0]].kind != SupportKind::Motor && alone_moment > 1e-9 {
        let support = &reactions[held[0]];
        warnings.push(format!(
            "`{}` holds the shaft alone and with it a moment of {} N·mm, which one {} barely takes: a second support along the shaft (another bearing, or the motor without a coupling) shares it",
            support.name,
            num(support.moment_nmm),
            kind_name(support.kind)
        ));
    }
    for reaction in reactions.iter().filter(|r| r.carries && Some(r.name.as_str()) != motor_support.map(|i| reactions[i].name.as_str())) {
        summary.push(format!(
            "`{}` ({}) takes {} N radially{}.",
            reaction.name,
            kind_name(reaction.kind),
            num(reaction.bearing_load_n),
            if reaction.axial_n > 1e-9 { format!(" and {} N along the axis", num(reaction.axial_n)) } else { String::new() }
        ));
    }

    let mut bearings = Vec::new();
    for (support, reaction) in scenario.supports.iter().zip(&reactions) {
        let Some(c0) = support.static_load_n.filter(|_| support.kind == SupportKind::Bearing) else { continue };
        let fr = reaction.bearing_load_n;
        let p0 = fr.max(0.6 * fr + 0.5 * reaction.axial_n);
        let margin = (p0 > 1e-9).then(|| c0 / p0);
        if let Some(m) = margin.filter(|m| *m < target) {
            warnings.push(format!(
                "the bearing `{}` takes {} N (equivalent static load), its static load rating is {} N: a margin of {}, under {}",
                support.name,
                num(p0),
                num(c0),
                num(m),
                num(target)
            ));
        }
        bearings.push(BearingMargin { name: support.name.clone(), static_load_n: c0, equivalent_load_n: p0, margin });
    }

    if let Some(worst) = loads.iter().max_by(|a, b| a.deflection_mm.total_cmp(&b.deflection_mm)) {
        summary.push(format!(
            "The shaft ({} mm, E = {} GPa) bends {} mm at `{}`.",
            num(shaft.d_mm),
            num(e_gpa),
            num(worst.deflection_mm),
            worst.name
        ));
        if worst.deflection_mm > VISIBLE_DEFLECTION_MM {
            warnings.push(format!(
                "the shaft bends {} mm at `{}`, a wobble one sees: a thicker or stiffer shaft, or a support nearer the load",
                num(worst.deflection_mm),
                worst.name
            ));
        }
    }

    Ok(Report {
        torque: Torque { static_mnm: static_torque, here_mnm: torque_here.abs(), friction_mnm: friction, needed_mnm: needed },
        supports: reactions,
        loads,
        max_deflection_mm: max_deflection,
        motor: motor_report,
        margins: Margins { torque: torque_margin, motor_radial: radial_margin, motor_axial: axial_margin, bearings },
        target_margin: target,
        ratio_needed,
        suggested_ratio,
        warnings,
        summary,
    })
}

/// A finite number over zero.
fn positive(x: f64) -> bool {
    x > 0.0 && x.is_finite()
}

fn kind_name(kind: SupportKind) -> &'static str {
    match kind {
        SupportKind::Motor => "motor",
        SupportKind::Bearing => "bearing",
        SupportKind::Bushing => "bushing",
    }
}

/// A figure to three significant digits, for the sentences.
pub fn num(x: f64) -> String {
    if x == 0.0 || !x.is_finite() {
        return format!("{x}");
    }
    let digits = (2 - x.abs().log10().floor() as i32).max(0) as usize;
    let text = format!("{x:.digits$}");
    if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.').to_string() } else { text }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() <= tolerance * b.abs().max(1e-12)
    }

    fn scenario(json: &str) -> Scenario {
        serde_json::from_str(json).unwrap()
    }

    /// The watch winder: a 150 g watch on a cradle 35 mm in front of the
    /// motor's face, 20 mm off a horizontal shaft along x.
    const WINDER: &str = r#"{
        "axis": { "at": [0, 0, 0], "direction": [1, 0, 0] },
        "loads": [{ "name": "watch", "mass_g": 150, "at": [35, 20, 0] }],
        "supports": [{ "name": "motor", "at_mm": 0, "kind": "motor" }],
        "motor": { "torque_mNm": 34, "max_radial_load_N": 10, "speed_rpm": 15 },
        "shaft": { "d_mm": 5, "material": "steel" }
    }"#;

    #[test]
    fn a_mass_off_a_horizontal_axis_takes_its_weight_times_its_offset() {
        let report = analyse(&scenario(WINDER)).unwrap();
        // 0.150 kg × 9.80665 m/s² × 20 mm = 29.42 N·mm = 29.42 mN·m.
        assert!(close(report.torque.static_mnm, 29.42, 1e-3), "{report:?}");
        assert!(close(report.torque.here_mnm, 29.42, 1e-3), "{report:?}");
        assert!(report.torque.needed_mnm > report.torque.static_mnm);
        // Below the axis, the same mass takes nothing here, as much in a turn.
        let below = scenario(&WINDER.replace("[35, 20, 0]", "[35, 0, -20]"));
        let report = analyse(&below).unwrap();
        assert!(report.torque.here_mnm < 1e-9 && close(report.torque.static_mnm, 29.42, 1e-3));
        // Along a vertical axis, gravity turns nothing.
        let vertical = scenario(&WINDER.replace("\"direction\": [1, 0, 0]", "\"direction\": [0, 0, 1]").replace("[35, 20, 0]", "[20, 0, 35]"));
        assert!(analyse(&vertical).unwrap().torque.static_mnm < 1e-9);
    }

    #[test]
    fn the_motor_alone_carries_an_overhung_load_that_a_front_bearing_takes_off_it() {
        let report = analyse(&scenario(WINDER)).unwrap();
        let motor = &report.supports[0];
        let weight = 0.150 * G;
        assert!(close(motor.radial_n, weight, 1e-9), "{motor:?}");
        assert!(close(motor.moment_nmm, weight * 35.0, 1e-9), "{motor:?}");
        assert!(close(motor.bearing_load_n, weight * (1.0 + 35.0 / SUPPORT_SPAN_MM), 1e-9), "{motor:?}");
        assert!(report.warnings.iter().any(|w| w.contains("overhung")), "{:?}", report.warnings);
        // 10 N against 7.9 N on its front bearing: under the target of 2.
        assert!(report.margins.motor_radial.is_some_and(|m| m < 2.0));
        assert!(report.warnings.iter().any(|w| w.contains("maximum radial load")), "{:?}", report.warnings);

        // A bearing 5 mm behind the watch, the motor still a support.
        let front = WINDER.replace(
            r#""supports": [{ "name": "motor", "at_mm": 0, "kind": "motor" }]"#,
            r#""supports": [{ "name": "front", "at_mm": 30, "kind": "bearing", "static_load_N": 340 }, { "name": "motor", "at_mm": 0, "kind": "motor" }]"#,
        );
        let report = analyse(&scenario(&front)).unwrap();
        let (bearing, motor) = (&report.supports[0], &report.supports[1]);
        // Statics: the bearing takes 35/30 of the weight, the motor 5/30.
        assert!(close(bearing.radial_n, weight * 35.0 / 30.0, 1e-9), "{bearing:?}");
        assert!(close(motor.radial_n, weight * 5.0 / 30.0, 1e-9), "{motor:?}");
        assert!(motor.bearing_load_n < 0.05 * weight * (1.0 + 35.0 / SUPPORT_SPAN_MM), "{motor:?}");
        assert!(!report.warnings.iter().any(|w| w.contains("overhung")), "{:?}", report.warnings);
        assert!(report.margins.bearings[0].margin.is_some_and(|m| m > 100.0));

        // On a coupling, the motor carries nothing.
        let coupled = front.replace(r#""kind": "motor" }"#, r#""kind": "motor", "coupling": true }"#);
        let report = analyse(&scenario(&coupled)).unwrap();
        let motor = &report.supports[1];
        assert!(!motor.carries && motor.radial_n == 0.0 && motor.bearing_load_n == 0.0, "{motor:?}");
        assert_eq!(report.margins.motor_radial, None);
        // The bearing alone then holds the 5 mm overhang's moment.
        assert!(close(report.supports[0].moment_nmm, weight * 5.0, 1e-9));
        assert!(report.warnings.iter().any(|w| w.contains("`front` holds the shaft alone")), "{:?}", report.warnings);
    }

    #[test]
    fn a_weak_motor_gets_a_reduction() {
        let weak = scenario(&WINDER.replace("\"torque_mNm\": 34", "\"torque_mNm\": 5"));
        let report = analyse(&weak).unwrap();
        let margin = report.margins.torque.unwrap();
        assert!(close(margin, 5.0 / report.torque.needed_mnm, 1e-9) && margin < 1.0);
        assert!(report.warnings.iter().any(|w| w.starts_with("the motor gives 5 mN·m")), "{:?}", report.warnings);
        let exact = 2.0 * report.torque.needed_mnm / 5.0;
        let suggested = report.suggested_ratio.unwrap();
        assert!(suggested >= exact && suggested >= report.ratio_needed.unwrap(), "{suggested} {exact}");
        assert!(RATIOS.contains(&suggested));
        // It is enough, with its stages' losses, and the one before is not.
        let enough = |r: f64| 5.0 * r * STAGE_EFFICIENCY.powi(stages_for(r) as i32) >= 2.0 * report.torque.needed_mnm;
        assert!(enough(suggested));
        let before = RATIOS[RATIOS.iter().position(|&r| r == suggested).unwrap() - 1];
        assert!(!enough(before));
        // Through that reduction, the margin is met.
        let geared = scenario(&WINDER.replace("\"torque_mNm\": 34", "\"torque_mNm\": 5").replace("\"shaft\"", &format!("\"ratio\": {suggested}, \"shaft\"")));
        let report = analyse(&geared).unwrap();
        assert!(report.margins.torque.unwrap() >= 2.0 && report.suggested_ratio.is_none(), "{report:?}");
        assert!(report.motor.unwrap().shaft_speed_rpm.is_some_and(|s| close(s, 15.0 / suggested, 1e-9)));
    }

    #[test]
    fn a_cantilever_bends_as_the_hand_formula_says() {
        // 1 kg on the axis 50 mm past a motor, a 5 mm steel shaft:
        // δ = P L³ / (3 E I), I = π d⁴ / 64.
        let json = WINDER.replace("\"mass_g\": 150, \"at\": [35, 20, 0]", "\"mass_g\": 1000, \"at\": [50, 0, 0]");
        let report = analyse(&scenario(&json)).unwrap();
        let i = std::f64::consts::PI * 5f64.powi(4) / 64.0;
        let expected = G * 50f64.powi(3) / (3.0 * 210_000.0 * i);
        assert!(close(report.loads[0].deflection_mm, expected, 1e-9), "{} {expected}", report.loads[0].deflection_mm);
        // Simply supported at its ends, under a load in the middle:
        // δ = P L³ / (48 E I).
        let json = json.replace(
            r#"[{ "name": "motor", "at_mm": 0, "kind": "motor" }]"#,
            r#"[{ "name": "motor", "at_mm": 0, "kind": "motor" }, { "name": "end", "at_mm": 100, "kind": "bushing" }]"#,
        );
        let report = analyse(&scenario(&json)).unwrap();
        let expected = G * 100f64.powi(3) / (48.0 * 210_000.0 * i);
        assert!(close(report.loads[0].deflection_mm, expected, 1e-9), "{} {expected}", report.loads[0].deflection_mm);
        // PLA bends sixty times as much.
        let pla = analyse(&scenario(&json.replace("\"material\": \"steel\"", "\"material\": \"PLA\""))).unwrap();
        assert!(close(pla.loads[0].deflection_mm, expected * 210.0 / 3.5, 1e-9));
    }

    #[test]
    fn more_supports_share_the_load_and_bad_scenarios_are_refused() {
        let json = WINDER.replace(
            r#"[{ "name": "motor", "at_mm": 0, "kind": "motor" }]"#,
            r#"[{ "name": "motor", "at_mm": 0, "kind": "motor" }, { "name": "a", "at_mm": 30, "kind": "bushing" }, { "name": "b", "at_mm": 40, "kind": "bushing" }]"#,
        );
        let report = analyse(&scenario(&json)).unwrap();
        let total: f64 = report.supports.iter().map(|s| s.radial_n).sum();
        assert!(total >= 0.150 * G - 1e-9);
        assert!(report.supports[1].radial_n > report.supports[0].radial_n && report.supports[2].radial_n > report.supports[0].radial_n);
        let bad = |from: &str, to: &str| analyse(&scenario(&WINDER.replace(from, to))).unwrap_err();
        assert!(bad("[1, 0, 0]", "[0, 0, 0]").contains("axis.direction"));
        assert!(bad("\"steel\"", "\"wood\"").contains("unknown shaft material"));
        assert!(bad("\"kind\": \"motor\"", "\"kind\": \"motor\", \"coupling\": true").contains("nothing holds the shaft"));
        assert!(serde_json::from_str::<Scenario>(&WINDER.replace("\"mass_g\"", "\"mass\"")).is_err());
    }

    #[test]
    fn the_motor_figures_come_from_its_part_sheet() {
        let dir = std::env::temp_dir().join(format!("c2f-loads-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("electronics")).unwrap();
        std::fs::write(
            dir.join("electronics/gear_motor.c2f"),
            "// A small gear motor.\n// @spec torque: 34 mN·m at 5 V\n// @spec stall torque: 0.8 kgf·cm\n// @spec speed: 15 rpm\n// @spec max radial load: 20 N\n// @spec max axial load: 5 N\ncylinder(d = 12, h = 10);\n",
        )
        .unwrap();
        std::fs::write(dir.join("electronics/bearing_625.c2f"), "// @spec static load: 0.33 kN\ncylinder(d = 16, h = 5);\n").unwrap();
        let json = r#"{
            "axis": { "at": [0, 0, 0], "direction": [1, 0, 0] },
            "loads": [{ "name": "watch", "mass_g": 150, "at": [35, 20, 0] }],
            "supports": [
                { "name": "motor", "at_mm": 0, "kind": "motor", "part": "electronics/gear_motor.c2f" },
                { "name": "front", "at_mm": 30, "kind": "bearing", "part": "electronics/bearing_625.c2f" }
            ],
            "motor": { "speed_rpm": 12 }
        }"#;
        let filled = with_sheets(&scenario(json), &[dir.clone()]).unwrap();
        let motor = filled.motor.as_ref().unwrap();
        assert_eq!((motor.torque_mnm, motor.max_radial_load_n, motor.max_axial_load_n), (Some(34.0), Some(20.0), Some(5.0)));
        assert!(close(motor.stall_torque_mnm.unwrap(), 78.4532, 1e-6));
        // The scenario's own figure wins over the sheet's.
        assert_eq!(motor.speed_rpm, Some(12.0));
        assert_eq!(filled.supports[1].static_load_n, Some(330.0));
        let report = analyse(&filled).unwrap();
        assert_eq!(report.motor.as_ref().unwrap().part.as_deref(), Some("electronics/gear_motor.c2f"));
        assert!(close(report.margins.torque.unwrap(), 34.0 / report.torque.needed_mnm, 1e-9));
        assert!(with_sheets(&scenario(&json.replace("gear_motor", "nope")), &[dir.clone()]).unwrap_err().contains("cannot find the part"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn figures_read_to_three_digits() {
        assert_eq!(num(29.41995), "29.4");
        assert_eq!(num(1.4709975), "1.47");
        assert_eq!(num(1471.0), "1471");
        assert_eq!(num(0.003_26), "0.00326");
        assert_eq!(num(2.0), "2");
    }
}
