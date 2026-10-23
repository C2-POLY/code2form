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
    assert_eq!(lid.kind, c2f_eval::NodeKind::Part { name: "lid".into() });
    assert_eq!(lid.children.len(), 2);
    assert!(output.csg().contains("\tgroup() {\n\t\tcube("), "{}", output.csg());
}

#[test]
fn a_part_without_a_name_is_named_part() {
    let output = evaluate("part() cube(1); part(42) cube(1); part(name = \"x\") cube(1);");
    let names: Vec<_> = output.nodes.iter().map(|n| n.kind.clone()).collect();
    let part = |name: &str| c2f_eval::NodeKind::Part { name: name.into() };
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
