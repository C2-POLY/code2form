//! The two legacy functions that read measurements out of a DXF drawing:
//! `dxf_dim` (the value of a named DIMENSION) and `dxf_cross` (where two
//! LINEs of a layer cross).

use std::path::Path;

/// A DXF entity: its type and its group-code/value pairs.
struct Entity {
    kind: String,
    groups: Vec<(i32, String)>,
}

impl Entity {
    fn text(&self, code: i32) -> Option<&str> {
        self.groups.iter().find(|(c, _)| *c == code).map(|(_, v)| v.as_str())
    }

    fn number(&self, code: i32) -> Option<f64> {
        self.text(code)?.trim().parse().ok()
    }

    /// Point `n` (group codes 10+n / 20+n), moved by `origin` and scaled.
    fn point(&self, n: i32, origin: [f64; 2], scale: f64) -> [f64; 2] {
        let x = self.number(10 + n).unwrap_or(0.0);
        let y = self.number(20 + n).unwrap_or(0.0);
        [(x - origin[0]) * scale, (y - origin[1]) * scale]
    }

    fn on_layer(&self, layer: &str) -> bool {
        layer.is_empty() || self.text(8) == Some(layer)
    }
}

fn read_entities(path: &Path) -> Result<Vec<Entity>, String> {
    let text = crate::program::read_source(path).map_err(|e| format!("Can't open DXF file '{}': {e}", path.display()))?;
    let mut lines = text.lines();
    let mut entities: Vec<Entity> = Vec::new();
    while let (Some(code), Some(value)) = (lines.next(), lines.next()) {
        let Ok(code) = code.trim().parse::<i32>() else { continue };
        let value = value.trim().to_string();
        if code == 0 {
            entities.push(Entity { kind: value, groups: Vec::new() });
        } else if let Some(entity) = entities.last_mut() {
            entity.groups.push((code, value));
        }
    }
    Ok(entities)
}

/// The measurement of the DIMENSION called `name` (any name when empty).
pub fn dim(path: &Path, name: &str, layer: &str, origin: [f64; 2], scale: f64) -> Result<f64, String> {
    for d in read_entities(path)?.iter().filter(|e| e.kind == "DIMENSION" && e.on_layer(layer)) {
        if !name.is_empty() && d.text(1) != Some(name) {
            continue;
        }
        let p = |n| d.point(n, origin, scale);
        let kind = d.number(70).unwrap_or(0.0) as i64;
        let value = match kind & 7 {
            // Linear: the distance between points 3 and 4 along the angle.
            0 => {
                let ([x3, y3], [x4, y4]) = (p(3), p(4));
                let angle = d.number(50).unwrap_or(0.0).to_radians();
                ((x4 - x3) * angle.cos() + (y4 - y3) * angle.sin()).abs()
            }
            // Aligned.
            1 => {
                let ([x3, y3], [x4, y4]) = (p(3), p(4));
                ((x4 - x3).powi(2) + (y4 - y3).powi(2)).sqrt()
            }
            // Angular: between lines 0-5 and 3-4.
            2 => {
                let ([x0, y0], [x3, y3], [x4, y4], [x5, y5]) = (p(0), p(3), p(4), p(5));
                ((x0 - x5).atan2(y0 - y5) - (x4 - x3).atan2(y4 - y3)).abs().to_degrees()
            }
            // Diameter and radius.
            3 | 4 => {
                let ([x0, y0], [x5, y5]) = (p(0), p(5));
                ((x5 - x0).powi(2) + (y5 - y0).powi(2)).sqrt()
            }
            // Ordinate: x or y of point 3.
            6 => {
                let [x3, y3] = p(3);
                if kind & 64 != 0 { x3 } else { y3 }
            }
            _ => return Err(format!("Unsupported DXF dimension type {} for '{name}'", kind & 7)),
        };
        return Ok(value);
    }
    Err(format!("Can't find dimension '{name}' in '{}', layer '{layer}'", path.display()))
}

/// Where the first two crossing LINEs of `layer` meet.
pub fn cross(path: &Path, layer: &str, origin: [f64; 2], scale: f64) -> Result<[f64; 2], String> {
    let entities = read_entities(path)?;
    let lines: Vec<[[f64; 2]; 2]> = entities
        .iter()
        .filter(|e| e.kind == "LINE" && e.on_layer(layer))
        .map(|e| [e.point(0, origin, scale), e.point(1, origin, scale)])
        .collect();
    for (i, [[x1, y1], [x2, y2]]) in lines.iter().enumerate() {
        for [[x3, y3], [x4, y4]] in &lines[i + 1..] {
            let denominator = (y4 - y3) * (x2 - x1) - (x4 - x3) * (y2 - y1);
            if denominator.abs() < 1e-12 {
                continue;
            }
            let ua = ((x4 - x3) * (y1 - y3) - (y4 - y3) * (x1 - x3)) / denominator;
            return Ok([x1 + ua * (x2 - x1), y1 + ua * (y2 - y1)]);
        }
    }
    Err(format!("Can't find cross in '{}', layer '{layer}'", path.display()))
}
