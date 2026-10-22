//! `pause()` collects the print's pauses and makes no geometry.

fn evaluate(src: &str) -> c2f_eval::Output {
    let program = c2f_eval::Program::load(src, std::path::Path::new("t.scad"), &[]).expect("parses");
    c2f_eval::evaluate(&program)
}

fn pause(z: f64, message: &str) -> c2f_eval::Pause {
    c2f_eval::Pause { z, message: message.to_string() }
}

#[test]
fn pauses_are_collected_in_call_order() {
    let output = evaluate(
        "h = 20;
        module lid() { pause(3, \"Lid magnets\"); cube([40, 30, 3.6]); }
        cube([40, 30, h]);
        pause(h - 0.6, message = \"Box magnets\");
        translate([50, 0, 0]) lid();
        if (false) pause(10);",
    );
    assert!(output.warnings.is_empty(), "{:?}", output.warnings);
    assert_eq!(output.pauses, vec![pause(19.4, "Box magnets"), pause(3.0, "Lid magnets")]);
    assert!(output.csg().contains("cube(size = [40, 30, 20]"), "{}", output.csg());
    assert!(!output.csg().contains("pause"));
}

#[test]
fn a_message_is_optional_and_any_value_reads_as_text() {
    let output = evaluate("pause(5); pause(6, 42);");
    assert_eq!(output.pauses, vec![pause(5.0, ""), pause(6.0, "42")]);
}

#[test]
fn a_pause_without_a_height_is_ignored() {
    let output = evaluate("pause(); pause(\"3\"); pause(1/0);");
    assert!(output.pauses.is_empty());
    assert_eq!(output.warnings.len(), 3, "{:?}", output.warnings);
    assert!(output.warnings[0].contains("pause()"), "{:?}", output.warnings);
}

#[test]
fn a_user_module_named_pause_wins() {
    let output = evaluate("module pause(z) { cube(z); } pause(2);");
    assert!(output.pauses.is_empty());
    assert!(output.csg().contains("cube(size = [2, 2, 2]"), "{}", output.csg());
}
