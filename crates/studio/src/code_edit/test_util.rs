//! Checks shared by the tests of the edits: the rewritten source must
//! still parse, and where it is cheap, render to the same solid as the
//! same design written by hand.

use std::ops::Range;
use std::path::Path;

/// The byte range of the `n`th (from 0) occurrence of `needle`, through
/// the end of its statement: `needle` must hold the whole statement.
pub fn at(src: &str, needle: &str, n: usize) -> Range<usize> {
    let start = src.match_indices(needle).nth(n).unwrap_or_else(|| panic!("{needle:?} not in {src:?}")).0;
    start..start + needle.len()
}

/// The source parses.
pub fn parses(src: &str) {
    if let Err(err) = c2f_lang::parse(src) {
        panic!("does not parse: {}\n{src}", err.message);
    }
}

/// The volume and bounding box of the solid a program renders.
pub fn solid(src: &str) -> (f64, [f64; 3], [f64; 3]) {
    let program = c2f_eval::Program::load(src, Path::new("test.scad"), &[]).expect("loads");
    let output = c2f_eval::evaluate(&program);
    assert!(output.error.is_none(), "{:?}", output.error);
    let rendered = c2f_geom::render(&output.nodes);
    let (min, max) = rendered.bounds().expect("not empty");
    (rendered.stats.volume, min, max)
}

/// Two programs render to the same solid, to a thousandth.
pub fn same_solid(a: &str, b: &str) {
    let (va, mina, maxa) = solid(a);
    let (vb, minb, maxb) = solid(b);
    let close = |x: f64, y: f64| (x - y).abs() < 1e-3 * (1.0 + x.abs());
    assert!(close(va, vb), "volume {va} vs {vb}\n{a}\n---\n{b}");
    for i in 0..3 {
        assert!(close(mina[i], minb[i]) && close(maxa[i], maxb[i]), "bounds differ\n{a}\n---\n{b}");
    }
}
