//! SVG and DXF exports of 2D models against the reference implementation
//! (2021.01), and read back through `import()`.
//!
//! The expected numbers were measured on the macOS build of the reference
//! release (2021.01): each example was exported with its `-o out.svg` and
//! `-o out.dxf`, and the contours, area (shoelace over the exported
//! points) and bounds read from both files, which agree. The reference
//! prints six significant digits, hence the tolerances. The text example
//! needs the Liberation fonts, as `tests/text.rs` does.

use std::path::Path;

use c2f_geom::{Model, Rendered};

fn render(src: &str, dir: &Path) -> Rendered {
    let program = c2f_eval::Program::load(src, &dir.join("test.scad"), &[]).expect("parses");
    c2f_geom::render_in(&c2f_eval::evaluate(&program).nodes, dir)
}

struct Oracle {
    src: &'static str,
    /// The reference's `<svg>` element, with the page size.
    svg_element: &'static str,
    /// Vertices per contour, in any order.
    contours: &'static [usize],
    area: f64,
    /// `[min x, min y, max x, max y]`.
    bounds: [f64; 4],
}

const EXAMPLES: [Oracle; 3] = [
    // A hole, and a separate part off the page's whole millimeters.
    Oracle {
        src: "difference() { square(30, center = true); circle(r = 10.3, $fn = 20); }
              translate([40, -7.25]) circle(r = 4.4, $fn = 9);",
        svg_element: r#"<svg width="60mm" height="30mm" viewBox="-15 -15 60 30" xmlns="http://www.w3.org/2000/svg" version="1.1">"#,
        contours: &[4, 9, 20],
        area: 628.164360,
        bounds: [-15.0, -15.0, 44.4, 15.0],
    },
    // Curved glyphs, with holes, below the baseline.
    Oracle {
        src: r#"text("Laser", size = 12, font = "Liberation Sans:style=Bold");"#,
        svg_element: r#"<svg width="43mm" height="13mm" viewBox="1 -12 43 13" xmlns="http://www.w3.org/2000/svg" version="1.1">"#,
        contours: &[6, 17, 34, 43, 50, 94, 110],
        area: 195.417214,
        bounds: [1.11479, -0.16272, 43.3311, 11.4658],
    },
    // A polygon with a hole given by its paths, scaled and rotated.
    Oracle {
        src: "rotate(17) scale([1.5, 0.8]) polygon(
                points = concat([for (i = [0:9]) let (r = i % 2 == 0 ? 20 : 8) [r * cos(36 * i), r * sin(36 * i)]],
                                [[-3, -3], [3, -3], [3, 3], [-3, 3]]),
                paths = [[0, 1, 2, 3, 4, 5, 6, 7, 8, 9], [10, 11, 12, 13]]);",
        svg_element: r#"<svg width="55mm" height="35mm" viewBox="-26 -18 55 35" xmlns="http://www.w3.org/2000/svg" version="1.1">"#,
        contours: &[4, 10],
        area: 521.072647,
        bounds: [-25.9596, -16.0896, 28.6891, 17.2624],
    },
];

/// The subpaths of the SVG's one path, y negated back.
fn svg_contours(svg: &str) -> Vec<Vec<[f64; 2]>> {
    let start = svg.find("<path d=\"").expect("a path") + 9;
    let d = &svg[start..start + svg[start..].find('"').expect("a closed path")];
    d.split('z')
        .map(|sub| {
            sub.split(['M', 'L'])
                .filter_map(|point| point.trim().split_once(','))
                .map(|(x, y)| [x.parse().unwrap(), -y.parse::<f64>().unwrap()])
                .collect::<Vec<_>>()
        })
        .filter(|contour| !contour.is_empty())
        .collect()
}

/// The DXF's LINEs chained back into outlines: each outline's edges come
/// in order and the last one ends where the first began.
fn dxf_contours(dxf: &str) -> Vec<Vec<[f64; 2]>> {
    let lines: Vec<&str> = dxf.lines().map(str::trim).collect();
    let (mut contours, mut current) = (Vec::new(), Vec::new());
    for entity in lines.windows(12).filter(|w| w[0] == "0" && w[1] == "LINE") {
        assert_eq!([entity[2], entity[4], entity[6], entity[8], entity[10]], ["8", "10", "20", "11", "21"]);
        let value = |at: usize| entity[at].parse::<f64>().unwrap();
        let (from, to) = ([value(5), value(7)], [value(9), value(11)]);
        current.push(from);
        if to == current[0] {
            contours.push(std::mem::take(&mut current));
        }
    }
    assert!(current.is_empty(), "an outline does not close");
    contours
}

fn shoelace(contours: &[Vec<[f64; 2]>]) -> f64 {
    let ring = |c: &Vec<[f64; 2]>| -> f64 {
        (0..c.len()).map(|i| c[i][0] * c[(i + 1) % c.len()][1] - c[(i + 1) % c.len()][0] * c[i][1]).sum()
    };
    contours.iter().map(ring).sum::<f64>() / 2.0
}

fn check(what: &str, contours: &[Vec<[f64; 2]>], expected: &Oracle) {
    let mut counts: Vec<usize> = contours.iter().map(Vec::len).collect();
    let mut want = expected.contours.to_vec();
    counts.sort_unstable();
    want.sort_unstable();
    assert_eq!(counts, want, "{what}: vertices per contour");
    // The oracle's area comes from its rounded points.
    let area = shoelace(contours);
    assert!((area - expected.area).abs() <= 5e-6 * expected.area, "{what}: area {area} vs {}", expected.area);
    let points = contours.iter().flatten();
    let bounds = points.fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, p| {
        [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]
    });
    for (got, want) in bounds.iter().zip(expected.bounds) {
        assert!((got - want).abs() <= 6e-6 * want.abs().max(1e-3), "{what}: bounds {bounds:?} vs {:?}", expected.bounds);
    }
}

#[test]
fn svg_and_dxf_match_the_reference() {
    let dir = std::env::temp_dir();
    for expected in &EXAMPLES {
        let rendered = render(expected.src, &dir);
        assert!(rendered.warnings.is_empty(), "{}: {:?}", expected.src, rendered.warnings);
        let svg = rendered.svg().expect("a 2D model");
        assert_eq!(svg.lines().nth(2), Some(expected.svg_element), "{}", expected.src);
        check(&format!("SVG of {}", expected.src), &svg_contours(&svg), expected);
        let dxf = rendered.dxf().expect("a 2D model");
        check(&format!("DXF of {}", expected.src), &dxf_contours(&dxf), expected);
    }
}

#[test]
fn exports_read_back_through_import() {
    let dir = std::env::temp_dir().join(format!("c2f-flat-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (n, expected) in EXAMPLES.iter().enumerate() {
        let exported = render(expected.src, &dir);
        let Model::Flat(original) = &exported.model else { panic!("{} is 2D", expected.src) };
        for (extension, file) in [("dxf", exported.dxf().unwrap()), ("svg", exported.svg().unwrap())] {
            let name = format!("part{n}.{extension}");
            std::fs::write(dir.join(&name), file).unwrap();
            let imported = render(&format!("import(\"{name}\");"), &dir);
            assert!(imported.warnings.is_empty(), "{name}: {:?}", imported.warnings);
            let Model::Flat(section) = &imported.model else { panic!("{name} reads back as 2D") };
            assert_eq!(section.num_contour(), original.num_contour(), "{name}");
            // The DXF reader snaps points to a 1/1024 grid.
            let (a, b) = (original.area(), section.area());
            assert!((a - b).abs() <= 1e-4 * a, "{name}: area {b} vs {a}");
            // The SVG reader moves the drawing (the page's corner to the
            // origin, and the reference's sign slip on the viewBox's y), as
            // the reference's reader does with its own SVG: only the size
            // is compared.
            let (p, q) = (original.bounds(), section.bounds());
            let sizes = [(q.max.x - q.min.x, p.max.x - p.min.x), (q.max.y - q.min.y, p.max.y - p.min.y)];
            for (got, want) in sizes {
                assert!((got - want).abs() <= 2e-3, "{name}: size of {q:?} vs {p:?}");
            }
            if extension == "dxf" {
                for (got, want) in [q.min.x, q.min.y].into_iter().zip([p.min.x, p.min.y]) {
                    assert!((got - want).abs() <= 1e-3, "{name}: bounds {q:?} vs {p:?}");
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn solid_and_empty_models_have_no_drawing() {
    let dir = std::env::temp_dir();
    for src in ["cube(10);", ""] {
        let rendered = render(src, &dir);
        assert!(rendered.svg().is_none() && rendered.dxf().is_none(), "{src:?}");
    }
}

#[test]
fn layout_of_a_square() {
    let rendered = render("square([20, 10]);", &std::env::temp_dir());
    let svg = rendered.svg().unwrap();
    let path = &svg[svg.find("<path").unwrap()..];
    assert_eq!(
        path,
        "<path d=\"\nM 0.0,0.0 L 20.0,0.0 L 20.0,-10.0 L 0.0,-10.0 z\n\" stroke=\"black\" fill=\"lightgray\" stroke-width=\"0.5\"/>\n</svg>\n"
    );
    let dxf = rendered.dxf().unwrap();
    assert!(dxf.starts_with("  0\nSECTION\n  2\nBLOCKS\n  0\nENDSEC\n  0\nSECTION\n  2\nENTITIES\n  0\nLINE\n  8\n0\n 10\n0.0\n 20\n0.0\n 11\n20.0\n 21\n0.0\n"));
    assert!(dxf.ends_with("  0\nENDSEC\n  0\nSECTION\n  2\nOBJECTS\n  0\nDICTIONARY\n  0\nENDSEC\n  0\nEOF\n"));
    assert_eq!(dxf.matches("LINE\n").count(), 4);
}
