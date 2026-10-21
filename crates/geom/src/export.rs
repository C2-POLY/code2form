//! 3MF export: the solid as one mesh object, in millimeters, in a 3MF
//! package (a ZIP archive of XML parts). SVG and DXF exports: the outlines
//! of a 2D model, laid out as the reference implementation (release
//! 2021.01) writes them.

use std::fmt::Write;

use manifold_rust::manifold::Manifold;

use crate::build::Model;
use crate::mesh::Rendered;

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
 <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
 <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
</Types>
"#;

const RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
 <Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>
</Relationships>
"#;

impl Rendered {
    /// The model as a 3MF package. `None` for an empty or 2D model.
    pub fn three_mf(&self) -> Option<Vec<u8>> {
        let Model::Solid(manifold) = &self.model else { return None };
        let (vertices, triangles) = indexed_mesh(manifold);
        if triangles.is_empty() {
            return None;
        }
        let model = model_xml(&vertices, &triangles);
        Some(zip(&[
            ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
            ("_rels/.rels", RELATIONSHIPS.as_bytes()),
            ("3D/3dmodel.model", model.as_bytes()),
        ]))
    }

    /// The 2D model as an SVG file, laid out as the reference writes it:
    /// one path with a subpath per outline (holes run the other way, so the
    /// default nonzero fill leaves them open), y pointing down, on a page of
    /// whole millimeters around the bounds. `None` for an empty or 3D model.
    ///
    /// Numbers use Rust's debug notation, the shortest that reads back to
    /// the same value, where the reference prints six significant digits.
    pub fn svg(&self) -> Option<String> {
        let (outlines, [lo_x, lo_y, hi_x, hi_y]) = flat_outlines(&self.model)?;
        // The page's coordinates, where y points down.
        let (min_x, min_y) = (lo_x.floor() as i64, (-hi_y).floor() as i64);
        let (max_x, max_y) = (hi_x.ceil() as i64, (-lo_y).ceil() as i64);
        let (width, height) = (max_x - min_x, max_y - min_y);
        let mut out = String::with_capacity(48 * outlines.iter().map(Vec::len).sum::<usize>() + 512);
        out.push_str(concat!(
            "<?xml version=\"1.0\" standalone=\"no\"?>\n",
            "<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\">\n",
        ));
        let _ = writeln!(
            out,
            "<svg width=\"{width}mm\" height=\"{height}mm\" viewBox=\"{min_x} {min_y} {width} {height}\" \
             xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\">"
        );
        out.push_str("<title>Code2Form Model</title>\n<path d=\"\n");
        for outline in &outlines {
            for (i, [x, y]) in outline.iter().enumerate() {
                let _ = write!(out, "{}{x:?},{:?}", if i == 0 { "M " } else { " L " }, unsigned_zero(-y));
                // Six points to a line, as the reference wraps them.
                if i % 6 == 5 {
                    out.push('\n');
                }
            }
            out.push_str(" z\n");
        }
        out.push_str("\" stroke=\"black\" fill=\"lightgray\" stroke-width=\"0.5\"/>\n</svg>\n");
        Some(out)
    }

    /// The 2D model as a DXF drawing, laid out as the reference writes it:
    /// a LINE on layer 0 for each edge of each outline, between the empty
    /// BLOCKS and OBJECTS sections that some readers (Inkscape) need.
    /// `None` for an empty or 3D model.
    pub fn dxf(&self) -> Option<String> {
        let (outlines, _) = flat_outlines(&self.model)?;
        let mut out = String::with_capacity(96 * outlines.iter().map(Vec::len).sum::<usize>() + 256);
        out.push_str("  0\nSECTION\n  2\nBLOCKS\n  0\nENDSEC\n  0\nSECTION\n  2\nENTITIES\n");
        for outline in &outlines {
            for (i, [x1, y1]) in outline.iter().enumerate() {
                let [x2, y2] = outline[(i + 1) % outline.len()];
                let _ = write!(out, "  0\nLINE\n  8\n0\n 10\n{x1:?}\n 20\n{y1:?}\n 11\n{x2:?}\n 21\n{y2:?}\n");
            }
        }
        out.push_str("  0\nENDSEC\n  0\nSECTION\n  2\nOBJECTS\n  0\nDICTIONARY\n  0\nENDSEC\n  0\nEOF\n");
        Some(out)
    }
}

/// The outlines of a non-empty 2D model, none of them empty, and their
/// bounds as `[min x, min y, max x, max y]`.
type Outlines = (Vec<Vec<[f64; 2]>>, [f64; 4]);

fn flat_outlines(model: &Model) -> Option<Outlines> {
    let Model::Flat(section) = model else { return None };
    let outlines: Vec<Vec<[f64; 2]>> = section
        .to_polygons()
        .into_iter()
        .filter(|outline| !outline.is_empty())
        .map(|outline| outline.into_iter().map(|p| [unsigned_zero(p.x), unsigned_zero(p.y)]).collect())
        .collect();
    if outlines.is_empty() {
        return None;
    }
    let b = section.bounds();
    Some((outlines, [b.min.x, b.min.y, b.max.x, b.max.y]))
}

/// `v`, with `-0.0` made `0.0`, which prints without a minus sign.
fn unsigned_zero(v: f64) -> f64 {
    v + 0.0
}

/// The solid's vertices and triangles, each vertex once. Colors are vertex
/// properties, so Manifold splits a vertex where two colors meet; its merge
/// lists say which copies are one vertex, and without them the mesh would
/// have open edges along every color boundary.
fn indexed_mesh(manifold: &Manifold) -> (Vec<[f64; 3]>, Vec<[usize; 3]>) {
    let mesh = manifold.get_mesh_gl64(-1);
    let stride = mesh.num_prop as usize;
    let count = mesh.vert_properties.len() / stride.max(1);
    let mut target: Vec<usize> = (0..count).collect();
    for (&from, &to) in mesh.merge_from_vert.iter().zip(&mesh.merge_to_vert) {
        target[from as usize] = to as usize;
    }
    // Numbered in order of first use, leaving out the merged copies.
    let mut number = vec![usize::MAX; count];
    let mut vertices = Vec::new();
    let mut corner = |i: u64| {
        let v = target[i as usize];
        if number[v] == usize::MAX {
            number[v] = vertices.len();
            let p = &mesh.vert_properties[v * stride..v * stride + 3];
            vertices.push([p[0], p[1], p[2]]);
        }
        number[v]
    };
    let triangles = mesh.tri_verts.chunks(3).map(|t| [corner(t[0]), corner(t[1]), corner(t[2])]).collect();
    (vertices, triangles)
}

/// The model part. Numbers use Rust's debug notation, the shortest that
/// reads back to the same value, which the 3MF number type allows.
fn model_xml(vertices: &[[f64; 3]], triangles: &[[usize; 3]]) -> String {
    let mut out = String::with_capacity(64 * (vertices.len() + triangles.len()) + 512);
    out.push_str(concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<model unit=\"millimeter\" xml:lang=\"en-US\" xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\">\n",
        " <metadata name=\"Application\">Code2Form</metadata>\n",
        " <resources>\n",
        "  <object id=\"1\" type=\"model\">\n",
        "   <mesh>\n",
        "    <vertices>\n",
    ));
    for [x, y, z] in vertices {
        let _ = writeln!(out, "     <vertex x=\"{x:?}\" y=\"{y:?}\" z=\"{z:?}\"/>");
    }
    out.push_str("    </vertices>\n    <triangles>\n");
    for [a, b, c] in triangles {
        let _ = writeln!(out, "     <triangle v1=\"{a}\" v2=\"{b}\" v3=\"{c}\"/>");
    }
    out.push_str(concat!(
        "    </triangles>\n",
        "   </mesh>\n",
        "  </object>\n",
        " </resources>\n",
        " <build>\n",
        "  <item objectid=\"1\"/>\n",
        " </build>\n",
        "</model>\n",
    ));
    out
}

/// A ZIP archive of deflated entries, dated 1980-01-01 so that the same
/// model always gives the same file. Without ZIP64, each part must stay
/// under 4 GiB, some 40 million triangles.
fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let size = |n: usize| u32::try_from(n).expect("3MF parts are limited to 4 GiB");
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, content) in files {
        let offset = size(out.len());
        let deflated = miniz_oxide::deflate::compress_to_vec(content, 6);
        let mut fields = Vec::new();
        fields.extend(20u16.to_le_bytes()); // version needed: 2.0
        fields.extend(0u16.to_le_bytes()); // flags
        fields.extend(8u16.to_le_bytes()); // method: deflate
        fields.extend(0u16.to_le_bytes()); // time: 00:00
        fields.extend(0x21u16.to_le_bytes()); // date: 1980-01-01
        fields.extend(crc32(content).to_le_bytes());
        fields.extend(size(deflated.len()).to_le_bytes());
        fields.extend(size(content.len()).to_le_bytes());
        fields.extend((name.len() as u16).to_le_bytes());
        fields.extend(0u16.to_le_bytes()); // extra field length

        out.extend(b"PK\x03\x04");
        out.extend(&fields);
        out.extend(name.as_bytes());
        out.extend(&deflated);

        central.extend(b"PK\x01\x02");
        central.extend(20u16.to_le_bytes()); // made by: 2.0
        central.extend(&fields);
        central.extend(0u16.to_le_bytes()); // comment length
        central.extend(0u16.to_le_bytes()); // disk
        central.extend(0u16.to_le_bytes()); // internal attributes
        central.extend(0u32.to_le_bytes()); // external attributes
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let start = size(out.len());
    let count = (files.len() as u16).to_le_bytes();
    out.extend(&central);
    out.extend(b"PK\x05\x06");
    out.extend(0u32.to_le_bytes()); // this disk, the directory's disk
    out.extend(count);
    out.extend(count);
    out.extend(size(central.len()).to_le_bytes());
    out.extend(start.to_le_bytes());
    out.extend(0u16.to_le_bytes()); // comment length
    out
}

/// The CRC-32 that ZIP stores for each entry (IEEE, reflected).
fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (n, entry) in table.iter_mut().enumerate() {
        let mut c = n as u32;
        for _ in 0..8 {
            c = if c & 1 == 1 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *entry = c;
    }
    !data.iter().fold(!0u32, |c, &b| table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    fn solid(src: &str) -> Manifold {
        let program = c2f_eval::Program::load(src, std::path::Path::new("t.scad"), &[]).expect("parses");
        match crate::render(&c2f_eval::evaluate(&program).nodes).model {
            Model::Solid(manifold) => manifold,
            _ => panic!("{src} is not a solid"),
        }
    }

    #[test]
    fn color_boundaries_stay_closed() {
        // Two colors meet along the seam, where Manifold splits vertices.
        let manifold = solid("color(\"red\") cube(10); color(\"blue\") translate([10, 0, 0]) cube(10);");
        let mesh = manifold.get_mesh_gl64(-1);
        assert!(!mesh.merge_from_vert.is_empty(), "the test needs split vertices");
        let (vertices, triangles) = indexed_mesh(&manifold);
        assert_eq!(triangles.len(), manifold.num_tri());
        assert_eq!(vertices.len(), manifold.num_vert());
        // Closed: each edge runs once in each direction.
        let mut edges = std::collections::HashMap::new();
        for [a, b, c] in &triangles {
            for edge in [(*a, *b), (*b, *c), (*c, *a)] {
                *edges.entry(edge).or_insert(0) += 1;
            }
        }
        for (&(a, b), &count) in &edges {
            assert_eq!((count, edges.get(&(b, a))), (1, Some(&1)), "edge {a}-{b}");
        }
    }

    #[test]
    fn zip_entries_read_back() {
        let archive = zip(&[("a.txt", b"hello hello hello"), ("dir/b", b"")]);
        let entries = crate::import::zip::entries(&archive).unwrap();
        assert_eq!(entries, vec![("a.txt".to_string(), b"hello hello hello".to_vec()), ("dir/b".to_string(), Vec::new())]);
    }
}
