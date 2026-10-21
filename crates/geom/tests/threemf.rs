//! The 3MF export reads back through `import()` as the same solid.

use std::path::Path;

fn render(src: &str, dir: &Path) -> c2f_geom::Rendered {
    let program = c2f_eval::Program::load(src, &dir.join("test.scad"), &[]).expect("parses");
    c2f_geom::render_in(&c2f_eval::evaluate(&program).nodes, dir)
}

#[test]
fn export_reads_back_as_the_same_solid() {
    let dir = std::env::temp_dir().join(format!("c2f-threemf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (n, src) in [
        "difference() {
            sphere(d = 50, $fn = 100);
            rotate([90, 0, 0]) cylinder(d = 25, h = 62.5, center = true, $fn = 100);
        }",
        "color(\"red\") cube(10); color(\"blue\") translate([10, 0, 0]) cylinder(r = 5, h = 20, $fn = 40);",
    ]
    .into_iter()
    .enumerate()
    {
        let exported = render(src, &dir);
        let package = exported.three_mf().expect("a solid model");
        let file = format!("part{n}.3mf");
        std::fs::write(dir.join(&file), &package).unwrap();
        let imported = render(&format!("import(\"{file}\");"), &dir);
        assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
        let (a, b) = (exported.stats, imported.stats);
        assert_eq!(a.triangles, b.triangles, "{src}");
        assert_eq!(a.vertices, b.vertices, "{src}");
        assert!((a.volume - b.volume).abs() <= 1e-9 * a.volume, "{src}: volume {} vs {}", a.volume, b.volume);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn flat_and_empty_models_have_no_3mf() {
    let dir = std::env::temp_dir();
    assert!(render("square(10);", &dir).three_mf().is_none());
    assert!(render("", &dir).three_mf().is_none());
}

#[test]
fn same_model_same_file() {
    let dir = std::env::temp_dir();
    let a = render("cube(5);", &dir).three_mf().unwrap();
    let b = render("cube(5);", &dir).three_mf().unwrap();
    assert_eq!(a, b);
}
