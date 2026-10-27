//! The measurements and pictures of `docs/research/conges-chanfreins.md`:
//! three ways to round or chamfer edges, timed on the same models.
//!
//! ```sh
//! cargo run --release -p c2f-geom --example fillet_research
//! ```
//!
//! Prints a Markdown table row per experiment and writes the pictures to
//! `docs/research/img/`. Times are the median of five runs (one for the
//! slow ones), evaluation and rendering included.

use std::collections::HashMap;
use std::f64::consts::PI;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use c2f_geom::{Model, Rendered};
use manifold_rust::manifold::Manifold;

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn render(src: &str) -> Rendered {
    let program = c2f_eval::Program::load(src, &repository().join("research.scad"), &c2f_eval::default_library_paths())
        .unwrap_or_else(|e| panic!("{}", e.render()));
    let output = c2f_eval::evaluate(&program);
    assert!(output.error.is_none(), "{:?}", output.error);
    c2f_geom::render(&output.nodes)
}

fn timed<T>(runs: usize, mut f: impl FnMut() -> T) -> (Duration, T) {
    let mut times = Vec::new();
    let mut last = None;
    for _ in 0..runs {
        let start = Instant::now();
        last = Some(f());
        times.push(start.elapsed());
    }
    times.sort();
    (times[times.len() / 2], last.unwrap())
}

fn solid(rendered: &Rendered) -> Manifold {
    match &rendered.model {
        Model::Solid(m) => m.clone(),
        _ => panic!("not a solid"),
    }
}

fn picture(rendered: &Rendered, name: &str) {
    // The app's diagonal view, but from behind for the corner (+x, +y).
    let yaw = if name.contains("corner") { 30.0 } else { -60.0 };
    // The six variants together need more room.
    let (width, height) = if name.contains("variants") { (640, 380) } else { (360, 260) };
    let png = rendered.png(yaw, 28.0, width, height).expect("a picture");
    let dir = repository().join("docs/research/img");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{name}.png")), png).unwrap();
}

/// A solid made outside the language, pictured by importing it as STL.
fn picture_of(manifold: &Manifold, name: &str) {
    let mesh = manifold.get_mesh_gl64(-1);
    let stride = mesh.num_prop as usize;
    let v = |i: u64| {
        let i = i as usize * stride;
        [mesh.vert_properties[i], mesh.vert_properties[i + 1], mesh.vert_properties[i + 2]]
    };
    let mut stl = String::from("solid r\n");
    for t in mesh.tri_verts.chunks(3) {
        stl += "facet normal 0 0 0\nouter loop\n";
        for &i in t {
            let p = v(i);
            stl += &format!("vertex {} {} {}\n", p[0], p[1], p[2]);
        }
        stl += "endloop\nendfacet\n";
    }
    stl += "endsolid r\n";
    let path = std::env::temp_dir().join(format!("c2f-fillet-{name}.stl"));
    std::fs::write(&path, stl).unwrap();
    picture(&render(&format!("import(\"{}\");", path.display())), name);
    let _ = std::fs::remove_file(path);
}

fn row(label: &str, time: Duration, m: &Manifold, expected: Option<f64>) {
    let b = m.bounding_box();
    let size = [b.max.x - b.min.x, b.max.y - b.min.y, b.max.z - b.min.z];
    let error = expected.map_or("—".to_string(), |e| format!("{:+.2} %", (m.volume() - e) / e * 100.0));
    println!(
        "| {label} | {:.1} ms | {} | {:.1} | {error} | {:.2} × {:.2} × {:.2} |",
        time.as_secs_f64() * 1000.0,
        m.num_tri(),
        m.volume(),
        size[0],
        size[1],
        size[2]
    );
}

/// Runs a program: its time, its solid, and a picture named `name`.
fn program(label: &str, name: Option<&str>, runs: usize, src: &str, expected: Option<f64>) -> Manifold {
    let (time, rendered) = timed(runs, || render(src));
    if let Some(name) = name {
        picture(&rendered, name);
    }
    let m = solid(&rendered);
    row(label, time, &m, expected);
    m
}

/// The edges whose faces turn by more than `degrees`, counted after
/// welding the triangles' corners by position.
fn sharp_edges(m: &Manifold, degrees: f64) -> usize {
    let mesh = m.get_mesh_gl64(-1);
    let stride = mesh.num_prop as usize;
    let mut ids: HashMap<[i64; 3], usize> = HashMap::new();
    let mut points = Vec::new();
    let mut id = |i: u64| {
        let i = i as usize * stride;
        let p = [mesh.vert_properties[i], mesh.vert_properties[i + 1], mesh.vert_properties[i + 2]];
        let key = p.map(|x| (x * 1e6).round() as i64);
        *ids.entry(key).or_insert_with(|| {
            points.push(p);
            points.len() - 1
        })
    };
    let tris: Vec<[usize; 3]> = mesh.tri_verts.chunks(3).map(|t| [id(t[0]), id(t[1]), id(t[2])]).collect();
    let normal = |t: &[usize; 3]| {
        let [a, b, c] = t.map(|i| points[i]);
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        n.map(|x| x / l)
    };
    let mut faces: HashMap<(usize, usize), Vec<[f64; 3]>> = HashMap::new();
    for t in &tris {
        let n = normal(t);
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            faces.entry((a.min(b), a.max(b))).or_default().push(n);
        }
    }
    let limit = degrees.to_radians().cos();
    faces.values().filter(|n| n.len() == 2 && n[0][0] * n[1][0] + n[0][1] * n[1][1] + n[0][2] * n[1][2] < limit).count()
}

fn main() {
    let (x, y, z, r) = (30.0f64, 20.0f64, 10.0f64, 2.0f64);
    let (a, b, c) = (x - 2.0 * r, y - 2.0 * r, z - 2.0 * r);
    let steiner = a * b * c + 2.0 * r * (a * b + b * c + a * c) + PI * r * r * (a + b + c) + 4.0 / 3.0 * PI * r.powi(3);
    let fine = "$fn = 32;\n";

    println!("## Box 30 × 20 × 10, every edge rounded r = 2 (expected {steiner:.1} mm³)\n");
    println!("| Method | Time | Triangles | Volume | vs analytic | Box |\n|---|---|---|---|---|---|");
    program("`cube()` as it is", None, 5, "cube([30, 20, 10]);", None);
    program(
        "1. `rounded_cube()` (library)",
        Some("box-option1"),
        5,
        &format!("{fine}use <shapes/rounded.c2f>\nrounded_cube([30, 20, 10], fillet = 2);"),
        Some(steiner),
    );
    program(
        "1. `hull()` of `sphere()` written out",
        None,
        5,
        &format!("{fine}hull() for (x = [2, 28], y = [2, 18], z = [2, 8]) translate([x, y, z]) sphere(2);"),
        Some(steiner),
    );
    program(
        "2. `minkowski()` shrunk box + sphere",
        Some("box-minkowski"),
        5,
        &format!("{fine}translate([2, 2, 2]) minkowski() {{ cube([26, 16, 6]); sphere(2); }}"),
        Some(steiner),
    );
    let ball = solid(&render(&format!("{fine}sphere(2);")));
    let cube = solid(&render("cube([30, 20, 10]);"));
    let (time, opened) = timed(5, || cube.minkowski_difference(&ball).minkowski_sum(&ball));
    row("2. Opening (Manifold: erode then dilate)", time, &opened, Some(steiner));
    let (time, smooth) = timed(5, || cube.smooth_out(100.0, 0.0).refine(8));
    picture_of(&smooth, "box-smoothout");
    row("3. `smooth_out` + `refine(8)` (Manifold)", time, &smooth, Some(steiner));

    // A straight cylinder, its top rim rounded.
    let (h, big) = (20.0f64, 10.0f64);
    let k = (10.0 - 3.0 * PI) / (12.0 - 3.0 * PI);
    let rim = 2.0 * PI * (big - k * r) * (1.0 - PI / 4.0) * r * r;
    println!("\n## Cylinder h = 20, r = 10, top rim rounded r = 2 (expected {:.1} mm³)\n", PI * big * big * h - rim);
    println!("| Method | Time | Triangles | Volume | vs analytic | Box |\n|---|---|---|---|---|---|");
    program(
        "1. `rounded_cylinder()` (library)",
        Some("cylinder-option1"),
        5,
        &format!("{fine}use <shapes/rounded.c2f>\nrounded_cylinder(h = 20, r = 10, fillet = 2, rims = \"top\");"),
        Some(PI * big * big * h - rim),
    );
    program(
        "2. `minkowski()` (rounds both rims)",
        Some("cylinder-minkowski"),
        5,
        &format!("{fine}translate([0, 0, 2]) minkowski() {{ cylinder(h = 16, r = 8); sphere(2); }}"),
        Some(PI * big * big * h - 2.0 * rim),
    );

    // A bracket: not convex, with a hole. Rounding r = 1.
    let bracket = "difference() {
    union() { cube([40, 20, 4]); cube([4, 20, 30]); }
    translate([25, 10, -1]) cylinder(h = 6, r = 3);
}";
    println!("\n## Bracket (L with a hole), r = 1\n");
    println!("| Method | Time | Triangles | Volume | vs analytic | Box |\n|---|---|---|---|---|---|");
    let plain = program("Bracket as it is", Some("bracket-plain"), 5, &format!("{fine}{bracket}"), None);
    program(
        "2. `minkowski()` with a sphere r = 1",
        Some("bracket-minkowski"),
        1,
        &format!("{fine}minkowski() {{ {bracket} sphere(1); }}"),
        None,
    );
    let ball = solid(&render(&format!("{fine}sphere(1);")));
    let (time, opened) = timed(1, || plain.minkowski_difference(&ball).minkowski_sum(&ball));
    picture_of(&opened, "bracket-opening");
    row("2. Opening (Manifold: erode then dilate)", time, &opened, None);
    let (time, smooth) = timed(1, || plain.smooth_out(60.0, 0.0).refine(4));
    picture_of(&smooth, "bracket-smoothout");
    row("3. `smooth_out(60°)` + `refine(4)` (Manifold)", time, &smooth, None);
    // One straight edge between two planes, picked on the mesh and rounded
    // by subtracting a cutter: the end of the base (x = 40, z = 4), r = 2,
    // and the inside corner of the L filled, r = 2.
    let cutter = "
module edge_fillet(length, r) {
    // Along +y from the origin; the corner of the material at the origin,
    // the material towards -x and -z.
    translate([-r, -0.01, -r]) difference() {
        cube([r + 0.01, length + 0.02, r + 0.01]);
        translate([0, -0.01, 0]) rotate([-90, 0, 0]) cylinder(h = length + 0.04, r = r);
    }
}
module inside_fillet(length, r) {
    // The inside corner at the origin, the material towards -x and -z.
    difference() {
        cube([r, length, r]);
        translate([r, -0.01, r]) rotate([-90, 0, 0]) cylinder(h = length + 0.02, r = r);
    }
}";
    let removed = (1.0 - PI / 4.0) * 4.0 * 20.0;
    program(
        "3'. Picked edge → cutter in the code (outside and inside)",
        Some("bracket-cutter"),
        5,
        &format!(
            "{fine}{cutter}\nunion() {{ difference() {{ {bracket} translate([40, 0, 4]) edge_fillet(20, 2); }} translate([4, 0, 4]) inside_fillet(20, 2); }}"
        ),
        Some(plain.volume()),
    );
    println!("(expected: as it is, {removed:.2} mm³ taken at the end and as much added inside)");
    // The limit: three rounded edges meeting at a corner want a patch of
    // sphere there; the cutters leave a point.
    program(
        "3'. Three cutters at a corner",
        Some("box-cutter-corner"),
        5,
        &format!(
            "{fine}{cutter}\ndifference() {{ cube([30, 20, 10]); translate([30, 0, 10]) edge_fillet(20, 2); translate([30, 20, 10]) rotate([0, 0, 90]) edge_fillet(30, 2); translate([30, 20, 10]) rotate([-90, 0, 0]) edge_fillet(10, 2); }}"
        ),
        None,
    );

    // Every variant of option 1, for the note.
    picture(
        &render(&format!(
            "{fine}use <shapes/rounded.c2f>
rounded_cube([30, 20, 10], fillet = 3, edges = \"vertical\");
translate([40, 0, 0]) rounded_cube([30, 20, 10], fillet = 3, edges = \"top\");
translate([80, 0, 0]) chamfered_cube([30, 20, 10], chamfer = 2);
translate([15, 45, 0]) rounded_cylinder(h = 20, r1 = 12, r2 = 8, fillet = 3);
translate([55, 45, 0]) chamfered_cylinder(h = 20, r = 10, chamfer = 2, rims = \"top\");
translate([95, 45, 0]) chamfered_cube([20, 20, 20], chamfer = 3, edges = \"bottom\", center = true);"
        )),
        "option1-variants",
    );

    println!("\n## Sharp edges found by the dihedral angle\n");
    println!("| Model | > 30° | > 60° |\n|---|---|---|");
    for (label, src) in [
        ("Bracket, hole `$fn = 32`", format!("$fn = 32;\n{bracket}")),
        ("Bracket, hole by default (`$fs = 2`)", bracket.to_string()),
        ("Box rounded r = 2 by option 1", format!("{fine}use <shapes/rounded.c2f>\nrounded_cube([30, 20, 10], fillet = 2);")),
    ] {
        let m = solid(&render(&src));
        println!("| {label} | {} | {} |", sharp_edges(&m, 30.0), sharp_edges(&m, 60.0));
    }
}
