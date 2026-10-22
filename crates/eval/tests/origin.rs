//! Each node remembers the instantiation that made it, and where that
//! statement is written: what a 3D view needs to find a clicked object's
//! line in the source.

use c2f_eval::{CallKind, Node, Output};

fn evaluate(src: &str) -> Output {
    let program = c2f_eval::Program::load(src, std::path::Path::new("t.scad"), &[]).expect("parses");
    c2f_eval::evaluate(&program)
}

/// The source text of a node's statement.
fn text<'a>(src: &'a str, output: &Output, node: &Node) -> &'a str {
    let origin = &output.origins[node.origin.expect("an origin") as usize];
    &src[origin.span.range()]
}

#[test]
fn a_node_points_at_its_whole_statement() {
    let src = "translate([1, 0, 0]) #cube(2);\nsphere(1);";
    let output = evaluate(src);
    assert_eq!(output.nodes.len(), 2);
    assert_eq!(text(src, &output, &output.nodes[0]), "translate([1, 0, 0]) #cube(2);");
    assert_eq!(text(src, &output, &output.nodes[0].children[0]), "#cube(2);");
    assert_eq!(text(src, &output, &output.nodes[1]), "sphere(1);");
    let cube = &output.origins[output.nodes[0].children[0].origin.unwrap() as usize];
    assert_eq!(cube.parent, output.nodes[0].origin);
    assert_eq!(cube.kind, CallKind::Builtin);
    assert!(cube.in_main_file());
}

#[test]
fn a_statement_run_several_times_has_one_origin_per_run() {
    let src = "for (i = [0:2]) translate([i * 3, 0, 0]) cube(1);";
    let output = evaluate(src);
    let cubes: Vec<_> = output.origins.iter().filter(|o| &src[o.span.range()] == "cube(1);").collect();
    assert_eq!(cubes.len(), 3);
    let parents: Vec<_> = cubes.iter().map(|c| c.parent.unwrap()).collect();
    assert_eq!(parents.len(), 3);
    assert!(parents.windows(2).all(|p| p[0] != p[1]));
    assert_eq!(output.origins[0].kind, CallKind::Flow);
}

#[test]
fn a_user_module_call_is_the_parent_of_its_body() {
    let src = "module post(h) { cylinder(h, 1, 1); }\nmodule pair() { post(2); translate([4, 0, 0]) children(); }\npair() post(5);";
    let output = evaluate(src);
    let top = &output.nodes[0];
    assert_eq!(text(src, &output, top), "pair() post(5);");
    assert_eq!(output.origins[top.origin.unwrap() as usize].kind, CallKind::Module);
    // The children of `pair()` keep the origin of the statement that
    // wrote them, at the call site, under the `children()` that ran them.
    let mut seen = Vec::new();
    collect(top, &mut seen);
    let texts: Vec<_> = seen.iter().map(|&o| &src[output.origins[o as usize].span.range()]).collect();
    assert!(texts.contains(&"post(2);"), "{texts:?}");
    assert!(texts.contains(&"post(5);"), "{texts:?}");
    assert!(texts.contains(&"cylinder(h, 1, 1);"), "{texts:?}");
}

fn collect(node: &Node, out: &mut Vec<u32>) {
    out.extend(node.origin);
    for child in &node.children {
        collect(child, out);
    }
}

#[test]
fn an_included_statement_is_not_in_the_main_file() {
    let dir = std::env::temp_dir().join(format!("c2f-origin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("part.scad"), "cube(3);\n").unwrap();
    let src = "include <part.scad>\nsphere(1);";
    let program = c2f_eval::Program::load(src, &dir.join("main.scad"), &[]).expect("parses");
    let output = c2f_eval::evaluate(&program);
    std::fs::remove_dir_all(&dir).ok();
    let origins: Vec<_> = output.nodes.iter().map(|n| &output.origins[n.origin.unwrap() as usize]).collect();
    assert_eq!(origins.len(), 2);
    assert!(!origins[0].in_main_file());
    assert!(origins[1].in_main_file());
}
