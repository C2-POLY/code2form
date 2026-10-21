//! DXF drawings, read like `DxfData` in the reference implementation
//! (2021.01): LINE, LWPOLYLINE (straight segments, bulges ignored),
//! CIRCLE, ARC, ELLIPSE and INSERT of BLOCKs become line segments; other
//! entities (POLYLINE, SPLINE, TEXT...) are reported as unsupported, as
//! the reference does.
//!
//! Endpoints snap to a 1/1024 grid (or to a point already within one grid
//! step), segments meeting at a point are chained into paths, open paths
//! first, and every path of three or more points becomes an outline:
//! open paths are closed by their last edge.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::f64::consts::PI;

use super::DxfParams;
use crate::primitives::fragments;

/// The reference's `GRID_COARSE`.
const GRID: f64 = 0.0009765625;

/// Points snapped to the grid, and the segments touching each grid cell.
#[derive(Default)]
struct Grid {
    cells: HashMap<(i64, i64), Vec<usize>>,
}

impl Grid {
    /// The reference's `Grid2d::align`: the cell of `p`, or an existing
    /// cell around it. The search moves its center when it finds a cell,
    /// as the original loop does.
    fn align(&mut self, [x, y]: [f64; 2]) -> (i64, i64) {
        let mut ix = (x / GRID).round() as i64;
        let mut iy = (y / GRID).round() as i64;
        if !self.cells.contains_key(&(ix, iy)) {
            let mut dist = 10;
            let mut jx = ix - 1;
            while jx <= ix + 1 {
                let mut jy = iy - 1;
                while jy <= iy + 1 {
                    if self.cells.contains_key(&(jx, jy)) {
                        let d = (ix - jx).abs() + (iy - jy).abs();
                        if d < dist {
                            dist = d;
                            ix = jx;
                            iy = jy;
                        }
                    }
                    jy += 1;
                }
                jx += 1;
            }
        }
        self.cells.entry((ix, iy)).or_default();
        (ix, iy)
    }

    fn at(&self, cell: (i64, i64)) -> &[usize] {
        self.cells.get(&cell).map_or(&[], Vec::as_slice)
    }
}

fn point_of((ix, iy): (i64, i64)) -> [f64; 2] {
    [ix as f64 * GRID, iy as f64 * GRID]
}

struct Line {
    /// Grid cells of the two ends.
    ends: [(i64, i64); 2],
    disabled: bool,
}

struct Reader<'a> {
    params: &'a DxfParams<'a>,
    grid: Grid,
    lines: Vec<Line>,
    /// Segments of each block, in block coordinates.
    blocks: HashMap<String, Vec<[[f64; 2]; 2]>>,
    in_entities: bool,
    in_blocks: bool,
    current_block: String,
    /// The entity being read: its group values so far.
    layer: String,
    name: String,
    xs: Vec<f64>,
    ys: Vec<f64>,
    radius: f64,
    arc_start: f64,
    arc_stop: f64,
    ellipse_start: f64,
    ellipse_stop: f64,
    flags: i64,
    unsupported: BTreeMap<String, usize>,
}

impl<'a> Reader<'a> {
    fn new(params: &'a DxfParams<'a>) -> Self {
        Reader {
            params,
            grid: Grid::default(),
            lines: Vec::new(),
            blocks: HashMap::new(),
            in_entities: false,
            in_blocks: false,
            current_block: String::new(),
            layer: String::new(),
            name: String::new(),
            xs: Vec::new(),
            ys: Vec::new(),
            radius: 0.0,
            arc_start: 0.0,
            arc_stop: 0.0,
            ellipse_start: 0.0,
            ellipse_stop: 0.0,
            flags: 0,
            unsupported: BTreeMap::new(),
        }
    }

    /// The reference's `ADD_LINE`: a segment of the drawing, or of the
    /// block being defined.
    fn add_line(&mut self, p: [f64; 2], q: [f64; 2]) {
        if !self.in_entities && !self.in_blocks {
            return;
        }
        let layer = self.params.layer;
        if self.in_entities && !(layer.is_empty() || layer == self.layer) {
            return;
        }
        let a = self.grid.align(p);
        let b = self.grid.align(q);
        // In a block, the reference files the segment in the grid under
        // the index the next drawing segment will get.
        let index = self.lines.len();
        self.grid.cells.entry(a).or_default().push(index);
        self.grid.cells.entry(b).or_default().push(index);
        if self.in_entities {
            self.lines.push(Line { ends: [a, b], disabled: false });
        }
        if self.in_blocks && !self.current_block.is_empty() {
            self.blocks.entry(self.current_block.clone()).or_default().push([point_of(a), point_of(b)]);
        }
    }

    fn vertex(&self, i: usize) -> Option<[f64; 2]> {
        Some([*self.xs.get(i)?, *self.ys.get(i)?])
    }

    /// The entity `mode` is complete.
    fn finish_entity(&mut self, mode: &str) {
        let segments = self.params.segments;
        let frags = |r: f64| fragments(r, segments.fn_, segments.fs, segments.fa) as f64;
        match mode {
            "SECTION" => {
                self.in_entities = self.name == "ENTITIES";
                self.in_blocks = self.name == "BLOCKS";
            }
            "LINE" => {
                if let (Some(p), Some(q)) = (self.vertex(0), self.vertex(1)) {
                    self.add_line(p, q);
                }
            }
            "LWPOLYLINE" => {
                let n = self.xs.len().min(self.ys.len());
                for i in 1..n {
                    self.add_line([self.xs[i - 1], self.ys[i - 1]], [self.xs[i], self.ys[i]]);
                }
                if self.flags & 1 != 0 && n > 0 {
                    self.add_line([self.xs[n - 1], self.ys[n - 1]], [self.xs[0], self.ys[0]]);
                }
            }
            "CIRCLE" => {
                let Some([cx, cy]) = self.vertex(0) else { return };
                let r = self.radius;
                let n = frags(r) as u32;
                for i in 0..n {
                    let a1 = 2.0 * PI * i as f64 / n as f64;
                    let a2 = 2.0 * PI * (i + 1) as f64 / n as f64;
                    self.add_line([a1.cos() * r + cx, a1.sin() * r + cy], [a2.cos() * r + cx, a2.sin() * r + cy]);
                }
            }
            "ARC" => {
                let Some([cx, cy]) = self.vertex(0) else { return };
                let r = self.radius;
                let (start, mut stop) = (self.arc_start, self.arc_stop);
                while start > stop {
                    stop += 360.0;
                }
                let n = (frags(r) * (stop - start) / 360.0).ceil() as i64;
                for i in 0..n {
                    let a1 = ((stop - start) * i as f64) / n as f64;
                    let a2 = ((stop - start) * (i + 1) as f64) / n as f64;
                    let a1 = (start + a1) * PI / 180.0;
                    let a2 = (start + a2) * PI / 180.0;
                    self.add_line([a1.cos() * r + cx, a1.sin() * r + cy], [a2.cos() * r + cx, a2.sin() * r + cy]);
                }
            }
            "ELLIPSE" => {
                let (Some([cx, cy]), Some([ex, ey])) = (self.vertex(0), self.vertex(1)) else { return };
                let (start, mut stop) = (self.ellipse_start, self.ellipse_stop);
                while start > stop {
                    stop += 2.0 * PI;
                }
                // The end of the major axis is relative to the center, yet
                // moved by the origin like any point, as in the reference.
                let r_major = (ex * ex + ey * ey).sqrt();
                let mut rotation = (ex / r_major).clamp(-1.0, 1.0).acos();
                if ey < 0.0 {
                    rotation = 2.0 * PI - rotation;
                }
                // Group 40 is the axis ratio here, scaled like a radius.
                let r_minor = r_major * self.radius;
                let sweep = stop - start;
                let n = (frags(r_major) * sweep / (2.0 * PI)).ceil() as i64;
                let (sin, cos) = rotation.sin_cos();
                let mut previous = [0.0; 2];
                for i in 0..=n {
                    let a = start + sweep * i as f64 / n as f64;
                    let (x, y) = (a.cos() * r_major, a.sin() * r_minor);
                    let p = [cos * x - sin * y + cx, sin * x + cos * y + cy];
                    if i > 0 {
                        self.add_line(previous, p);
                    }
                    previous = p;
                }
            }
            "INSERT" => {
                let Some([px, py]) = self.vertex(0) else { return };
                let segments = self.blocks.get(&self.name).cloned().unwrap_or_default();
                // Rotation in group 50, scales in 41 and 42.
                let a = self.arc_start * PI / 180.0;
                let (sx, sy) = (self.ellipse_start, self.ellipse_stop);
                let scale = self.params.scale;
                let place = |[x, y]: [f64; 2]| {
                    let (lx, ly) = (x * sx, y * sy);
                    [(a.cos() * lx - a.sin() * ly) * scale + px, (a.sin() * lx + a.cos() * ly) * scale + py]
                };
                for [p, q] in segments {
                    self.add_line(place(p), place(q));
                }
            }
            "BLOCK" => self.current_block = self.name.clone(),
            "ENDBLK" => self.current_block.clear(),
            "ENDSEC" => {}
            "DIMENSION" if self.on_layer() => {}
            _ => {
                if self.in_blocks || (self.in_entities && self.on_layer()) {
                    *self.unsupported.entry(mode.to_string()).or_default() += 1;
                }
            }
        }
    }

    fn on_layer(&self) -> bool {
        let layer = self.params.layer;
        layer.is_empty() || layer == self.layer
    }

    fn start_entity(&mut self, mode: &str) {
        self.layer.clear();
        self.name.clear();
        self.flags = 0;
        self.xs.clear();
        self.ys.clear();
        self.radius = 0.0;
        self.arc_start = 0.0;
        self.arc_stop = 0.0;
        // An INSERT's scales default to 1.
        let unit = if mode == "INSERT" { 1.0 } else { 0.0 };
        self.ellipse_start = unit;
        self.ellipse_stop = unit;
    }

    /// One group of the entity being read.
    fn group(&mut self, code: i32, value: &str) -> Result<(), ()> {
        let number = || value.parse::<f64>().map_err(|_| ());
        let DxfParams { origin, scale, .. } = *self.params;
        match code {
            1 => {}
            2 => self.name = value.to_string(),
            8 => self.layer = value.to_string(),
            10 | 11 => {
                let x = number()?;
                self.xs.push(if self.in_blocks { x } else { (x - origin[0]) * scale });
            }
            20 | 21 => {
                let y = number()?;
                self.ys.push(if self.in_blocks { y } else { (y - origin[1]) * scale });
            }
            40 => self.radius = if self.in_blocks { number()? } else { number()? * scale },
            41 => self.ellipse_start = number()?,
            42 => self.ellipse_stop = number()?,
            50 => self.arc_start = number()?,
            51 => self.arc_stop = number()?,
            70 => self.flags = value.parse().map_err(|_| ())?,
            _ => {}
        }
        Ok(())
    }

    /// Chains the segments into paths: open ones (starting at an end no
    /// other segment touches), then closed ones. Returns point lists and
    /// whether each path is closed.
    fn paths(&mut self) -> Vec<(Vec<[f64; 2]>, bool)> {
        let mut enabled: BTreeSet<usize> = (0..self.lines.len()).collect();
        let mut paths = Vec::new();
        // Open paths.
        loop {
            let start = enabled.iter().find_map(|&idx| {
                (0..2).find(|&j| {
                    let cell = self.lines[idx].ends[j];
                    !self.grid.at(cell).iter().any(|&k| k != idx && self.lines.get(k).is_some_and(|l| !l.disabled))
                })
                .map(|j| (idx, j))
            });
            let Some((line, end)) = start else { break };
            paths.push((self.follow(line, end, &mut enabled), false));
        }
        // Closed paths.
        while let Some(&line) = enabled.iter().next() {
            paths.push((self.follow(line, 0, &mut enabled), true));
        }
        paths
    }

    /// The path from end `end` of `line`, through the segments meeting at
    /// each point.
    fn follow(&mut self, mut line: usize, mut end: usize, enabled: &mut BTreeSet<usize>) -> Vec<[f64; 2]> {
        let mut points = vec![point_of(self.lines[line].ends[end])];
        loop {
            let reached = self.lines[line].ends[1 - end];
            points.push(point_of(reached));
            self.lines[line].disabled = true;
            enabled.remove(&line);
            let next = self.grid.at(reached).iter().find_map(|&k| {
                let l = self.lines.get(k).filter(|l| !l.disabled)?;
                (0..2).find(|&j| l.ends[j] == reached).map(|j| (k, j))
            });
            match next {
                Some((k, j)) => (line, end) = (k, j),
                None => return points,
            }
        }
    }
}

/// The outlines of `text`, a DXF drawing.
pub(super) fn read(text: &str, name: &str, params: &DxfParams, warnings: &mut Vec<String>) -> Vec<Vec<[f64; 2]>> {
    let mut reader = Reader::new(params);
    let mut mode = String::new();
    let mut lines = text.lines().peekable();
    while let Some(code) = lines.next() {
        let value = lines.next().unwrap_or("").trim();
        let Ok(code) = code.trim().parse::<i32>() else {
            if lines.peek().is_some() {
                warnings.push(format!("Illegal ID '{}' in `{name}'", code.trim()));
            }
            break;
        };
        if code == 0 {
            reader.finish_entity(&mode);
            mode = value.to_string();
            reader.start_entity(&mode);
        } else if reader.group(code, value).is_err() {
            warnings.push(format!("Illegal value '{value}'in `{name}'"));
        }
    }
    for (entity, count) in &reader.unsupported {
        warnings.push(if params.layer.is_empty() {
            format!("Unsupported DXF Entity '{entity}' ({count:x}) in \"{name}\".")
        } else {
            format!("Unsupported DXF Entity '{entity}' ({count:x}) in layer '{}' of \"{name}\"", params.layer)
        });
    }
    reader
        .paths()
        .into_iter()
        .filter_map(|(mut points, closed)| {
            // A closed path ends where it starts.
            if closed {
                points.pop();
            }
            (points.len() > 2).then_some(points)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use c2f_eval::node::Segments;

    const SEGMENTS: Segments = Segments { fn_: 0.0, fa: 12.0, fs: 2.0 };

    fn dxf(entities: &str) -> String {
        format!("0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n")
    }

    fn line(a: [f64; 2], b: [f64; 2], layer: &str) -> String {
        format!("0\nLINE\n8\n{layer}\n10\n{}\n20\n{}\n11\n{}\n21\n{}\n", a[0], a[1], b[0], b[1])
    }

    fn outlines(text: &str, layer: &str, segments: Segments) -> (Vec<Vec<[f64; 2]>>, Vec<String>) {
        let params = DxfParams { layer, origin: [0.0, 0.0], scale: 1.0, segments };
        let mut warnings = Vec::new();
        (read(text, "t.dxf", &params, &mut warnings), warnings)
    }

    #[test]
    fn lines_chain_into_a_closed_outline() {
        let text = dxf(&(line([0.0, 0.0], [10.0, 0.0], "a")
            + &line([10.0, 5.0], [10.0, 0.0], "a")
            + &line([10.0, 5.0], [0.0, 5.0], "a")
            + &line([0.0, 5.0], [0.0, 0.0], "a")
            + &line([20.0, 0.0], [30.0, 0.0], "b")));
        let (paths, warnings) = outlines(&text, "a", SEGMENTS);
        assert!(warnings.is_empty());
        assert_eq!(paths, vec![vec![[0.0, 0.0], [10.0, 0.0], [10.0, 5.0], [0.0, 5.0]]]);
        // An open path of three points is closed by its last edge; a
        // single segment is dropped.
        let text = dxf(&(line([0.0, 0.0], [10.0, 0.0], "a") + &line([10.0, 0.0], [10.0, 5.0], "a") + &line([20.0, 0.0], [30.0, 0.0], "b")));
        let (paths, _) = outlines(&text, "", SEGMENTS);
        assert_eq!(paths, vec![vec![[0.0, 0.0], [10.0, 0.0], [10.0, 5.0]]]);
    }

    #[test]
    fn points_snap_to_the_grid_and_to_neighbours() {
        let text = dxf(&(line([0.0, 0.0], [10.00049, 0.0], "0")
            + &line([10.0004, 0.0004], [10.0, 5.0], "0")
            + &line([10.0, 5.0], [0.0, 5.0012], "0")
            + &line([0.0, 5.0], [0.0, 0.0], "0")));
        let (paths, _) = outlines(&text, "", SEGMENTS);
        // As the reference release (2021.01) exports this drawing.
        assert_eq!(paths, vec![vec![[0.0, 0.0], [10.0009765625, 0.0], [10.0, 5.0], [0.0, 5.0009765625]]]);
    }

    #[test]
    fn circles_and_arcs_follow_fn() {
        let segments = Segments { fn_: 12.0, ..SEGMENTS };
        let circle = "0\nCIRCLE\n8\n0\n10\n3\n20\n4\n40\n10\n";
        let (paths, _) = outlines(&dxf(circle), "", segments);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].len(), 12);
        // 270 to 90 degrees wraps: a half circle in 6 segments.
        let arc = "0\nARC\n8\n0\n10\n0\n20\n0\n40\n10\n50\n270\n51\n90\n".to_string() + &line([0.0, 10.0], [0.0, -10.0], "0");
        let (paths, _) = outlines(&dxf(&arc), "", segments);
        assert_eq!(paths[0].len(), 7);
    }

    #[test]
    fn ellipse_points() {
        let ellipse = "0\nELLIPSE\n8\n0\n10\n0\n20\n0\n11\n10\n21\n0\n40\n0.5\n41\n0\n42\n6.283185307179586\n";
        let (paths, _) = outlines(&dxf(ellipse), "", Segments { fn_: 12.0, ..SEGMENTS });
        assert_eq!(paths[0].len(), 12);
        let has = |p: [f64; 2]| paths[0].iter().any(|q| (q[0] - p[0]).abs() < 1e-3 && (q[1] - p[1]).abs() < 1e-3);
        assert!(has([8.660, 2.5]) && has([0.0, -5.0]) && has([-10.0, 0.0]));
    }

    #[test]
    fn inserted_blocks() {
        let block = "0\nBLOCK\n8\n0\n2\nB1\n".to_string()
            + &line([0.0, 0.0], [4.0, 0.0], "0")
            + &line([4.0, 0.0], [4.0, 2.0], "0")
            + &line([4.0, 2.0], [0.0, 2.0], "0")
            + &line([0.0, 2.0], [0.0, 0.0], "0")
            + "0\nENDBLK\n8\n0\n";
        let text = format!(
            "0\nSECTION\n2\nBLOCKS\n{block}0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nINSERT\n8\n0\n2\nB1\n10\n10\n20\n5\n41\n2\n42\n3\n50\n30\n0\nENDSEC\n0\nEOF\n"
        );
        let params = DxfParams { layer: "", origin: [1.0, 1.0], scale: 2.0, segments: SEGMENTS };
        let paths = read(&text, "t.dxf", &params, &mut Vec::new());
        assert_eq!(paths.len(), 1);
        // As the reference release (2021.01) places it.
        for p in [[18.0, 8.0], [31.8564, 16.0], [25.8564, 26.3926], [12.0, 18.3926]] {
            assert!(paths[0].iter().any(|q| (q[0] - p[0]).abs() < 1e-3 && (q[1] - p[1]).abs() < 1e-3), "{p:?} in {paths:?}");
        }
    }

    #[test]
    fn unsupported_entities_are_reported() {
        let polyline = "0\nPOLYLINE\n8\n0\n66\n1\n0\nVERTEX\n8\n0\n10\n0\n20\n0\n0\nVERTEX\n8\n0\n10\n1\n20\n0\n0\nSEQEND\n8\n0\n";
        let (paths, warnings) = outlines(&dxf(polyline), "", SEGMENTS);
        assert!(paths.is_empty());
        assert_eq!(
            warnings,
            [
                "Unsupported DXF Entity 'POLYLINE' (1) in \"t.dxf\".",
                "Unsupported DXF Entity 'SEQEND' (1) in \"t.dxf\".",
                "Unsupported DXF Entity 'VERTEX' (2) in \"t.dxf\"."
            ]
        );
    }
}
