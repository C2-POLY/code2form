//! The STL export must load in slicers built on admesh (PrusaSlicer, Bambu
//! Studio): it reads each normal component into 31 characters and counts
//! facets by lines read 99 characters at a time.

use std::path::Path;

fn stl(src: &str) -> String {
    let program = c2f_eval::Program::load(src, Path::new("test.scad"), &[]).expect("parses");
    let output = c2f_eval::evaluate(&program);
    c2f_geom::render(&output.nodes).stl().expect("a solid model")
}

#[test]
fn numbers_fit_admesh() {
    // Rotated cylinders leave normal components around 1e-31, which plain
    // notation spells with 48 characters.
    let text = stl("difference() {
        sphere(d = 50, $fn = 100);
        rotate([90, 0, 0]) cylinder(d = 25, h = 62.5, center = true, $fn = 100);
        rotate([0, 90, 0]) cylinder(d = 25, h = 62.5, center = true, $fn = 100);
    }");
    let mut facets = 0;
    for line in text.lines() {
        assert!(line.len() < 99, "line too long for admesh: {line}");
        let words: Vec<&str> = line.split_whitespace().collect();
        let numbers = match words.as_slice() {
            ["facet", "normal", rest @ ..] => {
                facets += 1;
                rest
            }
            ["vertex", rest @ ..] => rest,
            _ => continue,
        };
        assert_eq!(numbers.len(), 3, "{line}");
        for number in numbers {
            assert!(number.len() <= 31, "number too long for admesh: {number}");
            assert!(number.parse::<f64>().is_ok_and(f64::is_finite), "not a number: {number}");
        }
    }
    assert!(facets > 1000, "{facets} facets");
}
