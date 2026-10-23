//! `c2f circuit` on the Grogu example, against the goldens in
//! `tests/circuit/`. `C2F_BLESS=1 cargo test -p c2f-cli --test circuit`
//! rewrites them.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Runs `c2f circuit <args>` in `dir`, so that the paths it prints are
/// relative and the goldens do not depend on where the repository is.
fn c2f(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_c2f"))
        .arg("circuit")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("running c2f")
}

fn grogu(args: &[&str]) -> Output {
    let output = c2f(&manifest().join("../../examples/grogu"), args);
    assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    output
}

/// Compares `actual` with the golden file `name`, or rewrites it with
/// `C2F_BLESS=1`. Numbers may differ in their last digits, as volumes do
/// from one platform to another.
fn golden(name: &str, actual: &str) {
    let path = manifest().join("tests/circuit").join(name);
    if std::env::var_os("C2F_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("{}: {err} (C2F_BLESS=1 writes it)", path.display()));
    let same = |a: &str, b: &str| match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(x), Ok(y)) => (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1.0),
        _ => a == b,
    };
    let (a, e): (Vec<&str>, Vec<&str>) = (actual.split([' ', '\n']).collect(), expected.split([' ', '\n']).collect());
    let matches = a.len() == e.len() && a.iter().zip(&e).all(|(a, e)| same(a, e));
    assert!(matches, "{name} differs from its golden (C2F_BLESS=1 rewrites it):\n{actual}");
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[test]
fn check() {
    let output = grogu(&["check", "grogu.c2fc"]);
    golden("check.txt", &(text(&output.stderr) + &text(&output.stdout)));
}

#[test]
fn wires() {
    golden("wires.txt", &text(&grogu(&["wires", "grogu.c2fc"]).stdout));
    golden("wires.csv", &text(&grogu(&["wires", "--csv", "grogu.c2fc"]).stdout));
}

#[test]
fn bom() {
    golden("bom.txt", &text(&grogu(&["bom", "grogu.c2fc"]).stdout));
    golden("bom.csv", &text(&grogu(&["bom", "grogu.c2fc", "--csv"]).stdout));
}

#[test]
fn exports() {
    let stl = grogu(&["stl", "grogu.c2fc"]);
    assert!(text(&stl.stdout).starts_with("solid Code2Form\n"));
    golden("stl.txt", &text(&stl.stderr));
    let package = grogu(&["3mf", "grogu.c2fc"]);
    assert!(package.stdout.starts_with(b"PK\x03\x04"));
    // The same parts, so the same size.
    assert_eq!(text(&package.stderr), text(&stl.stderr));
}

/// A folder of its own holding `files`, removed when dropped.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str, files: &[(&str, &str)]) -> Folder {
        let dir = std::env::temp_dir().join(format!("c2f-cli-circuit-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (file, text) in files {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        Folder(dir)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn errors_fail_with_their_line() {
    let folder = Folder::new(
        "errors",
        &[
            ("board.c2f", "// @pin A [0, 0, 0]\n// @pin B [1, 0, 0]\nassert(size > 1, \"too small\");\ncube(size);\n"),
            ("broken.c2f", "// @pin A [0, 0, 0]\ncube(;\n"),
            (
                "test.c2fc",
                "[parts.a]\npart = \"board.c2f\"\nparams = { size = 1 }\n\n[parts.b]\npart = \"board.c2f\"\nparams = { size = 2 }\n\n\
                 [parts.c]\npart = \"broken.c2f\"\n\n[[wires]]\nfrom = \"a.A\"\nto = \"b.B\"\n",
            ),
        ],
    );
    let output = c2f(&folder.0, &["check", "test.c2fc"]);
    assert!(!output.status.success());
    let stderr = text(&output.stderr);
    // The part that stops, at its name in the circuit; the one that does
    // not parse, in its own file.
    assert!(stderr.contains("error: `a` (board.c2f) stops:") && stderr.contains("too small"), "{stderr}");
    assert!(stderr.contains("--> test.c2fc:1:8"), "{stderr}");
    assert!(stderr.contains("--> broken.c2f:2:6"), "{stderr}");
    assert!(stderr.ends_with("test.c2fc: 3 parts, 1 wire: 2 errors\n"), "{stderr}");
    assert!(!c2f(&folder.0, &["stl", "test.c2fc"]).status.success());

    // A circuit that does not read fails every command.
    let output = c2f(&folder.0, &["wires", "missing.c2fc"]);
    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("cannot read the circuit"));
    assert_eq!(c2f(&folder.0, &["solder", "test.c2fc"]).status.code(), Some(2));
    assert_eq!(c2f(&folder.0, &["check", "--csv", "test.c2fc"]).status.code(), Some(2));
}

#[test]
fn wiring_errors_fail_and_warnings_do_not() {
    let board = "// @pin V5 [0, 0, 0] power_out 5V\n// @pin GND [1, 0, 0] ground\n// @pin IO [2, 0, 0] io 3.3V reserved\ncube(1);\n";
    let wire = |from: &str, to: &str| format!("\n[[wires]]\nfrom = \"{from}\"\nto = \"{to}\"\n");
    let parts = "[parts.a]\npart = \"board.c2f\"\n";
    let folder = Folder::new(
        "wiring",
        &[
            ("board.c2f", board),
            ("bad.c2fc", &(parts.to_string() + &wire("a.V5", "a.IO"))),
            ("warned.c2fc", &(parts.to_string() + &wire("a.GND", "a.IO"))),
        ],
    );
    let output = c2f(&folder.0, &["check", "bad.c2fc"]);
    assert!(!output.status.success());
    let stderr = text(&output.stderr);
    assert!(stderr.contains("error: `a.V5` supplies 5 V to `a.IO`, which takes at most 3.3 V\n --> bad.c2fc:5:8"), "{stderr}");
    assert!(stderr.contains("warning: `a.IO` is reserved by its part: better left unwired\n --> bad.c2fc:5:8"), "{stderr}");
    assert!(stderr.ends_with("bad.c2fc: 1 part, 1 wire: 2 errors, 1 warning\n"), "{stderr}");

    let output = c2f(&folder.0, &["check", "warned.c2fc"]);
    assert!(output.status.success());
    assert_eq!(text(&output.stdout), "warned.c2fc: 1 part, 1 wire: no errors, 1 warning\n");
}

#[test]
fn wire_lengths_take_a_margin() {
    let length = |args: &[&str]| -> f64 {
        let csv = text(&grogu(args).stdout);
        csv.lines().nth(1).unwrap().rsplit(',').next().unwrap().parse().unwrap()
    };
    let default = length(&["wires", "--csv", "grogu.c2fc"]);
    assert!((default - length(&["wires", "--csv", "--margin", "0", "grogu.c2fc"]) - 10.0).abs() < 0.11);
    assert!((length(&["wires", "--csv", "--margin=25", "grogu.c2fc"]) - default - 15.0).abs() < 0.11);
    let dir = manifest().join("../../examples/grogu");
    assert_eq!(c2f(&dir, &["wires", "--margin", "-1", "grogu.c2fc"]).status.code(), Some(2));
    assert_eq!(c2f(&dir, &["bom", "--margin", "5", "grogu.c2fc"]).status.code(), Some(2));
}

#[test]
fn collisions_are_warnings() {
    let block = "// @pin A [0, 0, 1]\ncube(10);\n";
    let enclosure = "translate([-20, -20, -2]) cube([60, 40, 2]);\n";
    let circuit = "enclosure = \"box.c2f\"\n\n[parts.a]\npart = \"block.c2f\"\n\n[parts.b]\npart = \"block.c2f\"\nat = [8, 0, 0]\n\n\
                   [parts.c]\npart = \"block.c2f\"\nat = [0, -15, -1]\n\n[parts.d]\npart = \"block.c2f\"\nat = [18, 0, 0]\n";
    let folder = Folder::new("collide", &[("block.c2f", block), ("box.c2f", enclosure), ("test.c2fc", circuit)]);
    let output = c2f(&folder.0, &["check", "test.c2fc"]);
    assert!(output.status.success());
    let stderr = text(&output.stderr);
    assert!(stderr.contains("WARNING: collision: `a` and `b` overlap by 200.00 mm³\n"), "{stderr}");
    assert!(stderr.contains("WARNING: collision: `c` runs into the enclosure by 100.00 mm³\n"), "{stderr}");
    // b and d only touch, as the parts that sit on the plate do.
    assert_eq!(stderr.matches("collision").count(), 2, "{stderr}");
    assert_eq!(text(&output.stdout), "test.c2fc: 4 parts, 0 wires: no errors, 2 warnings\n");
}

/// The fit tests' circuit, `tests/fit/winder.c2fc`, in a folder of its own
/// with `edit` made to its files (file, text, replacement).
fn winder(name: &str, edits: &[(&str, &str, &str)]) -> Folder {
    let dir = manifest().join("tests/fit");
    let files = ["winder.c2fc", "frame.c2f", "parts/uln2003.c2f", "parts/stepper_28byj48.c2f"].map(|file| {
        let mut text = std::fs::read_to_string(dir.join(file)).unwrap();
        for (_, from, to) in edits.iter().filter(|(f, _, _)| *f == file) {
            assert!(text.contains(from), "{file}: {from}");
            text = text.replace(from, to);
        }
        (file, text)
    });
    let files: Vec<(&str, &str)> = files.iter().map(|(file, text)| (*file, text.as_str())).collect();
    Folder::new(name, &files)
}

#[test]
fn fit_gives_each_part_its_clearance_and_finds_a_port_left_closed() {
    let output = c2f(&manifest().join("tests/fit"), &["fit", "winder.c2fc"]);
    assert!(output.status.success());
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    // Each part's least room: the XIAO's to the back wall, the driver's to
    // the left wall, the motor's boss in its 1 mm cutout.
    let row = |part: &str| stdout.lines().find(|line| line.starts_with(part)).unwrap().split_whitespace().collect::<Vec<_>>();
    assert_eq!(row("xiao")[1..3], ["3.50", "mm"], "{stdout}");
    assert_eq!(row("driver")[1..3], ["6.00", "mm"], "{stdout}");
    assert_eq!(row("motor")[1..3], ["1.00", "mm"], "{stdout}");
    // The XIAO's USB-C faces the back wall, which has no opening for it.
    assert_eq!(
        stderr,
        "WARNING: no cutout_for in front of port `xiao.usb_c`: the enclosure covers it at [26.5, 36.0, 5.6]\n",
        "{stderr}"
    );
    assert!(stdout.ends_with("winder.c2fc: 3 parts, 1 warning\n"), "{stdout}");

    // With its cutout, everything fits; a larger clearance finds the
    // motor's shaft too close to the shelf.
    let open = "    cutout_for(\"motor\", \"shaft\", clearance = 1);\n    cutout_for(\"xiao\", \"usb_c\");\n";
    let folder = winder("fit-open", &[("frame.c2f", "    cutout_for(\"motor\", \"shaft\", clearance = 1);\n", open)]);
    let output = c2f(&folder.0, &["fit", "winder.c2fc"]);
    assert_eq!(text(&output.stderr), "");
    assert!(text(&output.stdout).ends_with("winder.c2fc: 3 parts, everything fits with 0.5 mm of clearance\n"));
    let stderr = text(&c2f(&folder.0, &["fit", "--clearance", "2", "winder.c2fc"]).stderr);
    assert!(stderr.starts_with("WARNING: `motor` is 1.00 mm from the enclosure at ["), "{stderr}");
    assert!(stderr.ends_with("], under the 2 mm clearance\n"), "{stderr}");
    assert_eq!(c2f(&folder.0, &["check", "--clearance", "2", "winder.c2fc"]).status.code(), Some(2));
}

#[test]
fn fit_tells_where_a_part_touches_and_which_holes_nothing_holds() {
    // The driver against the left wall, its standoffs gone.
    let folder = winder(
        "fit-touch",
        &[
            ("winder.c2fc", "at = [-45, -5, 9]", "at = [-52, -5, 9]"),
            ("frame.c2f", "standoffs_for(\"driver\", height = 5, screw = \"M3\");\n", ""),
        ],
    );
    let output = c2f(&folder.0, &["fit", "winder.c2fc"]);
    assert!(output.status.success());
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    assert!(stdout.lines().any(|line| line.starts_with("driver  0.00 mm")), "{stdout}");
    let touch = stderr.lines().find(|line| line.contains("`driver` touches the enclosure at [-52.0, ")).unwrap_or_else(|| panic!("{stderr}"));
    assert!(touch.starts_with("WARNING: "), "{touch}");
    for hole in 1..=4 {
        let message = format!("no standoff under mount hole {hole} of `driver` (at [");
        assert!(stderr.contains(&message), "{stderr}");
    }
}

#[test]
fn check_finds_a_wire_through_a_wall() {
    // The 5 V wire led out through the left wall.
    let folder = winder("fit-wire", &[("winder.c2fc", "label = \"5 V\"\n", "label = \"5 V\"\nvia = [[-60, 0, 20]]\n")]);
    let output = c2f(&folder.0, &["check", "winder.c2fc"]);
    assert!(output.status.success());
    let stderr = text(&output.stderr);
    assert!(
        stderr.contains("WARNING: wire 1 (xiao.5V -> driver.VCC) runs into the enclosure at [-52.0, ")
            && stderr.contains("], through a face facing +x: route it around with `via`, or carve it a channel_for(1)\n"),
        "{stderr}"
    );
    assert_eq!(text(&output.stdout), "winder.c2fc: 3 parts, 4 wires: no errors, 1 warning\n");
    // The same wire fits once it runs over the wall.
    let folder = winder("fit-over", &[("winder.c2fc", "label = \"5 V\"\n", "label = \"5 V\"\nvia = [[-53.5, 0, 55]]\n")]);
    let output = c2f(&folder.0, &["check", "winder.c2fc"]);
    assert_eq!(text(&output.stdout), "winder.c2fc: 3 parts, 4 wires: no errors\n", "{}", text(&output.stderr));
    // Its length to cut follows the detour.
    let length = |dir: &Path| -> f64 {
        let csv = text(&c2f(dir, &["wires", "--csv", "winder.c2fc"]).stdout);
        csv.lines().nth(1).unwrap().rsplit(',').next().unwrap().parse().unwrap()
    };
    assert!(length(&folder.0) > length(&manifest().join("tests/fit")) + 40.0);
}
