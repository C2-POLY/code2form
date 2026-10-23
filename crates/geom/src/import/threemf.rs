//! 3MF: the ZIP package's model parts (`*.model`), one mesh per mesh
//! object, in the objects' own coordinates. As in the reference
//! implementation (2021.01, which lists the mesh objects with lib3mf),
//! build items, their transforms and component objects are not applied.
//! `object` (not in the reference) keeps the objects of that name.

use super::{MeshBuilder, xml, zip};
use crate::primitives::TriMesh;

pub(super) fn read(data: &[u8], name: &str, object: &str, warnings: &mut Vec<String>) -> Option<Vec<TriMesh>> {
    let entries = match zip::entries(data) {
        Ok(entries) => entries,
        Err(e) => {
            warnings.push(format!("Could not read file '{name}': {e}"));
            return None;
        }
    };
    let mut meshes = Vec::new();
    for (_, content) in entries.iter().filter(|(n, _)| n.to_ascii_lowercase().ends_with(".model")) {
        let model = match xml::parse(&String::from_utf8_lossy(content)) {
            Ok(model) => model,
            Err(e) => {
                warnings.push(format!("Failed to parse file '{name}': {e}"));
                return None;
            }
        };
        for found in model.descendants("object") {
            if !object.is_empty() && found.attr("name") != Some(object) {
                continue;
            }
            for m in found.named("mesh") {
                let mut mesh = MeshBuilder::default();
                let mut ids = Vec::new();
                for vertex in m.named("vertices").flat_map(|v| v.named("vertex")) {
                    let c = |axis: &str| vertex.attr(axis).and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(0.0);
                    ids.push(mesh.vertex([c("x"), c("y"), c("z")]));
                }
                for triangle in m.named("triangles").flat_map(|t| t.named("triangle")) {
                    let corner = |key: &str| triangle.attr(key).and_then(|v| v.trim().parse::<usize>().ok()).and_then(|i| ids.get(i).copied());
                    match (corner("v1"), corner("v2"), corner("v3")) {
                        (Some(a), Some(b), Some(c)) => mesh.triangle(a, b, c),
                        _ => warnings.push(format!("3MF file '{name}' has a bad triangle")),
                    }
                }
                meshes.extend(mesh.finish());
            }
        }
    }
    if meshes.is_empty() {
        if object.is_empty() {
            warnings.push(format!("No mesh found in 3MF file '{name}'"));
        } else {
            warnings.push(super::no_such_object(name, object, &objects(data)));
        }
        return None;
    }
    Some(meshes)
}

/// The names of the mesh objects, in order, those without one left out.
pub(super) fn objects(data: &[u8]) -> Vec<String> {
    let Ok(entries) = zip::entries(data) else { return Vec::new() };
    let mut names = Vec::new();
    for (_, content) in entries.iter().filter(|(n, _)| n.to_ascii_lowercase().ends_with(".model")) {
        let Ok(model) = xml::parse(&String::from_utf8_lossy(content)) else { continue };
        for object in model.descendants("object") {
            if let Some(name) = object.attr("name").filter(|_| object.child("mesh").is_some()) {
                names.push(name.to_string());
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_objects() {
        let model = r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">
 <resources><object id="1" type="model"><mesh>
  <vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/><vertex x="0" y="0" z="1"/></vertices>
  <triangles><triangle v1="0" v2="2" v3="1"/><triangle v1="0" v2="1" v3="3"/><triangle v1="0" v2="3" v3="2"/><triangle v1="1" v2="2" v3="3"/></triangles>
 </mesh></object></resources>
 <build><item objectid="1"/></build>
</model>"#;
        let package = zip::tests::stored(&[("[Content_Types].xml", b"<Types/>"), ("3D/3dmodel.model", model.as_bytes())]);
        let meshes = read(&package, "t.3mf", "", &mut Vec::new()).unwrap();
        assert_eq!(meshes.len(), 1);
        assert_eq!(meshes[0].triangles.len(), 4);
        assert_eq!(meshes[0].positions[3], [0.0, 0.0, 1.0]);
    }

    #[test]
    fn one_object_by_its_name() {
        let tetra = |id: u32, name: &str| {
            format!(
                r#"<object id="{id}" type="model" name="{name}"><mesh>
  <vertices><vertex x="{id}" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/><vertex x="0" y="0" z="1"/></vertices>
  <triangles><triangle v1="0" v2="2" v3="1"/><triangle v1="0" v2="1" v3="3"/><triangle v1="0" v2="3" v3="2"/><triangle v1="1" v2="2" v3="3"/></triangles>
 </mesh></object>"#
            )
        };
        let model = format!(
            r#"<?xml version="1.0"?><model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources>{}{}</resources></model>"#,
            tetra(0, "Lid"),
            tetra(2, "Box")
        );
        let package = zip::tests::stored(&[("3D/3dmodel.model", model.as_bytes())]);
        assert_eq!(objects(&package), ["Lid", "Box"]);
        let meshes = read(&package, "t.3mf", "Box", &mut Vec::new()).unwrap();
        assert_eq!((meshes.len(), meshes[0].positions[0]), (1, [2.0, 0.0, 0.0]));
        let mut warnings = Vec::new();
        assert!(read(&package, "t.3mf", "Hinge", &mut warnings).is_none());
        assert!(warnings[0].contains("it holds Lid, Box"), "{warnings:?}");
    }
}
