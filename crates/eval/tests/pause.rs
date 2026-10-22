//! `pause()` collects the print's pauses and makes no geometry.

fn evaluate(src: &str) -> c2f_eval::Output {
    let program = c2f_eval::Program::load(src, std::path::Path::new("t.scad"), &[]).expect("parses");
    c2f_eval::evaluate(&program)
}

/// Each pause's height and message.
fn pauses(output: &c2f_eval::Output) -> Vec<(f64, &str)> {
    output.pauses.iter().map(|p| (p.z, p.message.as_str())).collect()
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
    assert_eq!(pauses(&output), [(19.4, "Box magnets"), (3.0, "Lid magnets")]);
    assert!(output.csg().contains("cube(size = [40, 30, 20]"), "{}", output.csg());
    assert!(!output.csg().contains("pause"));
}

#[test]
fn a_message_is_optional_and_any_value_reads_as_text() {
    let output = evaluate("pause(5); pause(6, 42);");
    assert_eq!(pauses(&output), [(5.0, ""), (6.0, "42")]);
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

#[test]
fn pauses_know_where_they_are_written() {
    let src = "h = 20;\nmodule lid() { pause(3.2); }\nlid(); lid();\npause(h - 0.6, \"Box\");\npause(message = \"x\", z = 7);\n";
    let output = evaluate(src);
    let at = |text: &str| src.find(text).unwrap();
    let sources: Vec<_> = output.pauses.iter().map(|p| p.source.clone().unwrap()).collect();
    // Both calls of `lid()` run the same `pause(3.2)`.
    assert_eq!(sources[0], sources[1]);
    assert_eq!(sources[0].call, at("pause(3.2)"));
    assert_eq!(&src[sources[0].number.clone().unwrap()], "3.2");
    // A computed height cannot be moved by rewriting a number.
    assert_eq!((sources[2].call, sources[2].number.clone()), (at("pause(h"), None));
    assert_eq!(&src[sources[3].number.clone().unwrap()], "7");
}

#[test]
fn pauses_from_a_library_have_no_source() {
    let dir = std::env::temp_dir().join(format!("c2f-pause-lib-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("lib.scad"), "module magnets() { pause(4); }\n").unwrap();
    let program = c2f_eval::Program::load("use <lib.scad>\nmagnets();\n", &dir.join("main.scad"), &[]).expect("parses");
    let output = c2f_eval::evaluate(&program);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(pauses(&output), [(4.0, "")]);
    assert_eq!(output.pauses[0].source, None);
}
