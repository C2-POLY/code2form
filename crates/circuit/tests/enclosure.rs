//! Enclosures made for a circuit (`// @circuit robot.c2fc`): the standoffs,
//! holes and cutouts of its parts, and the Grogu plate.

use std::path::{Path, PathBuf};

type Bounds = ([f64; 3], [f64; 3]);

/// A folder of its own holding `files`, removed when dropped.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str, files: &[(&str, &str)]) -> Folder {
        let dir = std::env::temp_dir().join(format!("c2f-enclosure-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let folder = Folder(dir);
        for (file, text) in files {
            folder.write(file, text);
        }
        folder
    }

    fn write(&self, file: &str, text: &str) {
        std::fs::write(self.0.join(file), text).unwrap();
    }

    /// Evaluates `src` as the file `box.c2f` of this folder.
    fn evaluate(&self, src: &str) -> c2f_eval::Output {
        let program = c2f_eval::Program::load(src, &self.0.join("box.c2f"), &[]).unwrap();
        c2f_eval::evaluate(&program)
    }

    /// The bounds of what `src` draws, after checking it warns of nothing.
    fn bounds(&self, src: &str) -> Bounds {
        let output = self.evaluate(src);
        assert!(output.warnings.is_empty(), "{:?}", output.warnings);
        c2f_geom::render_in(&output.nodes, &self.0).bounds().unwrap()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A board with a mount hole, a rectangular port facing -y and a round one
/// facing up.
const BOARD: &str = "\
// @pin A [1, 1, 1]
// @mount_hole [2, 3] d=2
// @port usb [1, 0, 0.5] -y [4, 2]
// @port led [5, 5, 1] up d=2
cube([6, 6, 1]);
";

fn circuit(at: &str) -> String {
    format!("[parts.b]\npart = \"board.c2f\"\nat = {at}\nrotate = [0, 0, 90]\n")
}

fn close(a: Bounds, b: Bounds) -> bool {
    (0..3).all(|k| (a.0[k] - b.0[k]).abs() < 1e-6 && (a.1[k] - b.1[k]).abs() < 1e-6)
}

const HEAD: &str = "// @circuit test.c2fc\n$fn = 16;\n";

#[test]
fn standoffs_holes_and_cutouts_land_where_the_part_says() {
    let folder = Folder::new("land", &[("board.c2f", BOARD), ("test.c2fc", &circuit("[10, 20, 5]"))]);
    // The hole [2, 3, 0], turned a quarter about z, is at [-3, 2, 0], then
    // moved to [7, 22, 5]; the standoff goes down from it.
    let standoff = folder.bounds(&format!("{HEAD}standoffs_for(\"b\", height = 4, d = 3, hole = 1);"));
    assert!(close(standoff, ([5.5, 20.5, 1.0], [8.5, 23.5, 5.0])), "{standoff:?}");
    let hole = folder.bounds(&format!("{HEAD}holes_for(\"b\", depth = 3, screw = \"M3\");"));
    assert!(close(hole, ([5.3, 20.3, 2.0], [8.7, 23.7, 6.0])), "{hole:?}");
    // The port [1, 0, 0.5] faces +x once turned, at [10, 21, 5.5]: 4 wide
    // across it and 2 high, 0.5 more on each side, from 1 mm behind it.
    let cutout = folder.bounds(&format!("{HEAD}cutout_for(\"b\", \"usb\", clearance = 0.5, depth = 5);"));
    assert!(close(cutout, ([9.0, 18.5, 4.0], [15.0, 23.5, 7.0])), "{cutout:?}");
    let round = folder.bounds(&format!("{HEAD}cutout_for(\"b\", \"led\", clearance = 0);"));
    assert!(close(round, ([4.0, 24.0, 5.0], [6.0, 26.0, 16.0])), "{round:?}");
    // The part itself, where the circuit places it.
    let part = folder.bounds(&format!("{HEAD}circuit_part(\"b\");"));
    assert!(close(part, ([4.0, 20.0, 5.0], [10.0, 26.0, 6.0])), "{part:?}");
    // A standoff's screw sizes it and its hole.
    let output = folder.evaluate(&format!("{HEAD}standoffs_for(\"b\", height = 4, screw = \"M2.5\");"));
    assert!(output.csg().contains("r1 = 2.75") && output.csg().contains("r1 = 1.05"), "{}", output.csg());
}

#[test]
fn moving_a_part_moves_them_at_the_next_load() {
    let folder = Folder::new("move", &[("board.c2f", BOARD), ("test.c2fc", &circuit("[10, 20, 5]"))]);
    let src = format!("{HEAD}standoffs_for(\"b\", height = 4, d = 3);\ncutout_for(\"b\", \"usb\");\n");
    let before = folder.bounds(&src);
    folder.write("test.c2fc", &circuit("[-30, 0, 2]"));
    let after = folder.bounds(&src);
    let moved = [-40.0, -20.0, -3.0];
    assert!(!close(after, before));
    assert!(close(after, ([0, 1, 2].map(|k| before.0[k] + moved[k]), [0, 1, 2].map(|k| before.1[k] + moved[k]))));
}

#[test]
fn without_a_circuit_they_warn_and_draw_nothing() {
    let folder = Folder::new("none", &[("board.c2f", BOARD), ("test.c2fc", &circuit("[0, 0, 0]"))]);
    let output = folder.evaluate("standoffs_for(\"b\", height = 4);\ncube(1);\n");
    assert_eq!(
        output.warnings,
        ["Ignoring standoffs_for(): the file names no circuit: add a comment such as `// @circuit robot.c2fc`"]
    );
    assert_eq!(output.csg(), folder.evaluate("cube(1);\n").csg());
    // A circuit that cannot be read: a warning when loading, then at each call.
    let output = folder.evaluate("// @circuit missing.c2fc\ncutout_for(\"b\", \"usb\");\n");
    assert_eq!(output.warnings.len(), 2, "{:?}", output.warnings);
    assert!(output.warnings[0].starts_with("Can't read circuit 'missing.c2fc':"), "{}", output.warnings[0]);
    let call = &output.warnings[1];
    assert!(call.starts_with("Ignoring cutout_for(): the circuit ") && call.ends_with("could not be read"), "{call}");
    // Names the circuit does not have.
    let output = folder.evaluate(&format!(
        "{HEAD}circuit_part(\"x\");\ncutout_for(\"b\", \"hdmi\");\n\
         standoffs_for(\"b\");\nholes_for(\"b\", screw = \"M7\");\n"
    ));
    assert_eq!(
        output.warnings,
        [
            "Ignoring circuit_part(): the circuit has no part named \"x\" (b)",
            "Ignoring cutout_for(): b (board.c2f) has no port named \"hdmi\" (usb, led)",
            "Ignoring standoffs_for(): height = undef is not a positive size",
            "holes_for(): unknown screw = \"M7\", expected one of M2, M2.5, M3",
        ]
    );
    let files = [("plain.c2f", "cube(1);\n"), ("test.c2fc", "[parts.p]\npart = \"plain.c2f\"\n")];
    let folder = Folder::new("no-holes", &files);
    let output = folder.evaluate(&format!("{HEAD}standoffs_for(\"p\", height = 2);\n"));
    assert_eq!(output.warnings, ["Ignoring standoffs_for(): p (plain.c2f) declares no mount hole"]);
}

#[test]
fn a_file_that_does_not_use_them_is_unchanged() {
    let folder = Folder::new("plain", &[("board.c2f", BOARD), ("test.c2fc", &circuit("[0, 0, 0]"))]);
    let src = "difference() { cube(10); translate([1, 1, 1]) sphere(3); }\necho(len([1, 2]));\n";
    let plain = folder.evaluate(src);
    let named = folder.evaluate(&format!("// @circuit test.c2fc\n{src}"));
    assert_eq!((plain.csg(), plain.echo, plain.warnings), (named.csg(), named.echo, named.warnings));
    let program = c2f_eval::Program::load(src, &folder.0.join("box.c2f"), &[]).unwrap();
    assert!(program.circuit.is_none() && program.circuit_path.is_none());
    // A module of the file's own takes over a built-in's name.
    let own = folder.evaluate("module cutout_for(a, b) cube(2);\ncutout_for(\"b\", \"usb\");\n");
    assert!(own.warnings.is_empty() && own.csg().contains("cube(size = [2, 2, 2]"));
}

// ---- Grogu ----

fn grogu() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/grogu")
}

fn library() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library")
}

const PARTS: [&str; 8] = ["xiao", "amp", "cap", "speaker", "mic", "driver", "motor", "touch"];

/// Renders the Grogu plate's file with `extra` appended.
fn plate(extra: &str) -> c2f_geom::Rendered {
    let path = grogu().join("grogu_plate.c2f");
    let src = std::fs::read_to_string(&path).unwrap() + extra;
    let program = c2f_eval::Program::load(&src, &path, &[library()]).unwrap();
    let output = c2f_eval::evaluate(&program);
    assert_eq!(output.error, None);
    assert!(output.warnings.is_empty(), "{:?}", output.warnings);
    let rendered = c2f_geom::render_in(&output.nodes, &grogu());
    assert!(rendered.warnings.is_empty(), "{:?}", rendered.warnings);
    rendered
}

#[test]
fn the_grogu_plate_is_a_printable_solid() {
    let circuit = c2f_circuit::load(&grogu().join("grogu.c2fc"), &[library()]).unwrap();
    assert_eq!(circuit.enclosure, Some(grogu().join("grogu_plate.c2f")));
    let rendered = plate("");
    assert!(matches!(rendered.model, c2f_geom::Model::Solid(_)));
    assert!(rendered.stats.volume > 10_000.0, "{}", rendered.stats.volume);
    assert!(rendered.three_mf().is_some_and(|file| !file.is_empty()));
    // The parts are drawn `%`, seen through and left out of the print.
    assert!(!rendered.backgrounds.is_empty());
    let (min, _) = rendered.bounds().unwrap();
    assert!((min[2] + 2.0).abs() < 1e-9, "{min:?}");
}

#[test]
fn grogu_parts_sit_apart_on_the_plate() {
    let bounds: Vec<Bounds> = PARTS
        .iter()
        .map(|part| plate(&format!("\n!circuit_part(\"{part}\");\n")).bounds().unwrap())
        .collect();
    for (i, a) in bounds.iter().enumerate() {
        // On the plate's top, or above it.
        assert!(a.0[2] >= -1e-9, "{} goes through the plate: {a:?}", PARTS[i]);
        for (j, b) in bounds.iter().enumerate().skip(i + 1) {
            let apart = (0..3).any(|k| a.1[k] <= b.0[k] || b.1[k] <= a.0[k]);
            assert!(apart, "{} {a:?} and {} {b:?} overlap", PARTS[i], PARTS[j]);
        }
    }
    // Nothing of a part is inside the plate, its fences or its walls.
    for part in PARTS {
        let shared = plate(&format!("\n!intersection() {{ grogu_plate(); circuit_part(\"{part}\"); }}\n"));
        assert!(shared.stats.volume < 1e-6, "{part} is {} mm³ into the plate", shared.stats.volume);
    }
}

#[test]
fn a_channel_carves_a_passage_the_wire_no_longer_touches() {
    // Two boards either side of a wall, a wire between them over its top.
    let circuit = "[parts.a]\npart = \"board.c2f\"\n[parts.b]\npart = \"board.c2f\"\nat = [30, 0, 0]\n\n\
                   [[wires]]\nfrom = \"a.A\"\nto = \"b.A\"\nlabel = \"signal\"\n";
    let folder = Folder::new("channel", &[("board.c2f", BOARD), ("test.c2fc", circuit)]);
    let loaded = c2f_circuit::load(&folder.0.join("test.c2fc"), &[]).unwrap();
    let wall = "translate([13, -10, 0]) cube([4, 20, 20]);";
    let enclosure = |src: &str| {
        let output = folder.evaluate(&format!("{HEAD}{src}"));
        assert!(output.warnings.is_empty(), "{:?}", output.warnings);
        c2f_geom::render_in(&output.nodes, &folder.0).model
    };
    // The wall stops it: where it first meets it, on the face towards a.
    let crossings = c2f_geom::crossings(&loaded, &enclosure(wall));
    assert_eq!(crossings.len(), 1, "{crossings:?}");
    assert!((crossings[0].at[0] - 13.0).abs() < 1e-6 && crossings[0].facing == [-1.0, 0.0, 0.0], "{crossings:?}");
    assert!(crossings[0].message(&loaded).starts_with("wire 1 (a.A -> b.A) runs into the enclosure at [13.0, "));
    // A channel along it, by number or by label, lets it through, clear of
    // the wire's own radius (0.5 mm) all along.
    for channel in ["channel_for(1)", "channel_for(\"signal\", d = 2)"] {
        let carved = enclosure(&format!("difference() {{ {wall} {channel}; }}"));
        assert!(c2f_geom::crossings(&loaded, &carved).is_empty(), "{channel}");
        let c2f_geom::Model::Solid(carved) = carved else { panic!("{channel}: not a solid") };
        let path = c2f_circuit::route::path(&loaded, 0);
        for pair in path.windows(2) {
            for side in [[0.0, 0.0, 0.5], [0.0, 0.0, -0.5], [0.0, 0.5, 0.0], [0.0, -0.5, 0.0]] {
                let v = |p: [f64; 3]| manifold_rust::linalg::Vec3::new(p[0] + side[0], p[1] + side[1], p[2] + side[2]);
                assert!(carved.ray_cast(v(pair[0]), v(pair[1])).is_empty(), "{channel}: the wire's side {side:?} touches");
            }
        }
    }
    // What is not a wire of the circuit.
    let output = folder.evaluate(&format!("{HEAD}channel_for(2);\nchannel_for(\"power\");\nchannel_for(1, d = 0);\n"));
    assert_eq!(
        output.warnings,
        [
            "Ignoring channel_for(): wire = 2 is not a wire's number (1 to 1) or label",
            "Ignoring channel_for(): no wire has the label \"power\"",
            "Ignoring channel_for(): d = 0 is not a positive size",
        ]
    );
}
