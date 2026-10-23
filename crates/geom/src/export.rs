//! 3MF export: the solid as one mesh object, in millimeters, in a 3MF
//! package (a ZIP archive of XML parts), with the print's pauses for
//! PrusaSlicer. SVG and DXF exports: the outlines
//! of a 2D model, laid out as the reference implementation (release
//! 2021.01) writes them.

use std::fmt::Write;

use manifold_rust::cross_section::CrossSection;
use manifold_rust::linalg::{Mat3x4, Vec3};
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
        let mut files = vec![
            ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
            ("_rels/.rels", RELATIONSHIPS.as_bytes()),
            ("3D/3dmodel.model", model.as_bytes()),
        ];
        let pauses = pauses_xml(&self.pauses, manifold.bounding_box().min.z);
        if let Some(pauses) = &pauses {
            files.push((PRUSA_PAUSES, pauses.as_bytes()));
            files.push((PRUSA_CONFIG, PRUSA_NO_SETTINGS.as_bytes()));
        }
        Some(zip(&files))
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
        let Model::Flat(section) = &self.model else { return None };
        let (outlines, _) = section_outlines(section)?;
        Some(dxf_lines(&outlines))
    }

    /// The model seen from `yaw` and `pitch` (degrees, as in
    /// [`crate::VIEWS`]) as a DXF drawing: its silhouette projected
    /// straight along the view, at its real size, x to the view's right
    /// and y up it. Angles within half a degree of a quarter turn are taken
    /// as that turn, so that a view from the top is exactly that. A 2D
    /// model is drawn as it is. `None` for an empty model.
    pub fn view_dxf(&self, yaw: f64, pitch: f64) -> Option<String> {
        let section = match &self.model {
            Model::Solid(manifold) => {
                let [right, up, back] = view_basis(yaw, pitch);
                let col = |j: usize| Vec3::new(right[j], up[j], back[j]);
                manifold.transform(&Mat3x4::from_cols(col(0), col(1), col(2), Vec3::new(0.0, 0.0, 0.0))).project()
            }
            Model::Flat(section) => section.clone(),
            Model::Empty => return None,
        };
        let (outlines, _) = section_outlines(&section)?;
        Some(dxf_lines(&outlines))
    }
}

/// The view's right, up and back (towards the viewer) directions, in the
/// app's convention: yaw about Z from +X, pitch above the XY plane.
fn view_basis(yaw: f64, pitch: f64) -> [[f64; 3]; 3] {
    let snap = |degrees: f64| {
        let turn = (degrees / 90.0).round() * 90.0;
        if (degrees - turn).abs() <= 0.5 { turn } else { degrees }
    };
    let (yaw, pitch) = (snap(yaw).to_radians(), snap(pitch).clamp(-90.0, 90.0).to_radians());
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let back = [cp * cy, cp * sy, sp];
    let forward = back.map(|c| -c);
    // forward × Z, made of length 1: still the yaw's when looking straight
    // down, as the app keeps it.
    let right = [-sy, cy, 0.0];
    // right × forward
    let up = [
        right[1] * forward[2] - right[2] * forward[1],
        right[2] * forward[0] - right[0] * forward[2],
        right[0] * forward[1] - right[1] * forward[0],
    ];
    [right, up, back]
}

/// A LINE on layer 0 for each edge of each outline.
fn dxf_lines(outlines: &[Vec<[f64; 2]>]) -> String {
    let mut out = String::with_capacity(96 * outlines.iter().map(Vec::len).sum::<usize>() + 256);
    out.push_str("  0\nSECTION\n  2\nBLOCKS\n  0\nENDSEC\n  0\nSECTION\n  2\nENTITIES\n");
    for outline in outlines {
        for (i, [x1, y1]) in outline.iter().enumerate() {
            let [x2, y2] = outline[(i + 1) % outline.len()];
            let _ = write!(out, "  0\nLINE\n  8\n0\n 10\n{x1:?}\n 20\n{y1:?}\n 11\n{x2:?}\n 21\n{y2:?}\n");
        }
    }
    out.push_str("  0\nENDSEC\n  0\nSECTION\n  2\nOBJECTS\n  0\nDICTIONARY\n  0\nENDSEC\n  0\nEOF\n");
    out
}

/// The outlines of a non-empty 2D model, none of them empty, and their
/// bounds as `[min x, min y, max x, max y]`.
type Outlines = (Vec<Vec<[f64; 2]>>, [f64; 4]);

fn flat_outlines(model: &Model) -> Option<Outlines> {
    let Model::Flat(section) = model else { return None };
    section_outlines(section)
}

fn section_outlines(section: &CrossSection) -> Option<Outlines> {
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
    let mesh = colored_mesh(manifold);
    (mesh.vertices, mesh.triangles)
}

/// A solid's vertices, each once, and its triangles with their colors.
pub(crate) struct ColoredMesh {
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[usize; 3]>,
    /// Each triangle's color, [`crate::DEFAULT_COLOR`] where none is set.
    pub colors: Vec<[f32; 4]>,
}

/// The solid's vertices, merged where colors split them (see
/// [`indexed_mesh`]), numbered in order of first use, and its triangles.
pub(crate) fn colored_mesh(manifold: &Manifold) -> ColoredMesh {
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
    let colors = mesh
        .tri_verts
        .chunks(3)
        .map(|t| match &mesh.vert_properties[t[0] as usize * stride..] {
            p if stride >= 7 => [p[3] as f32, p[4] as f32, p[5] as f32, p[6] as f32],
            _ => crate::DEFAULT_COLOR,
        })
        .collect();
    ColoredMesh { vertices, triangles, colors }
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

/// The part where PrusaSlicer keeps the G-code it inserts between layers.
const PRUSA_PAUSES: &str = "Metadata/Prusa_Slicer_custom_gcode_per_print_z.xml";

/// The part where PrusaSlicer keeps a project's settings. With it, a file
/// opened from the Finder or dropped on the window can open as a project,
/// which keeps the pauses; without it, PrusaSlicer imports the geometry
/// alone.
const PRUSA_CONFIG: &str = "Metadata/Slic3r_PE.config";

/// Settings are `; key = value` lines; a comment without `=` sets none, so
/// the printer, print and filament settings in use stay as they are.
const PRUSA_NO_SETTINGS: &str = "; Code2Form sets no settings: this part only lets PrusaSlicer open the file as a project, with its print pauses.\n";

/// PrusaSlicer pauses before the first layer whose top reaches a pause's
/// `print_z`. Just above the height asked for, that is the layer which
/// covers it, whatever the layer height, when the height is the top of a
/// layer.
const PAUSE_ABOVE: f64 = 0.01;

/// The pauses as PrusaSlicer reads them (`M601`, with the message shown on
/// the printer), or `None` when there are none. PrusaSlicer sets the
/// model's bottom on the bed, so heights count from `bottom`.
fn pauses_xml(pauses: &[c2f_eval::Pause], bottom: f64) -> Option<String> {
    if pauses.is_empty() {
        return None;
    }
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<custom_gcodes_per_print_z>\n");
    for pause in pauses {
        // Rounded to a micrometer, so that 19.4 - 0 + 0.01 reads 19.41.
        let print_z = ((pause.z - bottom + PAUSE_ABOVE) * 1e6).round() / 1e6;
        let _ = writeln!(
            out,
            "<code print_z=\"{print_z:?}\" type=\"1\" extruder=\"1\" color=\"\" extra=\"{}\" gcode=\"M601\"/>",
            xml_escape(&pause.message)
        );
    }
    out.push_str("<mode value=\"SingleExtruder\"/>\n</custom_gcodes_per_print_z>\n");
    Some(out)
}

fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\n' => out.push_str("&#10;"),
            c => out.push(c),
        }
    }
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
pub(crate) fn crc32(data: &[u8]) -> u32 {
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

    fn pauses_part(src: &str) -> (Option<String>, Vec<String>) {
        let program = c2f_eval::Program::load(src, std::path::Path::new("t.scad"), &[]).expect("parses");
        let output = c2f_eval::evaluate(&program);
        let rendered = crate::render(&output.nodes).with_pauses(output.pauses);
        let entries = crate::import::zip::entries(&rendered.three_mf().unwrap()).unwrap();
        let config = entries.iter().any(|(name, _)| name == PRUSA_CONFIG);
        let part = entries.into_iter().find(|(name, _)| name == PRUSA_PAUSES);
        assert_eq!(config, part.is_some(), "the settings part comes with the pauses");
        (part.map(|(_, xml)| String::from_utf8(xml).unwrap()), rendered.warnings)
    }

    #[test]
    fn pauses_count_from_the_bed_just_above_the_height_asked() {
        // PrusaSlicer sets the bottom, here at z = -5, on the bed.
        let (xml, warnings) = pauses_part(
            "translate([0, 0, -5]) cube(20);
            pause(14.4, \"Box <magnets> & \\\"nuts\\\"\");
            pause(-2);
            pause(-2, \"twice\");
            pause(-5);
            pause(15);",
        );
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("z = -5"), "{warnings:?}");
        assert_eq!(
            xml.unwrap(),
            concat!(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<custom_gcodes_per_print_z>\n",
                "<code print_z=\"3.01\" type=\"1\" extruder=\"1\" color=\"\" extra=\"\" gcode=\"M601\"/>\n",
                "<code print_z=\"19.41\" type=\"1\" extruder=\"1\" color=\"\" ",
                "extra=\"Box &lt;magnets&gt; &amp; &quot;nuts&quot;\" gcode=\"M601\"/>\n",
                "<mode value=\"SingleExtruder\"/>\n</custom_gcodes_per_print_z>\n",
            )
        );
    }

    #[test]
    fn the_settings_part_sets_nothing() {
        // PrusaSlicer reads a setting from a line `; key = value`.
        assert!(PRUSA_NO_SETTINGS.starts_with("; ") && !PRUSA_NO_SETTINGS.contains('='));
    }

    #[test]
    fn no_pauses_no_pauses_part() {
        assert_eq!(pauses_part("cube(5);"), (None, Vec::new()));
    }

    fn view(src: &str, yaw: f64, pitch: f64) -> Vec<[f64; 4]> {
        let program = c2f_eval::Program::load(src, std::path::Path::new("t.scad"), &[]).expect("parses");
        let dxf = crate::render(&c2f_eval::evaluate(&program).nodes).view_dxf(yaw, pitch).expect("a drawing");
        let values: Vec<&str> = dxf.lines().collect();
        values
            .windows(11)
            .filter(|w| w[0] == "LINE")
            .map(|w| [w[4], w[6], w[8], w[10]].map(|v| v.parse().unwrap()))
            .collect()
    }

    fn extent(lines: &[[f64; 4]]) -> [f64; 4] {
        lines.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |[a, b, c, d], l| {
            [a.min(l[0]).min(l[2]), b.min(l[1]).min(l[3]), c.max(l[0]).max(l[2]), d.max(l[1]).max(l[3])]
        })
    }

    #[test]
    fn a_view_from_the_top_is_the_outline_at_its_size() {
        let lines = view("cube([10, 20, 30]);", -90.0, 90.0);
        assert_eq!(lines.len(), 4);
        assert_eq!(extent(&lines), [0.0, 0.0, 10.0, 20.0]);
        // The app's top view stops half a degree short: the same drawing.
        assert_eq!(extent(&view("cube([10, 20, 30]);", -90.0, 89.5)), [0.0, 0.0, 10.0, 20.0]);
    }

    #[test]
    fn a_view_from_the_front_is_in_x_and_z() {
        let lines = view("cube([10, 20, 30]);", -90.0, 0.0);
        let [x0, y0, x1, y1] = extent(&lines);
        assert!((x1 - x0 - 10.0).abs() < 1e-9 && (y1 - y0 - 30.0).abs() < 1e-9, "{:?}", extent(&lines));
    }

    #[test]
    fn a_hole_through_the_view_stays_a_hole() {
        let lines = view("difference() { cube(10); translate([5, 5, -1]) cylinder(r = 2, h = 12, $fn = 6); }", -90.0, 90.0);
        assert_eq!(lines.len(), 4 + 6);
    }

    #[test]
    fn zip_entries_read_back() {
        let archive = zip(&[("a.txt", b"hello hello hello"), ("dir/b", b"")]);
        let entries = crate::import::zip::entries(&archive).unwrap();
        assert_eq!(entries, vec![("a.txt".to_string(), b"hello hello hello".to_vec()), ("dir/b".to_string(), Vec::new())]);
    }
}
