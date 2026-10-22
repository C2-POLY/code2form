//! Checking a circuit's wiring: each rule on a circuit that breaks it and
//! on one that does not.

use std::path::{Path, PathBuf};

use c2f_circuit::check::{self, Severity};
use c2f_circuit::{Circuit, End, Error};

/// A board that supplies 5 V and 3.3 V.
const BOARD: &str = "\
// @pin V5    [0, 0, 0] power_out 5V
// @pin V3    [1, 0, 0] power_out 3.3V
// @pin GND   [2, 0, 0] ground
// @pin IO    [3, 0, 0] io 3.3V
// @pin STRAP [4, 0, 0] io 3.3V reserved \"boot mode\"
// @pin NC    [5, 0, 0] nc
// @pin OUT   [6, 0, 0] output 3.3V
cube(1);
";

/// A 3.3 V sensor with two ground pins.
const SENSOR: &str = "\
// @pin VDD  [0, 0, 0] power_in 3.6V
// @pin GND  [1, 0, 0] ground
// @pin GND2 [1, 1, 0] ground
// @pin DATA [2, 0, 0] output 3.6V
// @pin IN   [3, 0, 0] input 3.6V
cube(1);
";

/// The board `b` and the sensors `s` and `t`, wired as told.
fn circuit(name: &str, wires: &[(&str, &str)]) -> Circuit {
    let dir = std::env::temp_dir().join(format!("c2f-check-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("board.c2f"), BOARD).unwrap();
    std::fs::write(dir.join("sensor.c2f"), SENSOR).unwrap();
    let mut src = String::from(
        "[parts.b]\npart = \"board.c2f\"\n\n[parts.s]\npart = \"sensor.c2f\"\n\n[parts.t]\npart = \"sensor.c2f\"\n",
    );
    for (from, to) in wires {
        src += &format!("\n[[wires]]\nfrom = \"{from}\"\nto = \"{to}\"\n");
    }
    let circuit = c2f_circuit::parse(&src, &dir.join("test.c2fc"), &[]);
    let _ = std::fs::remove_dir_all(&dir);
    circuit.unwrap_or_else(|errors| panic!("{}", errors.iter().map(Error::render).collect::<String>()))
}

/// Each finding as `error: message` or `warning: message`.
fn findings(name: &str, wires: &[(&str, &str)]) -> Vec<String> {
    check::check(&circuit(name, wires)).iter().map(|f| format!("{}: {}", f.severity.name(), f.message)).collect()
}

/// The sensor `s` powered at 3.3 V, grounded and read.
const GOOD: [(&str, &str); 3] = [("b.V3", "s.VDD"), ("b.GND", "s.GND"), ("s.DATA", "b.IO")];

fn with(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
    GOOD.iter().chain(extra).copied().collect()
}

#[test]
fn a_good_circuit_has_no_findings() {
    assert_eq!(findings("good", &GOOD), Vec::<String>::new());
}

#[test]
fn a_supply_above_what_a_pin_takes() {
    assert_eq!(
        findings("over", &[("b.V5", "s.VDD"), ("b.GND", "s.GND")]),
        ["error: `b.V5` supplies 5 V to `s.VDD`, which takes at most 3.6 V"]
    );
    // An io pin of the board on its own 3.3 V is fine.
    assert_eq!(findings("io", &with(&[("b.V3", "b.IO")])), Vec::<String>::new());
}

#[test]
fn two_supplies_tied_together() {
    assert_eq!(
        findings("tied", &with(&[("b.V5", "b.V3")])),
        [
            "error: `b.V5` supplies 5 V to `s.VDD`, which takes at most 3.6 V",
            "error: `b.V3` (3.3 V) and `b.V5` (5 V) are tied together: two supplies of different voltages",
        ]
    );
}

#[test]
fn a_supply_wired_to_ground() {
    assert_eq!(
        findings("short", &with(&[("b.V3", "s.GND2")])),
        ["error: `b.V3` is a supply wired to the ground `b.GND`: a short circuit"]
    );
}

#[test]
fn a_supply_or_ground_left_open() {
    assert_eq!(
        findings("no-supply", &[("b.GND", "s.GND"), ("s.DATA", "b.IO")]),
        ["error: `s.VDD` is not wired, though other pins of `s` are: a supply it needs is left open"]
    );
    assert_eq!(
        findings("no-ground", &[("b.V3", "s.VDD"), ("s.DATA", "b.IO")]),
        [
            "error: `b.GND` is not wired, though other pins of `b` are: its ground is left open",
            "error: `s.GND` is not wired, though other pins of `s` are: its ground is left open",
        ]
    );
    // One ground pin grounds the part; `t`, not wired at all, needs nothing.
    assert_eq!(findings("one-ground", &[("b.V3", "s.VDD"), ("b.GND", "s.GND2")]), Vec::<String>::new());
}

#[test]
fn a_wired_nc_pin() {
    assert_eq!(findings("nc", &with(&[("b.NC", "s.IN")])), ["error: `b.NC` is not connected (nc): nothing may be wired to it"]);
}

#[test]
fn grounds_not_all_one_net() {
    let apart = [("b.V3", "t.VDD"), ("t.GND", "t.IN")];
    assert_eq!(
        findings("apart", &with(&apart)),
        ["warning: no common ground: `t.GND` is not connected to `b.GND`"]
    );
    // Through the sensor's other ground pin, `t` shares the board's ground.
    let shared = [("b.V3", "t.VDD"), ("s.GND2", "t.GND")];
    assert_eq!(findings("shared", &with(&shared)), Vec::<String>::new());
}

#[test]
fn a_wired_reserved_pin() {
    assert_eq!(
        findings("reserved", &with(&[("b.STRAP", "s.IN")])),
        ["warning: `b.STRAP` is reserved by its part (boot mode): better left unwired"]
    );
}

#[test]
fn two_outputs_on_one_net() {
    assert_eq!(
        findings("outputs", &with(&[("t.DATA", "b.IO"), ("b.V3", "t.VDD"), ("b.GND", "t.GND")])),
        ["warning: `s.DATA` and `t.DATA` are two outputs on one net: they drive it against each other"]
    );
}

#[test]
fn findings_point_at_their_wire() {
    let circuit = circuit("span", &with(&[("b.STRAP", "s.IN")]));
    let finding = &check::check(&circuit)[0];
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(finding.span, circuit.wires[3].span);
    // The net of a pin, in the order wires reach it.
    let pin = |part: usize, name: &str| End { part, pin: circuit.parts[part].pin(name).unwrap() };
    assert_eq!(check::net_of(&circuit, pin(1, "DATA")), [pin(1, "DATA"), pin(0, "IO")]);
    assert_eq!(check::net_of(&circuit, pin(2, "IN")), [pin(2, "IN")]);
}

#[test]
fn grogu_has_one_warning_its_strapping_pin() {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/grogu/grogu.c2fc");
    let library = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let circuit = c2f_circuit::load(&path, &[library]).unwrap_or_else(|errors| {
        panic!("{}", errors.iter().map(Error::render).collect::<String>())
    });
    let findings: Vec<String> =
        check::check(&circuit).iter().map(|f| format!("{}: {}", f.severity.name(), f.message)).collect();
    assert_eq!(findings, ["warning: `xiao.D2` is reserved by its part (GPIO3, A2, strapping (JTAG source)): better left unwired"]);
}
