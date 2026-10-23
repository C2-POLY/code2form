//! `part("name")` keeps a part of the model apart, for glTF exports; the
//! CSG tree reads it as a group, as the other exports do.

fn evaluate(src: &str) -> c2f_eval::Output {
    let program = c2f_eval::Program::load(src, std::path::Path::new("t.scad"), &[]).expect("parses");
    c2f_eval::evaluate(&program)
}

#[test]
fn a_part_holds_its_children_and_prints_as_a_group() {
    let output = evaluate("translate([1, 2, 3]) part(\"lid\") { cube(1); sphere(1); }");
    assert!(output.warnings.is_empty(), "{:?}", output.warnings);
    let lid = &output.nodes[0].children[0];
    assert_eq!(lid.kind, c2f_eval::NodeKind::Part { name: "lid".into(), joint: None });
    assert_eq!(lid.children.len(), 2);
    assert!(output.csg().contains("\tgroup() {\n\t\tcube("), "{}", output.csg());
}

#[test]
fn a_part_without_a_name_is_named_part() {
    let output = evaluate("part() cube(1); part(42) cube(1); part(name = \"x\") cube(1);");
    let names: Vec<_> = output.nodes.iter().map(|n| n.kind.clone()).collect();
    let part = |name: &str| c2f_eval::NodeKind::Part { name: name.into(), joint: None };
    assert_eq!(names, [part("part"), part("42"), part("x")]);
}

#[test]
fn its_children_see_its_special_variables() {
    let output = evaluate("part(\"wheel\", $fn = 6) circle(1);");
    assert!(output.csg().contains("$fn = 6"), "{}", output.csg());
}

#[test]
fn a_user_module_named_part_wins() {
    let output = evaluate("module part(name) { echo(name); children(); } part(\"lid\") cube(1);");
    assert_eq!(output.echo, ["ECHO: \"lid\""]);
    assert_eq!(output.nodes[0].kind, c2f_eval::NodeKind::Group);
}

#[test]
fn a_joint_says_how_a_part_moves() {
    use c2f_eval::node::{Joint, JointKind};
    let output = evaluate(
        "part(\"elbow\", joint = \"revolute\", axis = [0, 2, 0], limits = [-90, 45]) cube(1);
        part(\"wheel\", joint = \"continuous\") cube(1);
        part(\"bad\", joint = \"hinge\") cube(1);
        part(\"slide\", joint = \"prismatic\", axis = [0, 0, 0], limits = [5, 1]) cube(1);",
    );
    let joint = |i: usize| match &output.nodes[i].kind {
        c2f_eval::NodeKind::Part { joint, .. } => joint.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(joint(0), Some(Joint { kind: JointKind::Revolute, axis: [0.0, 1.0, 0.0], limits: Some([-90.0, 45.0]) }));
    assert_eq!(joint(1), Some(Joint { kind: JointKind::Continuous, axis: [0.0, 0.0, 1.0], limits: None }));
    assert_eq!(joint(2), None);
    assert_eq!(joint(3), Some(Joint { kind: JointKind::Prismatic, axis: [0.0, 0.0, 1.0], limits: None }));
    assert_eq!(output.warnings.len(), 3, "{:?}", output.warnings);
    assert!(output.warnings[0].contains("unknown joint \"hinge\""), "{:?}", output.warnings);
}
