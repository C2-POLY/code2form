//! Checks a part's annotations against its shape, as an agent or a person
//! who just wrote them would: the numbers are typed by hand from a
//! drawing, so a pin can land off the board, two pins on one pad, or a
//! port face the wrong way. The shape is the rendered part's bounding box.

use c2f_lang::Span;
use c2f_lang::annotations::{Annotations, PinKind};
use c2f_lang::dump::number;

use crate::check::{Finding, Severity};

/// How far, in millimeters, a pin or a mount hole may sit outside the
/// part's bounding box: annotations are rounded, and a pin's pad or a
/// lead's end may stick out a little.
pub const TOLERANCE: f64 = 0.5;

/// Two pins closer than this, in millimeters, are at the same place.
const SAME_PLACE: f64 = 0.01;

/// The problems of a part's annotations, in the order of its file, given
/// the bounding box of the part rendered with its default parameters
/// (none when it draws nothing).
///
/// Errors: a pin or a mount hole outside the bounding box, two pins at the
/// same place, a port facing into the part, a power pin without its
/// voltage, a mechanical figure (`mass`, `torque`…) without its unit. Warnings: no `@description`, no `@source`, nothing drawn.
pub fn check(annotations: &Annotations, bounds: Option<([f64; 3], [f64; 3])>) -> Vec<Finding> {
    let mut findings = Vec::new();
    let error = |rule, message: String, span| Finding { severity: Severity::Error, rule, message, span };
    let warning = |rule, message: &str| Finding {
        severity: Severity::Warning,
        rule,
        message: message.to_string(),
        span: Span::new(0, 0),
    };
    if let Some((lo, hi)) = bounds {
        let inside = |p: [f64; 3]| (0..3).all(|i| p[i] >= lo[i] - TOLERANCE && p[i] <= hi[i] + TOLERANCE);
        let size = format!("{} to {}", vector(&lo), vector(&hi));
        for pin in &annotations.pins {
            if !inside(pin.position) {
                let message = format!(
                    "pin `{}` at {} is outside the part, which spans {size}",
                    pin.name,
                    vector(&pin.position)
                );
                findings.push(error("pin_outside", message, pin.span));
            }
        }
        for hole in &annotations.mount_holes {
            if !inside(hole.position) {
                let message = format!("the mount hole at {} is outside the part, which spans {size}", vector(&hole.position));
                findings.push(error("hole_outside", message, hole.span));
            }
        }
        // A port opens out of the part: from the box's center towards the
        // port, not back into it.
        let center: [f64; 3] = std::array::from_fn(|i| (lo[i] + hi[i]) / 2.0);
        for port in &annotations.ports {
            let outwards: f64 = (0..3).map(|i| port.direction[i] * (port.position[i] - center[i])).sum();
            if outwards < -TOLERANCE {
                let message = format!(
                    "port `{}` at {} faces {}, into the part: its direction is the way the opening faces, out of it",
                    port.name,
                    vector(&port.position),
                    vector(&port.direction)
                );
                findings.push(error("port_inward", message, port.span));
            }
        }
    } else {
        findings.push(warning("empty", "the part draws nothing: its pins cannot be checked against its shape"));
    }
    for (k, pin) in annotations.pins.iter().enumerate() {
        let same = annotations.pins[..k].iter().find(|other| {
            (0..3).map(|i| (other.position[i] - pin.position[i]).powi(2)).sum::<f64>().sqrt() < SAME_PLACE
        });
        if let Some(other) = same {
            let message = format!("pins `{}` and `{}` are at the same place, {}", other.name, pin.name, vector(&pin.position));
            findings.push(error("same_place", message, pin.span));
        }
        if matches!(pin.kind, PinKind::PowerIn | PinKind::PowerOut) && pin.voltage.is_none() {
            let what = if pin.kind == PinKind::PowerIn { "the most it takes" } else { "what it supplies" };
            let message = format!(
                "power pin `{}` has no voltage: give {what}, as `5V`, so that the wiring checks can compare it",
                pin.name
            );
            findings.push(error("power_voltage", message, pin.span));
        }
    }
    // The figures the mechanical checks compute with read as quantities.
    for spec in &annotations.specs {
        if let Some(Err(message)) = crate::quantity::figure(spec) {
            findings.push(error("spec_quantity", message, spec.span));
        }
    }
    findings.sort_by_key(|finding| finding.span.start);
    if annotations.description.is_none() {
        findings.push(warning("description", "the part has no `// @description`: say what it is and does, in a sentence or two"));
    }
    if annotations.sources.is_empty() {
        findings.push(warning("source", "the part has no `// @source`: give its product page or its datasheet"));
    }
    findings
}

fn vector(v: &[f64]) -> String {
    let items: Vec<String> = v.iter().map(|&n| number(n)).collect();
    format!("[{}]", items.join(", "))
}

/// Whether `name` makes a part's file name and module: letters, digits
/// and `_`, not starting with a digit.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each finding as `rule: message`, for the part `src` within `bounds`.
    fn findings(src: &str, bounds: Option<([f64; 3], [f64; 3])>) -> Vec<String> {
        let (annotations, errors) = c2f_lang::annotations::read(src);
        assert!(errors.is_empty(), "{errors:?}");
        check(&annotations, bounds)
            .iter()
            .map(|f| format!("{} {}: {}", f.severity.name(), f.rule, f.message))
            .collect()
    }

    const BOARD: Option<([f64; 3], [f64; 3])> = Some(([0.0, 0.0, 0.0], [20.0, 10.0, 1.6]));
    const SHEET: &str = "// @description A board.\n// @source https://example.com\n";

    #[test]
    fn a_good_part_has_no_findings() {
        let src = format!(
            "{SHEET}\
// @pin VIN [1, 1, 1.6] power_in 5V
// @pin GND [3.54, 1, 1.6] ground
// @pin OUT [20.4, 5, 0.8] +x output 5V
// @mount_hole [18, 8] d=2
// @port usb [0, 5, 1] -x [8, 3]
// @port led [10, 5, 1.6] up d=1
"
        );
        assert_eq!(findings(&src, BOARD), Vec::<String>::new());
    }

    #[test]
    fn misplaced_annotations_are_errors() {
        let src = format!(
            "{SHEET}\
// @pin A [1, 1, 1.6]
// @pin B [25, 1, 1.6]
// @pin C [1, 1, 1.6]
// @mount_hole [18, -3] d=2
// @port usb [0, 5, 1] +x [8, 3]
// @port top [10, 5, 1.6] down d=1
"
        );
        assert_eq!(
            findings(&src, BOARD),
            [
                "error pin_outside: pin `B` at [25, 1, 1.6] is outside the part, which spans [0, 0, 0] to [20, 10, 1.6]",
                "error same_place: pins `A` and `C` are at the same place, [1, 1, 1.6]",
                "error hole_outside: the mount hole at [18, -3, 0] is outside the part, which spans [0, 0, 0] to [20, 10, 1.6]",
                "error port_inward: port `usb` at [0, 5, 1] faces [1, 0, 0], into the part: its direction is the way the opening faces, out of it",
                "error port_inward: port `top` at [10, 5, 1.6] faces [0, 0, -1], into the part: its direction is the way the opening faces, out of it",
            ]
        );
    }

    #[test]
    fn power_pins_need_their_voltage() {
        let src = format!("{SHEET}// @pin VIN [1, 1, 1.6] power_in\n// @pin 3V3 [2, 1, 1.6] power_out\n// @pin S [3, 1, 1.6] io\n");
        assert_eq!(
            findings(&src, BOARD),
            [
                "error power_voltage: power pin `VIN` has no voltage: give the most it takes, as `5V`, so that the wiring checks can compare it",
                "error power_voltage: power pin `3V3` has no voltage: give what it supplies, as `5V`, so that the wiring checks can compare it",
            ]
        );
    }

    #[test]
    fn a_part_without_its_sheet_or_shape_gets_warnings() {
        assert_eq!(
            findings("// @pin A [0, 0, 0]\n", None),
            [
                "warning empty: the part draws nothing: its pins cannot be checked against its shape",
                "warning description: the part has no `// @description`: say what it is and does, in a sentence or two",
                "warning source: the part has no `// @source`: give its product page or its datasheet",
            ]
        );
    }

    #[test]
    fn names_make_a_file_and_a_module() {
        assert!(valid_name("my_sensor") && valid_name("_a1") && valid_name("drv8833"));
        assert!(!valid_name("") && !valid_name("2bad") && !valid_name("my sensor") && !valid_name("../x") && !valid_name("é"));
    }
}
