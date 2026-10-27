//! The frames of the website's scroll story: a model turning on itself
//! (`turn-00.png` .. `turn-35.png`, every 10 degrees) and its parts coming
//! apart (`explode-00.png` .. `explode-23.png`), drawn by the app's own
//! software renderer at 1200 by 900 pixels.
//!
//! `site/scripts/frames.sh` runs it on the watch winder, then crops and
//! compresses the pictures into `site/public/frames`:
//!
//! ```sh
//! cargo run --release -p c2f-geom --example site_frames -- examples/winder/watch_winder.c2f <dir>
//! ```

use std::path::Path;

use c2f_geom::{Explode, RenderMesh};

const TURNS: usize = 36;
const EXPLODES: usize = 24;
/// How far the parts move at the end, as a multiple of their full move.
const SPREAD: f32 = 2.4;
const PITCH: f32 = 22.0;
const START_YAW: f32 = -50.0;
const SIZE: (usize, usize) = (1200, 900);

fn grow(lo: &mut [f32; 3], hi: &mut [f32; 3], mesh: &RenderMesh) {
    if let Some((l, h)) = mesh.bounds() {
        for k in 0..3 {
            lo[k] = lo[k].min(l[k]);
            hi[k] = hi[k].max(h[k]);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, file, out] = args.as_slice() else {
        eprintln!("usage: site_frames <model.c2f> <output directory>");
        std::process::exit(2);
    };
    let (file, out) = (Path::new(file), Path::new(out));
    std::fs::create_dir_all(out).expect("output directory");
    let src = c2f_eval::read_source(file).expect("model file");
    let program = c2f_eval::Program::load(&src, file, &c2f_eval::default_library_paths())
        .unwrap_or_else(|e| panic!("{}", e.render()));
    let output = c2f_eval::evaluate(&program);
    assert!(output.error.is_none(), "{:?}", output.error);
    let base = file.parent().unwrap_or(Path::new(""));
    let mut rendered = c2f_geom::render_in(&output.nodes, base);
    let parts = c2f_geom::exploded(&output.nodes, base).expect("the model has no part()");
    let assembled = parts.at(0.0, Explode::Radial);
    let apart = parts.at(SPREAD, Explode::Radial);

    // Two invisible points at the corners of a box keep the framing the
    // same in every frame: the assembled model's box while it turns, and
    // the box around it assembled and apart while it comes apart.
    let framing = |meshes: &[&RenderMesh]| {
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for mesh in meshes {
            grow(&mut lo, &mut hi, mesh);
        }
        let mut frame = RenderMesh::default();
        frame.push_triangle([lo, lo, lo], [[0.0; 4]; 3]);
        frame.push_triangle([hi, hi, hi], [[0.0; 4]; 3]);
        frame
    };
    rendered.highlights = RenderMesh::default();

    let (w, h) = SIZE;
    let write = |name: String, png: Option<Vec<u8>>| {
        std::fs::write(out.join(name), png.expect("nothing to draw")).expect("write frame");
    };
    rendered.backgrounds = framing(&[&assembled]);
    rendered.solid = assembled.clone();
    for i in 0..TURNS {
        let yaw = START_YAW + i as f32 * 360.0 / TURNS as f32;
        write(format!("turn-{i:02}.png"), rendered.png(yaw, PITCH, w, h));
    }
    rendered.backgrounds = framing(&[&assembled, &apart]);
    for i in 0..EXPLODES {
        let t = i as f32 / (EXPLODES - 1) as f32;
        let eased = t * t * (3.0 - 2.0 * t);
        rendered.solid = parts.at(eased * SPREAD, Explode::Radial);
        write(format!("explode-{i:02}.png"), rendered.png(START_YAW, PITCH, w, h));
    }
}
