//! `c2f mass` on designs and circuits: masses, the center of gravity and
//! the tipping angle, checked against sums done by hand.

use std::path::Path;
use std::process::{Command, Output};

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Runs `c2f mass <args>` in `dir`.
fn c2f(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_c2f")).arg("mass").args(args).current_dir(dir).output().expect("running c2f")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// A design written in a folder of its own.
fn design(name: &str, src: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("c2f-mass-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("model.c2f"), src).unwrap();
    dir
}

#[test]
fn a_cube_of_solid_pla_weighs_a_kilo_and_a_quarter() {
    let dir = design("cube", "size = 100; cube(size);");
    let output = c2f(&dir, &["--infill", "100", "model.c2f"]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    // A liter at 1.24 g/cm³; its center at its middle; 50 mm to each
    // edge, 50 mm high: 45°.
    assert!(stdout.contains("root  PLA        100 %  1000000 mm³  1240.0 g  [50.0, 50.0, 50.0]"), "{stdout}");
    assert!(stdout.contains("total: 1240.0 g, center of gravity at [50.0, 50.0, 50.0]"), "{stdout}");
    assert!(stdout.contains("when leaned 45.0°"), "{stdout}");
    // At 20 % in PETG, half the size: 15 000 mm² by 0.8 mm is 12 000 mm³
    // of walls, and 20 % of the 113 000 left is 22 600: 34 600 mm³ at
    // 1.27 g/cm³ is 43.942 g.
    let output = c2f(&dir, &["--material", "petg", "-D", "size=50", "model.c2f"]);
    assert!(text(&output.stdout).contains("total: 43.9 g"), "{}", text(&output.stdout));
    // Settings that cannot be printed are refused.
    let output = c2f(&dir, &["--infill", "150", "model.c2f"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(text(&output.stderr).contains("infill is a percentage from 0 to 100"), "{}", text(&output.stderr));
    let output = c2f(&dir, &["--material", "wood", "model.c2f"]);
    assert!(text(&output.stderr).contains("unknown material `wood`"), "{}", text(&output.stderr));
}

#[test]
fn an_l_tips_over_its_short_side() {
    // A foot 60 × 40 × 10 and an upright 10 × 40 × 90 at x = 0: the
    // center of gravity is at x = (24 000 × 30 + 36 000 × 5) / 60 000 =
    // 15, 35 mm high (24 000 × 5 + 36 000 × 55) / 60 000; 15 mm from the
    // edge x = 0, it tips when leaned atan(15 / 35) = 23.2°, toward -x.
    let dir = design("l", "cube([60, 40, 10]); translate([0, 0, 10]) cube([10, 40, 90]);");
    let output = c2f(&dir, &["--infill", "100", "model.c2f"]);
    let stdout = text(&output.stdout);
    assert!(stdout.contains("center of gravity at [15.0, 20.0, 35.0]"), "{stdout}");
    assert!(stdout.contains("tips first toward [-1.00, 0.00] when leaned 23.2° (15.0 mm from the edge, 35.0 mm high)"), "{stdout}");
}

#[test]
fn the_nightlight_weighs_its_enclosure_and_warns_of_parts_without_a_mass() {
    let dir = manifest().join("../../examples/nightlight");
    let design = c2f(&dir, &["nightlight.c2f"]);
    assert!(design.status.success(), "{}", text(&design.stderr));
    let circuit = c2f(&dir, &["nightlight.c2fc"]);
    assert!(circuit.status.success(), "{}", text(&circuit.stderr));
    let (stdout, stderr) = (text(&circuit.stdout), text(&circuit.stderr));
    // Its sheets give no mass yet.
    for part in ["xiao", "ring", "touch"] {
        assert!(stderr.contains(&format!("WARNING: `{part}` has no mass on its sheet")), "{stderr}");
        assert!(stdout.lines().any(|line| line.starts_with(part) && line.contains(" ? ")), "{stdout}");
    }
    // The enclosure's three pieces weigh what the design does alone (the
    // columns are as wide as the longest name).
    let pieces = |stdout: &str| -> Vec<Vec<String>> {
        stdout
            .lines()
            .filter(|line| ["Base", "Top", "Dome"].iter().any(|p| line.starts_with(p)))
            .map(|line| line.split_whitespace().map(String::from).collect())
            .collect()
    };
    assert_eq!(pieces(&stdout).len(), 3, "{stdout}");
    assert_eq!(pieces(&stdout), pieces(&text(&design.stdout)));
    let total = |stdout: &str| stdout.lines().find(|line| line.starts_with("total:")).unwrap().to_string();
    assert_eq!(total(&stdout), total(&text(&design.stdout)));
    assert!(stdout.contains("stands on"), "{stdout}");
}

#[test]
fn a_circuit_part_weighs_what_its_sheet_says() {
    let output = c2f(&manifest().join("tests/mass"), &["--infill", "100", "ballast.c2fc"]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    // The plate: 8 000 mm³ of PLA, 9.92 g at [20, 20, 2.5]; the cube's
    // sheet says 10 g, at its box's center [35, 35, 10]. Together 19.92 g
    // at x = y = (9.92 × 20 + 10 × 35) / 19.92 = 27.53 and z = (9.92 ×
    // 2.5 + 10 × 10) / 19.92 = 6.27.
    assert!(stdout.contains("weight  (its sheet)                    10.0 g  [35.0, 35.0, 10.0]"), "{stdout}");
    assert!(stdout.contains("total: 19.9 g, center of gravity at [27.5, 27.5, 6.3]"), "{stdout}");
    assert!(text(&output.stderr).is_empty(), "{}", text(&output.stderr));
    // -D sets a design's variables, not a circuit's.
    let output = c2f(&manifest().join("tests/mass"), &["-D", "x=1", "ballast.c2fc"]);
    assert_eq!(output.status.code(), Some(2));
}
