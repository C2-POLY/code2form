//! What a model weighs once printed, where its center of gravity is, and
//! how far it leans before it tips over: measured on the model, so that a
//! mechanical review does not guess them.
//!
//! Each `part()` (and the root, what is outside any) is printed in a
//! material, with walls and infill; a circuit adds the parts it places,
//! whose sheet gives their mass, and the caller can add masses the model
//! carries (a watch on a winder's cradle). Volumes are in cubic
//! millimeters, masses in grams, places in the model's millimeters.

use std::path::Path;

use c2f_eval::node::Node;

use crate::build::{Model, transform};
use crate::export::colored_mesh;
use crate::pieces::pieces;

/// The materials people print with, and their density in g/cm³.
pub const MATERIALS: &[(&str, f64)] =
    &[("PLA", 1.24), ("PETG", 1.27), ("ABS", 1.04), ("ASA", 1.07), ("TPU", 1.21), ("nylon", 1.14)];

/// Two perimeters of a 0.4 mm nozzle, in millimeters.
pub const DEFAULT_WALL: f64 = 0.8;

/// A slicer's usual infill, in percent.
pub const DEFAULT_INFILL: f64 = 20.0;

/// How high above the lowest point a point still rests on the ground, in
/// millimeters: a bottom face tessellated or printed a hair uneven still
/// stands on all of it.
pub const BASE_TOLERANCE: f64 = 0.2;

/// What a part is printed in: its name as written (`PLA`, or a density as
/// `1.3 g/cm³`) and its density in g/cm³.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    pub density: f64,
}

impl Material {
    /// A material of [`MATERIALS`] by name, whatever its case, or a
    /// density in g/cm³ (`1.3`).
    pub fn named(text: &str) -> Result<Material, String> {
        let text = text.trim();
        if let Some(&(name, density)) = MATERIALS.iter().find(|(name, _)| name.eq_ignore_ascii_case(text)) {
            return Ok(Material { name: name.into(), density });
        }
        match text.parse::<f64>() {
            Ok(density) => Material::density(density),
            Err(_) => {
                let names: Vec<&str> = MATERIALS.iter().map(|(name, _)| *name).collect();
                Err(format!("unknown material `{text}`: give {} or a density in g/cm³, as 1.3", names.join(", ")))
            }
        }
    }

    /// A material known only by its density, in g/cm³.
    pub fn density(density: f64) -> Result<Material, String> {
        if !(density.is_finite() && density > 0.0) {
            return Err(format!("a density is a positive number of g/cm³, not {density}"));
        }
        Ok(Material { name: format!("{density} g/cm³"), density })
    }
}

impl Default for Material {
    fn default() -> Self {
        Material { name: "PLA".into(), density: 1.24 }
    }
}

/// How a part is printed.
#[derive(Clone, Debug, PartialEq)]
pub struct Print {
    pub material: Material,
    /// The share of the inside that is filled, in percent: 100 is solid.
    pub infill: f64,
    /// The thickness of its walls, top and bottom, in millimeters.
    pub wall: f64,
}

impl Default for Print {
    fn default() -> Self {
        Print { material: Material::default(), infill: DEFAULT_INFILL, wall: DEFAULT_WALL }
    }
}

impl Print {
    /// Why these settings cannot be printed, if they cannot.
    pub fn check(&self) -> Result<(), String> {
        if !(self.infill.is_finite() && (0.0..=100.0).contains(&self.infill)) {
            return Err(format!("infill is a percentage from 0 to 100, not {}", self.infill));
        }
        if !(self.wall.is_finite() && self.wall > 0.0) {
            return Err(format!("wall is a positive thickness in millimeters, not {}", self.wall));
        }
        Ok(())
    }

    /// What a solid of `volume` mm³ and `area` mm² of surface weighs
    /// printed so, in grams. An estimate of a slicer's figure: the walls
    /// are the surface times their thickness (no more than the solid),
    /// and the infill fills its share of the rest.
    pub fn mass(&self, volume: f64, area: f64) -> f64 {
        let shell = (area * self.wall).min(volume);
        let filled = shell + self.infill / 100.0 * (volume - shell);
        filled * self.material.density / 1000.0
    }
}

/// A part of a circuit, whose mass its sheet gives (`// @spec mass`).
#[derive(Clone, Debug, PartialEq)]
pub struct Component {
    pub name: String,
    /// In grams; `None` when its sheet gives none.
    pub mass: Option<f64>,
    /// Its bounding box where the circuit places it.
    pub bounds: ([f64; 3], [f64; 3]),
}

impl Component {
    /// Its bounding box's center, where its mass is taken to be.
    pub fn center(&self) -> [f64; 3] {
        let (lo, hi) = self.bounds;
        [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0)
    }
}

/// A mass the model carries that it does not draw.
#[derive(Clone, Debug, PartialEq)]
pub struct Extra {
    pub name: String,
    /// In grams.
    pub mass: f64,
    pub at: [f64; 3],
}

/// What [`mass_properties`] measures with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MassOptions {
    /// How every part is printed, unless `parts` says otherwise.
    pub print: Print,
    /// How a part is printed, by the name the report gives it (`root` for
    /// what is outside any part).
    pub parts: Vec<(String, Print)>,
    /// A circuit's parts.
    pub components: Vec<Component>,
    pub extra: Vec<Extra>,
}

/// A printed part.
#[derive(Clone, Debug, PartialEq)]
pub struct PartMass {
    /// Unique among the parts: `lid`, then `lid 2`; `root` for what is
    /// outside any part.
    pub name: String,
    /// Of the solid, in mm³.
    pub volume: f64,
    /// Of its surface, in mm².
    pub area: f64,
    /// Printed, in grams.
    pub mass: f64,
    /// The solid's centroid, which the printed part's is taken to be:
    /// walls and infill spread evenly enough for a balance.
    pub center: [f64; 3],
    pub print: Print,
}

/// Whether the model stands, and how far it leans before it tips.
#[derive(Clone, Debug, PartialEq)]
pub struct Stability {
    /// Whether the center of gravity is above the base.
    pub inside: bool,
    /// The least angle the model must lean to tip over, in degrees;
    /// negative when it falls already.
    pub min_tipping_deg: f64,
    /// Which way it tips first: the outward normal of the base's edge it
    /// pivots on, as `[x, y]`.
    pub direction: [f64; 2],
    /// From the center of gravity, seen from above, to that edge, in
    /// millimeters: negative outside the base.
    pub margin: f64,
    /// Of the center of gravity above the base, in millimeters.
    pub height: f64,
    /// The support polygon: the convex hull, counter-clockwise, of the
    /// points within [`BASE_TOLERANCE`] of the lowest.
    pub base: Vec<[f64; 2]>,
    /// The base's height: the model's lowest point.
    pub base_z: f64,
}

/// What a model weighs and how it stands.
#[derive(Clone, Debug, PartialEq)]
pub struct MassReport {
    pub parts: Vec<PartMass>,
    pub components: Vec<Component>,
    pub extra: Vec<Extra>,
    /// Everything that has a mass, in grams.
    pub total: f64,
    /// The mass-weighted center of all of it; `None` when nothing weighs.
    pub center: Option<[f64; 3]>,
    /// `None` when nothing rests on the ground (no geometry).
    pub stability: Option<Stability>,
    pub warnings: Vec<String>,
}

/// Renders `nodes`, data files read relative to `base_dir`, and weighs
/// each part printed as `options` says, with the circuit's parts and the
/// extra masses, then finds the center of gravity and how the model
/// stands on its lowest face. Fails on a part setting that names no part
/// or cannot be printed, and when there is nothing to weigh.
pub fn mass_properties(nodes: &[Node], base_dir: &Path, options: &MassOptions) -> Result<MassReport, String> {
    options.print.check()?;
    let split = pieces(nodes, base_dir, "root");
    let mut warnings = split.warnings;
    let names: Vec<&str> = split.pieces.iter().map(|p| p.name.as_str()).collect();
    for (name, print) in &options.parts {
        if !names.contains(&name.as_str()) {
            return Err(format!("no part named `{name}`: the model's parts are {}", names.join(", ")));
        }
        print.check().map_err(|why| format!("part `{name}`: {why}"))?;
    }
    let mut parts = Vec::new();
    // Every vertex, for the base.
    let mut points: Vec<[f64; 3]> = Vec::new();
    for piece in &split.pieces {
        let placed = match transform(piece.model.clone(), &piece.world) {
            Model::Solid(solid) if !solid.is_empty() => solid,
            Model::Flat(flat) if !flat.is_empty() => {
                warnings.push(format!("`{}` is 2D: it has no volume, so no mass", piece.name));
                continue;
            }
            _ => continue,
        };
        let mesh = colored_mesh(&placed);
        let (volume, area, center) = measure(&mesh.vertices, &mesh.triangles);
        let print = options.parts.iter().find(|(name, _)| *name == piece.name).map_or(&options.print, |(_, print)| print).clone();
        parts.push(PartMass { name: piece.name.clone(), volume, area, mass: print.mass(volume, area), center, print });
        points.extend(mesh.vertices);
    }
    for component in &options.components {
        if component.mass.is_none() {
            warnings.push(format!(
                "`{}` has no mass on its sheet (`// @spec mass: 3.3 g` in its file gives one): it is left out of the total",
                component.name
            ));
        }
        let (lo, hi) = component.bounds;
        points.extend((0..8).map(|corner| [0, 1, 2].map(|k| if (corner >> k) & 1 == 0 { lo[k] } else { hi[k] })));
    }
    for extra in &options.extra {
        if !(extra.mass.is_finite() && extra.mass >= 0.0) {
            return Err(format!("extra `{}`: a mass is a positive number of grams, not {}", extra.name, extra.mass));
        }
        if !extra.at.iter().all(|c| c.is_finite()) {
            return Err(format!("extra `{}`: `at` is a point [x, y, z] in millimeters", extra.name));
        }
    }
    if parts.is_empty() && options.components.is_empty() && options.extra.is_empty() {
        return Err("the model is empty or 2D: nothing to weigh".into());
    }
    let masses = parts
        .iter()
        .map(|p| (p.mass, p.center))
        .chain(options.components.iter().filter_map(|c| Some((c.mass?, c.center()))))
        .chain(options.extra.iter().map(|e| (e.mass, e.at)));
    let (mut total, mut moment) = (0.0, [0.0; 3]);
    for (mass, at) in masses {
        total += mass;
        for k in 0..3 {
            moment[k] += mass * at[k];
        }
    }
    let center = (total > 0.0).then(|| moment.map(|m| m / total));
    let stability = center.and_then(|center| stability(&points, center));
    if let Some(stability) = &stability {
        if !stability.inside {
            warnings.push(format!(
                "the center of gravity is {:.1} mm outside the base: the model falls over toward [{:.2}, {:.2}]",
                -stability.margin, stability.direction[0], stability.direction[1]
            ));
        }
    }
    Ok(MassReport { parts, components: options.components.clone(), extra: options.extra.clone(), total, center, stability, warnings })
}

/// A closed mesh's volume, surface area and centroid: the volume and
/// centroid are exact, summed over the signed tetrahedra from the origin
/// to each triangle.
pub(crate) fn measure(vertices: &[[f64; 3]], triangles: &[[usize; 3]]) -> (f64, f64, [f64; 3]) {
    let (mut volume, mut area, mut moment) = (0.0, 0.0, [0.0; 3]);
    for t in triangles {
        let [a, b, c] = t.map(|v| vertices[v]);
        let v = (a[0] * (b[1] * c[2] - b[2] * c[1]) + a[1] * (b[2] * c[0] - b[0] * c[2]) + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.0;
        volume += v;
        for k in 0..3 {
            moment[k] += v * (a[k] + b[k] + c[k]) / 4.0;
        }
        let (u, w) = ([0, 1, 2].map(|k| b[k] - a[k]), [0, 1, 2].map(|k| c[k] - a[k]));
        let cross = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
        area += (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt() / 2.0;
    }
    let center = if volume != 0.0 { moment.map(|m| m / volume) } else { [0.0; 3] };
    (volume, area, center)
}

/// How the model stands on `points`' lowest, its center of gravity at
/// `center`; `None` without points.
fn stability(points: &[[f64; 3]], center: [f64; 3]) -> Option<Stability> {
    let base_z = points.iter().map(|p| p[2]).reduce(f64::min)?;
    let base = hull(points.iter().filter(|p| p[2] <= base_z + BASE_TOLERANCE).map(|p| [p[0], p[1]]).collect());
    let height = center[2] - base_z;
    let at = [center[0], center[1]];
    // Each edge's outward normal and the margin to it, positive inside.
    let edges: Vec<([f64; 2], f64)> = match base.len() {
        0 => return None,
        1 => {
            let d = [at[0] - base[0][0], at[1] - base[0][1]];
            let length = d[0].hypot(d[1]);
            let direction = if length > 0.0 { [d[0] / length, d[1] / length] } else { [1.0, 0.0] };
            vec![(direction, -length)]
        }
        n => (0..n)
            .filter_map(|i| {
                let (a, b) = (base[i], base[(i + 1) % n]);
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let length = dx.hypot(dy);
                if length == 0.0 {
                    return None;
                }
                // Counter-clockwise, the outside is on the right.
                let normal = [dy / length, -dx / length];
                let margin = -((at[0] - a[0]) * normal[0] + (at[1] - a[1]) * normal[1]);
                Some((normal, margin))
            })
            .collect(),
    };
    let &(direction, margin) = edges.iter().min_by(|a, b| a.1.total_cmp(&b.1))?;
    // A line or a point holds nothing up: a segment's two sides give a
    // margin of zero or less already.
    let inside = base.len() >= 3 && margin > 1e-9;
    Some(Stability {
        inside,
        min_tipping_deg: margin.atan2(height).to_degrees(),
        direction: direction.map(|c| c + 0.0),
        margin,
        height,
        base,
        base_z,
    })
}

/// The convex hull of `points`, counter-clockwise, without the points
/// along its edges (Andrew's monotone chain).
fn hull(mut points: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut lower: Vec<[f64; 2]> = Vec::new();
    for &p in &points {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 1e-12 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<[f64; 2]> = Vec::new();
    for &p in points.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 1e-12 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weigh(src: &str, options: &MassOptions) -> MassReport {
        let program = c2f_eval::Program::load(src, Path::new("t.scad"), &[]).expect("parses");
        mass_properties(&c2f_eval::evaluate(&program).nodes, Path::new(""), options).expect("weighs")
    }

    fn solid(material: &str) -> Print {
        Print { material: Material::named(material).unwrap(), infill: 100.0, wall: DEFAULT_WALL }
    }

    fn close(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() <= tolerance
    }

    #[test]
    fn a_solid_cube_of_pla_weighs_what_its_volume_does() {
        let report = weigh("cube(100);", &MassOptions { print: solid("pla"), ..MassOptions::default() });
        // A liter of PLA.
        assert!(close(report.total, 1240.0, 12.4), "{}", report.total);
        assert_eq!(report.parts.len(), 1);
        assert_eq!(report.parts[0].name, "root");
        assert!(close(report.parts[0].volume, 1e6, 1e-6));
        assert!(close(report.parts[0].area, 6e4, 1e-6));
        let center = report.center.unwrap();
        assert!(center.iter().all(|c| close(*c, 50.0, 1e-9)), "{center:?}");
        // Standing on its face: 50 mm to each edge, 50 mm high: 45°.
        let stability = report.stability.unwrap();
        assert!(stability.inside && close(stability.min_tipping_deg, 45.0, 1e-9), "{stability:?}");
        assert_eq!(stability.base.len(), 4);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }

    #[test]
    fn infill_fills_what_the_walls_leave() {
        let report = weigh("cube(100);", &MassOptions::default());
        // 60 000 mm² of surface by 0.8 mm of wall: 48 000 mm³ of shell;
        // 20 % of the 952 000 mm³ left is 190 400; 238 400 mm³ of PLA at
        // 1.24 g/cm³ is 295.616 g.
        assert!(close(report.total, 295.616, 1e-6), "{}", report.total);
        // A part thinner than its walls is solid.
        let thin = weigh("cube([10, 10, 1]);", &MassOptions::default());
        assert!(close(thin.total, 100.0 * 1.24 / 1000.0, 1e-9), "{}", thin.total);
    }

    #[test]
    fn an_l_tips_first_toward_its_upright() {
        // A foot 60 × 40 × 10 (24 000 mm³, its centroid at [30, 20, 5])
        // and an upright 10 × 40 × 90 on its end (36 000 mm³, at [5, 20,
        // 55]), solid: the center of gravity is at x = (24 000 × 30 +
        // 36 000 × 5) / 60 000 = 15, z = (24 000 × 5 + 36 000 × 55) /
        // 60 000 = 35, y = 20. The base is the foot's 60 × 40: 15 mm to the
        // edge x = 0, 20 mm to y = 0 and y = 40, 45 mm to x = 60. It tips
        // first over x = 0, when it leans atan(15 / 35) = 23.199°.
        let report = weigh(
            "cube([60, 40, 10]); translate([0, 0, 10]) cube([10, 40, 90]);",
            &MassOptions { print: solid("PLA"), ..MassOptions::default() },
        );
        let center = report.center.unwrap();
        assert!(close(center[0], 15.0, 1e-9) && close(center[1], 20.0, 1e-9) && close(center[2], 35.0, 1e-9), "{center:?}");
        let stability = report.stability.unwrap();
        assert!(stability.inside);
        assert!(close(stability.min_tipping_deg, (15.0f64 / 35.0).atan().to_degrees(), 1e-9), "{stability:?}");
        assert!(close(stability.min_tipping_deg, 23.199, 1e-3));
        assert_eq!(stability.direction, [-1.0, 0.0]);
        assert!(close(stability.margin, 15.0, 1e-9) && close(stability.height, 35.0, 1e-9));
    }

    #[test]
    fn each_part_in_its_own_material() {
        let options = MassOptions {
            print: solid("PLA"),
            parts: vec![("b".into(), solid("abs"))],
            ..MassOptions::default()
        };
        let report = weigh("part(\"a\") cube(10); translate([20, 0, 0]) part(\"b\") cube(10);", &options);
        let names: Vec<&str> = report.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        assert!(close(report.parts[0].mass, 1.24, 1e-9) && close(report.parts[1].mass, 1.04, 1e-9));
        assert_eq!(report.parts[1].print.material.name, "ABS");
        // The heavier PLA pulls the center toward it: (1.24 × 5 + 1.04 × 25) / 2.28.
        assert!(close(report.center.unwrap()[0], (1.24 * 5.0 + 1.04 * 25.0) / 2.28, 1e-9));
        // A setting for a part the model does not have is refused.
        let program = c2f_eval::Program::load("part(\"a\") cube(1);", Path::new("t.scad"), &[]).unwrap();
        let options = MassOptions { parts: vec![("lid".into(), Print::default())], ..MassOptions::default() };
        let error = mass_properties(&c2f_eval::evaluate(&program).nodes, Path::new(""), &options).unwrap_err();
        assert!(error.contains("no part named `lid`: the model's parts are root, a"), "{error}");
    }

    #[test]
    fn an_extra_mass_moves_the_center_and_the_tipping() {
        // 1 240 g of cube at x = 50 and 310 g at x = 200: x = (1240 × 50 +
        // 310 × 200) / 1550 = 80; z stays 50. 20 mm to the edge x = 100,
        // 50 mm high: atan(20 / 50) = 21.801° toward +x.
        let options = MassOptions {
            print: solid("PLA"),
            extra: vec![Extra { name: "watch".into(), mass: 310.0, at: [200.0, 50.0, 50.0] }],
            ..MassOptions::default()
        };
        let report = weigh("cube(100);", &options);
        assert!(close(report.total, 1550.0, 1e-6));
        let center = report.center.unwrap();
        assert!(close(center[0], 80.0, 1e-9) && close(center[1], 50.0, 1e-9) && close(center[2], 50.0, 1e-9), "{center:?}");
        let stability = report.stability.unwrap();
        assert!(close(stability.min_tipping_deg, 21.801, 1e-3), "{stability:?}");
        assert_eq!(stability.direction, [1.0, 0.0]);
        // Far enough out, it falls.
        let options = MassOptions {
            extra: vec![Extra { name: "weight".into(), mass: 5000.0, at: [300.0, 50.0, 50.0] }],
            ..options
        };
        let report = weigh("cube(100);", &options);
        let stability = report.stability.unwrap();
        assert!(!stability.inside && stability.min_tipping_deg < 0.0, "{stability:?}");
        assert!(report.warnings.iter().any(|w| w.contains("falls over toward [1.00, 0.00]")), "{:?}", report.warnings);
    }

    #[test]
    fn components_weigh_what_their_sheet_says() {
        let options = MassOptions {
            print: solid("PLA"),
            components: vec![
                Component { name: "battery".into(), mass: Some(1240.0), bounds: ([0.0, 0.0, 100.0], [100.0, 100.0, 200.0]) },
                Component { name: "led".into(), mass: None, bounds: ([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]) },
            ],
            ..MassOptions::default()
        };
        let report = weigh("cube(100);", &options);
        assert!(close(report.total, 2480.0, 1e-6));
        assert!(close(report.center.unwrap()[2], 100.0, 1e-9));
        assert_eq!(report.warnings.len(), 1);
        assert!(report.warnings[0].starts_with("`led` has no mass on its sheet"), "{:?}", report.warnings);
    }

    #[test]
    fn materials_by_name_or_density() {
        assert_eq!(Material::named("petg").unwrap(), Material { name: "PETG".into(), density: 1.27 });
        assert_eq!(Material::named("Nylon").unwrap().density, 1.14);
        assert_eq!(Material::named("1.3").unwrap().density, 1.3);
        assert!(Material::named("wood").unwrap_err().contains("unknown material `wood`"));
        assert!(Material::named("-1").is_err());
        assert!(Print { infill: 120.0, ..Print::default() }.check().is_err());
        assert!(Print { wall: 0.0, ..Print::default() }.check().is_err());
    }

    #[test]
    fn the_hull_leaves_out_inner_points() {
        let base = hull(vec![[0.0, 0.0], [2.0, 0.0], [1.0, 0.0], [1.0, 1.0], [2.0, 2.0], [0.0, 2.0]]);
        assert_eq!(base, [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]]);
    }
}
