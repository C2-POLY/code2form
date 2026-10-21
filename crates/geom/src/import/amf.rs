//! AMF, plain or zipped: the triangles of every `object`'s mesh volumes,
//! one mesh per object (the reference implementation unions them).
//! Units, materials and constellations are ignored, as the reference
//! release (2021.01) does.

use super::{MeshBuilder, xml, zip};
use crate::primitives::TriMesh;

pub(super) fn read(data: &[u8], name: &str, warnings: &mut Vec<String>) -> Option<Vec<TriMesh>> {
    let unzipped;
    let data = if data.starts_with(b"PK\x03\x04") {
        let entries = match zip::entries(data) {
            Ok(entries) => entries,
            Err(e) => {
                warnings.push(format!("Can't read zipped AMF '{name}': {e}"));
                return None;
            }
        };
        let entry = entries.iter().position(|(n, _)| n.to_ascii_lowercase().ends_with(".amf")).unwrap_or(0);
        let Some((_, content)) = entries.into_iter().nth(entry) else {
            warnings.push(format!("Can't read file from zipped AMF '{name}'"));
            return None;
        };
        unzipped = content;
        &unzipped[..]
    } else {
        data
    };
    let root = match xml::parse(&String::from_utf8_lossy(data)) {
        Ok(root) if root.name == "amf" => root,
        Ok(_) => {
            warnings.push(format!("'{name}' is not an AMF file"));
            return None;
        }
        Err(e) => {
            warnings.push(format!("Failed to parse file '{name}': {e}"));
            return None;
        }
    };
    let mut meshes = Vec::new();
    for object in root.named("object") {
        let mut mesh = MeshBuilder::default();
        let mut ids = Vec::new();
        for m in object.named("mesh") {
            let base = ids.len();
            for vertex in m.named("vertices").flat_map(|v| v.named("vertex")) {
                let coordinate = |axis: &str| {
                    vertex.child("coordinates").and_then(|c| c.child(axis)).and_then(|e| e.text.trim().parse::<f64>().ok())
                };
                let p = [coordinate("x").unwrap_or(0.0), coordinate("y").unwrap_or(0.0), coordinate("z").unwrap_or(0.0)];
                ids.push(mesh.vertex(p));
            }
            for triangle in m.named("volume").flat_map(|v| v.named("triangle")) {
                let corner = |key: &str| {
                    triangle.child(key).and_then(|e| e.text.trim().parse::<usize>().ok()).and_then(|i| ids.get(base + i).copied())
                };
                match (corner("v1"), corner("v2"), corner("v3")) {
                    (Some(a), Some(b), Some(c)) => mesh.triangle(a, b, c),
                    _ => warnings.push(format!("AMF file '{name}' has a bad triangle")),
                }
            }
        }
        meshes.extend(mesh.finish());
    }
    (!meshes.is_empty()).then_some(meshes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TETRA: &str = r#"<?xml version="1.0"?><amf unit="millimeter"><object id="0"><mesh><vertices>
        <vertex><coordinates><x>0</x><y>0</y><z>0</z></coordinates></vertex>
        <vertex><coordinates><x>1</x><y>0</y><z>0</z></coordinates></vertex>
        <vertex><coordinates><x>0</x><y>1</y><z>0</z></coordinates></vertex>
        <vertex><coordinates><x>0</x><y>0</y><z>1</z></coordinates></vertex>
        </vertices><volume>
        <triangle><v1>0</v1><v2>2</v2><v3>1</v3></triangle><triangle><v1>0</v1><v2>1</v2><v3>3</v3></triangle>
        <triangle><v1>0</v1><v2>3</v2><v3>2</v3></triangle><triangle><v1>1</v1><v2>2</v2><v3>3</v3></triangle>
        </volume></mesh></object></amf>"#;

    #[test]
    fn plain_and_zipped() {
        let plain = read(TETRA.as_bytes(), "t.amf", &mut Vec::new()).unwrap();
        assert_eq!(plain.len(), 1);
        assert_eq!(plain[0].triangles.len(), 4);
        let zipped = zip::tests::stored(&[("t.amf", TETRA.as_bytes())]);
        let unzipped = read(&zipped, "t.amf", &mut Vec::new()).unwrap();
        assert_eq!(unzipped[0].positions, plain[0].positions);
    }
}
