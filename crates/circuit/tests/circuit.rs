//! Reading circuit files: the Grogu example, pin positions and errors.

use std::path::{Path, PathBuf};

use c2f_circuit::{Circuit, End, Error};

fn grogu_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/grogu/grogu.c2fc")
}

/// The repository's electronics library, where Grogu's parts are.
fn library() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library")
}

fn grogu() -> Circuit {
    c2f_circuit::load(&grogu_path(), &[library()]).unwrap_or_else(|errors| {
        panic!("{}", errors.iter().map(Error::render).collect::<String>())
    })
}

/// A folder of its own holding `files`, removed when dropped.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str, files: &[(&str, &str)]) -> Folder {
        let dir = std::env::temp_dir().join(format!("c2f-circuit-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (file, text) in files {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        Folder(dir)
    }

    fn parse(&self, circuit: &str) -> Result<Circuit, Vec<Error>> {
        c2f_circuit::parse(circuit, &self.0.join("test.c2fc"), &[])
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const BOARD: &str = "\
// @pin A [1, 0, 0] +x io 3.3V
// @pin B [0, 1, 0] up
cube([2, 2, 1]);
";

/// Each error as `file:line: message`.
fn errors(result: Result<Circuit, Vec<Error>>) -> Vec<String> {
    let Err(errors) = result else { panic!("the circuit was read without error") };
    errors
        .iter()
        .map(|e| {
            let line = e.src[..e.diagnostic.span.start as usize].matches('\n').count() + 1;
            format!("{}:{line}: {}", e.path.file_name().unwrap().to_string_lossy(), e.diagnostic.message)
        })
        .collect()
}

fn close(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
}

#[test]
fn grogu_reads_in_file_order() {
    let circuit = grogu();
    let names: Vec<&str> = circuit.parts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["xiao", "amp", "cap", "speaker", "mic", "driver", "motor", "touch"]);
    assert_eq!(circuit.wires.len(), 23);
    let first = &circuit.wires[0];
    assert_eq!((circuit.end_name(first.from), circuit.end_name(first.to)), ("xiao.5V".into(), "amp.VIN".into()));
    assert_eq!((first.color.as_deref(), first.label.as_deref()), (Some("red"), Some("5 V")));
    let last = circuit.wires.last().unwrap();
    assert_eq!(circuit.end_name(last.to), "touch.PAD");
    assert_eq!(circuit.enclosure, None);
}

#[test]
fn grogu_parts_render_with_their_params() {
    let circuit = grogu();
    for part in &circuit.parts {
        let program = c2f_eval::Program::load(&part.source_with_params(), &part.path, &[])
            .unwrap_or_else(|err| panic!("{}", err.render()));
        let output = c2f_eval::evaluate(&program);
        assert_eq!(output.error, None, "{}", part.name);
        assert!(output.warnings.is_empty(), "{}: {:?}", part.name, output.warnings);
    }
    // The amplifier's header shows only with `headers = true`.
    let amp = circuit.parts.iter().find(|p| p.name == "amp").unwrap();
    assert_eq!(amp.defines(), ["headers=true"]);
    let csg = |src: &str| {
        let program = c2f_eval::Program::load(src, &amp.path, &[]).unwrap();
        c2f_eval::evaluate(&program).csg()
    };
    assert_ne!(csg(&amp.source_with_params()), csg(&amp.source));
}

#[test]
fn pins_follow_their_part() {
    let circuit = grogu();
    let mic = circuit.parts.iter().position(|p| p.name == "mic").unwrap();
    let sd = circuit.parts[mic].pin("SD").unwrap();
    let placed = circuit.place(End { part: mic, pin: sd });
    // [-2.54, -5, 1], turned half a turn about z, then moved to [35, 12, 0].
    assert!(close(placed.position, [37.54, 17.0, 1.0]), "{:?}", placed.position);
    assert!(close(placed.direction, [0.0, 0.0, 1.0]));
}

#[test]
fn rotations_turn_as_rotate_does() {
    let folder = Folder::new("rotate", &[("board.c2f", BOARD)]);
    let place = |rotate: &str| {
        let circuit = folder
            .parse(&format!("[parts.b]\npart = \"board.c2f\"\nat = [10, 20, 30]\nrotate = {rotate}\n"))
            .unwrap();
        let a = circuit.place(End { part: 0, pin: 0 });
        let b = circuit.place(End { part: 0, pin: 1 });
        (a.position, a.direction, b.position)
    };
    let (a, a_dir, b) = place("[0, 0, 90]");
    assert!(close(a, [10.0, 21.0, 30.0]) && close(a_dir, [0.0, 1.0, 0.0]) && close(b, [9.0, 20.0, 30.0]));
    // About y, x goes to -z.
    let (a, a_dir, _) = place("[0, 90, 0]");
    assert!(close(a, [10.0, 20.0, 29.0]) && close(a_dir, [0.0, 0.0, -1.0]));
    // About x first, then z.
    let (a, _, b) = place("[90, 0, 90]");
    assert!(close(a, [10.0, 21.0, 30.0]) && close(b, [10.0, 20.0, 31.0]), "{a:?} {b:?}");
    let circuit = folder.parse("[parts.b]\npart = \"board.c2f\"\nat = [10, 20, 30]\nrotate = [90, 0, 90]\n").unwrap();
    let m = circuit.parts[0].matrix();
    let pin = circuit.parts[0].annotations.pins[1].position;
    let by_matrix: Vec<f64> = (0..3).map(|r| (0..3).map(|c| m[r][c] * pin[c]).sum::<f64>() + m[r][3]).collect();
    assert!(close([by_matrix[0], by_matrix[1], by_matrix[2]], circuit.parts[0].place(pin)));
    assert_eq!(m[3], [0.0, 0.0, 0.0, 1.0]);
    let (a, _, _) = place("[0, 0, 45]");
    let h = 0.5f64.sqrt();
    assert!(close(a, [10.0 + h, 20.0 + h, 30.0]));
}

#[test]
fn params_become_defines_in_their_order() {
    let folder = Folder::new("params", &[("board.c2f", BOARD)]);
    let circuit = folder
        .parse("[parts.b]\npart = \"board.c2f\"\nparams = { z = 1, a = 2.5, name = \"x\\\"y\", on = false, v = [1, [2, \"t\"]] }\n")
        .unwrap();
    assert_eq!(circuit.parts[0].defines(), ["z=1", "a=2.5", "name=\"x\\\"y\"", "on=false", "v=[1, [2, \"t\"]]"]);
    assert!(circuit.parts[0].source_with_params().ends_with("\n\x03\nz=1;\na=2.5;\nname=\"x\\\"y\";\non=false;\nv=[1, [2, \"t\"]];\n"));
    // Numbers keep every digit.
    let circuit = folder.parse("[parts.b]\npart = \"board.c2f\"\nparams = { q = 1234567, r = 0.1234567, s = [-1e-7] }\n").unwrap();
    assert_eq!(circuit.parts[0].defines(), ["q=1234567", "r=0.1234567", "s=[-0.0000001]"]);
}

#[test]
fn a_part_file_is_read_once_for_all_its_instances() {
    let folder = Folder::new("twice", &[("lib/board.c2f", BOARD)]);
    let circuit = folder
        .parse("[parts.a]\npart = \"lib/board.c2f\"\n[parts.b]\npart = \"lib/board.c2f\"\nat = [5, 0, 0]\n[[wires]]\nfrom = \"a.A\"\nto = \"b.A\"\n")
        .unwrap();
    assert_eq!(circuit.parts[1].path, folder.0.join("lib/board.c2f"));
    assert_eq!(circuit.wires[0].to, End { part: 1, pin: 0 });
}

#[test]
fn parts_are_found_in_the_library_folders() {
    let library = Folder::new("library", &[("electronics/board.c2f", BOARD)]);
    let folder = Folder::new("uses-library", &[]);
    std::fs::create_dir_all(&folder.0).unwrap();
    let circuit = c2f_circuit::parse(
        "[parts.b]\npart = \"electronics/board.c2f\"\n",
        &folder.0.join("test.c2fc"),
        std::slice::from_ref(&library.0),
    )
    .unwrap();
    assert_eq!(circuit.parts[0].path, library.0.join("electronics/board.c2f"));
}

#[test]
fn errors_point_at_their_line() {
    let folder = Folder::new("errors", &[("board.c2f", BOARD), ("bad.c2f", "cube(1);\n// @pin X [0, 0]\n")]);
    let circuit = "\
version = 1
[parts.a]
part = \"board.c2f\"
[parts.lost]
part = \"missing.c2f\"
[parts.bad]
part = \"bad.c2f\"
at = [1, 2]
params = { ok = 1, \"not-a-name\" = 2, t = { x = 1 } }

[[wires]]
from = \"a.A\"
to = \"a.Z\"
[[wires]]
from = \"a.A\"
to = \"a.A\"
[[wires]]
from = \"a.B\"
to = \"nobody.A\"
[[wires]]
from = \"lost.A\"
to = \"a\"
";
    assert_eq!(
        errors(folder.parse(circuit)),
        [
            "test.c2fc:5: cannot find `missing.c2f` next to the circuit or in the library folders",
            "test.c2fc:8: `at` is three numbers, [x, y, z]",
            "test.c2fc:9: `not-a-name` is not a variable name",
            "test.c2fc:9: a parameter is a boolean, a finite number, a string or an array of them",
            "bad.c2f:2: expected the pin's position, [x, y, z]",
            "test.c2fc:13: `a` (board.c2f) has no pin named `Z`",
            "test.c2fc:16: a wire cannot go from `a.A` to itself",
            "test.c2fc:19: there is no part named `nobody`",
            "test.c2fc:22: expected `part.pin`, as in `xiao.D3`, found `a`",
        ]
    );
}

#[test]
fn the_file_itself_must_be_well_formed() {
    let folder = Folder::new("format", &[("board.c2f", BOARD)]);
    assert_eq!(errors(folder.parse("version = 2\n")), ["test.c2fc:1: unknown circuit version 2, expected 1"]);
    assert_eq!(errors(folder.parse("[parts.a]\npart = \"board.c2f\"\nsize = 3\n"))[0].split_once(": ").unwrap().0, "test.c2fc:3");
    assert_eq!(errors(folder.parse("[parts.a]\npart = \"board.c2f\"\n[[wires]]\nfrom = \"a.A\"\n")).len(), 1);
    assert_eq!(errors(folder.parse("\n\nparts = [\n"))[0].split_once(": ").unwrap().0, "test.c2fc:3");
    let dotted = "[parts.\"a.b\"]\npart = \"board.c2f\"\n[[wires]]\nfrom = \"a.b.A\"\nto = \"a.b.B\"\n";
    assert_eq!(errors(folder.parse(dotted)), ["test.c2fc:1: a part name cannot contain `.`"]);
    assert_eq!(errors(folder.parse("[parts.\"\"]\npart = \"board.c2f\"\n")), ["test.c2fc:1: a part needs a name"]);
    let infinite = "[parts.a]\npart = \"board.c2f\"\nat = [0, inf, 0]\nparams = { x = nan }\n";
    assert_eq!(
        errors(folder.parse(infinite)),
        [
            "test.c2fc:3: `at` must be finite numbers",
            "test.c2fc:4: a parameter is a boolean, a finite number, a string or an array of them"
        ]
    );
}

#[test]
fn the_schema_lists_the_fields_the_reader_takes() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("circuit.schema.json");
    let schema: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let keys = |value: &serde_json::Value| {
        let mut keys: Vec<String> = value["properties"].as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    };
    assert_eq!(keys(&schema), ["enclosure", "parts", "version", "wires"]);
    assert_eq!(keys(&schema["$defs"]["part"]), ["at", "color", "params", "part", "rotate"]);
    assert_eq!(keys(&schema["$defs"]["wire"]), ["color", "from", "label", "to"]);
}

#[test]
fn wires_run_from_pin_to_pin_and_stay_apart() {
    let circuit = grogu();
    let paths = c2f_circuit::route::paths(&circuit);
    for (wire, path) in circuit.wires.iter().zip(&paths) {
        assert!(close(path[0], circuit.place(wire.from).position));
        assert!(close(*path.last().unwrap(), circuit.place(wire.to).position));
        // Each wire first leaves its pin the way the pin points.
        let start = circuit.place(wire.from);
        let step = [0, 1, 2].map(|k| path[1][k] - path[0][k]);
        assert!(step.iter().zip(start.direction).map(|(s, d)| s * d).sum::<f64>() > 0.0);
        assert!(c2f_circuit::route::length_of(path) >= 0.0);
    }
    // xiao.GND starts two wires: their middles are apart.
    let gnd: Vec<usize> =
        (0..circuit.wires.len()).filter(|&i| circuit.end_name(circuit.wires[i].from) == "xiao.GND").collect();
    assert_eq!(gnd.len(), 2);
    let (a, b) = (paths[gnd[0]][12], paths[gnd[1]][12]);
    let apart = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
    assert!(apart > 1.0, "{apart}");
}
