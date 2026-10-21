//! Finding a font file from a fontconfig-style name such as
//! `"Liberation Sans:style=Bold Italic"`, the way the reference
//! implementation's fontconfig does for the fonts it bundles and the
//! system's.
//!
//! The bundled directory is searched first, then the system font
//! directories. Families compare ignoring case and spaces. A `style=`
//! must name a face's style exactly (ignoring case); otherwise, and when
//! it names none, the face closest in slant and then weight to the
//! request (regular upright by default) wins, as fontconfig's matching
//! does. An unknown family falls back to Liberation Sans.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// Where the reference release (2021.01) keeps the fonts it bundles;
/// `OPENSCAD_FONT_PATH` (colon-separated), then the studio app's own
/// copy, are searched before these.
const BUNDLED_DIRS: &[&str] = &[
    "/Applications/OpenSCAD-2021.01.app/Contents/Resources/fonts",
    "/Applications/OpenSCAD.app/Contents/Resources/fonts",
];

/// System font directories (macOS first), searched after the bundled ones.
const SYSTEM_DIRS: &[&str] = &["/System/Library/Fonts", "/Library/Fonts", "/usr/share/fonts", "/usr/local/share/fonts"];
const HOME_DIRS: &[&str] = &["Library/Fonts", ".fonts", ".local/share/fonts"];

pub const DEFAULT_FAMILY: &str = "Liberation Sans";

/// A loaded font file and the face to use in it.
pub struct Font {
    pub data: Arc<Vec<u8>>,
    pub index: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slant {
    Roman,
    Italic,
    Oblique,
}

/// One face of a font file, as found when scanning a directory.
struct Entry {
    path: PathBuf,
    index: u32,
    /// Family names, lowercase without spaces.
    families: Vec<String>,
    /// Style names, lowercase.
    styles: Vec<String>,
    weight: u16,
    slant: Slant,
}

/// A parsed font name.
#[derive(Debug, PartialEq)]
struct Request {
    families: Vec<String>,
    styles: Vec<String>,
    weight: Option<u16>,
    slant: Option<Slant>,
}

/// The font for `name` (the `font` parameter of `text()`; empty for the
/// default), falling back to Liberation Sans with a warning.
pub fn load(name: &str, warnings: &mut Vec<String>) -> Option<Font> {
    let mut request = parse_name(name);
    if request.families.is_empty() {
        request.families.push(DEFAULT_FAMILY.into());
    }
    let found = request.families.iter().find_map(|family| select(&request, &normalize(family)));
    let entry = match found {
        Some(entry) => entry,
        None => {
            let fallback = select(&request, &normalize(DEFAULT_FAMILY));
            match fallback {
                Some(entry) => {
                    warnings.push(format!("Can't find font \"{name}\", using {DEFAULT_FAMILY}"));
                    entry
                }
                None => {
                    warnings.push(format!(
                        "Can't find font \"{name}\" nor {DEFAULT_FAMILY}: install the Liberation fonts or set OPENSCAD_FONT_PATH"
                    ));
                    return None;
                }
            }
        }
    };
    let data = read_cached(&entry.path)?;
    Some(Font { data, index: entry.index })
}

/// The best face of `family`, searching the bundled fonts before the
/// system's.
fn select(request: &Request, family: &str) -> Option<&'static Entry> {
    best(bundled(), request, family).or_else(|| best(system(), request, family))
}

fn best<'a>(entries: &'a [Entry], request: &Request, family: &str) -> Option<&'a Entry> {
    let candidates: Vec<&Entry> = entries.iter().filter(|e| e.families.iter().any(|f| f == family)).collect();
    if let Some(entry) = candidates.iter().find(|e| e.styles.iter().any(|s| request.styles.contains(s))) {
        return Some(entry);
    }
    let weight = request.weight.unwrap_or(400);
    let slant = request.slant.unwrap_or(Slant::Roman);
    // Fontconfig ranks slant above weight; italic and oblique stand in for
    // each other before roman does.
    let slant_cost = |s: Slant| match (s, slant) {
        (a, b) if a == b => 0,
        (Slant::Roman, _) | (_, Slant::Roman) => 2,
        _ => 1,
    };
    candidates.into_iter().min_by_key(|e| (slant_cost(e.slant), e.weight.abs_diff(weight)))
}

/// Lowercase without blanks: fontconfig's family comparison.
fn normalize(family: &str) -> String {
    family.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect()
}

/// Splits a fontconfig name: `family[,family...][:property[=value]...]`,
/// with `\` escaping the separators.
fn parse_name(name: &str) -> Request {
    let mut parts: Vec<String> = vec![String::new()];
    let mut chars = name.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    parts.last_mut().expect("never empty").push(next);
                }
            }
            ':' => parts.push(String::new()),
            // Families are comma-separated: keep the commas, split below.
            ',' if parts.len() == 1 => parts[0].push('\u{0}'),
            c => parts.last_mut().expect("never empty").push(c),
        }
    }
    let families =
        parts[0].split('\u{0}').map(str::trim).filter(|f| !f.is_empty()).map(str::to_string).collect();
    let mut request = Request { families, styles: Vec::new(), weight: None, slant: None };
    for property in &parts[1..] {
        let (key, value) = match property.split_once('=') {
            Some((key, value)) => (key.trim().to_lowercase(), value.trim().to_lowercase()),
            None => (String::new(), property.trim().to_lowercase()),
        };
        match key.as_str() {
            "style" => request.styles.extend(value.split(',').map(|s| s.trim().to_string())),
            "weight" => request.weight = weight_value(&value).or(request.weight),
            "slant" => request.slant = slant_value(&value).or(request.slant),
            "" => {
                // A bare constant: `bold`, `italic`, ...
                if let Some(weight) = weight_name(&value) {
                    request.weight = Some(weight);
                } else if let Some(slant) = slant_value(&value) {
                    request.slant = Some(slant);
                }
            }
            _ => {}
        }
    }
    request
}

/// OpenType weights for fontconfig's weight names.
fn weight_name(name: &str) -> Option<u16> {
    Some(match name {
        "thin" => 100,
        "extralight" | "ultralight" => 200,
        "light" | "demilight" | "semilight" => 300,
        "book" | "regular" | "normal" => 400,
        "medium" => 500,
        "demibold" | "semibold" => 600,
        "bold" => 700,
        "extrabold" | "ultrabold" => 800,
        "black" | "heavy" => 900,
        _ => return None,
    })
}

/// A weight name, or a number on fontconfig's scale (80 regular, 200 bold).
fn weight_value(value: &str) -> Option<u16> {
    if let Some(weight) = weight_name(value) {
        return Some(weight);
    }
    let fc: f64 = value.parse().ok()?;
    const MAP: [(f64, f64); 9] =
        [(0.0, 100.0), (40.0, 200.0), (50.0, 300.0), (80.0, 400.0), (100.0, 500.0), (180.0, 600.0), (200.0, 700.0), (205.0, 800.0), (210.0, 900.0)];
    let fc = fc.clamp(0.0, 210.0);
    let i = MAP.iter().position(|&(f, _)| f >= fc).unwrap_or(MAP.len() - 1).max(1);
    let ((f0, o0), (f1, o1)) = (MAP[i - 1], MAP[i]);
    Some((o0 + (o1 - o0) * (fc - f0) / (f1 - f0)).round() as u16)
}

fn slant_value(value: &str) -> Option<Slant> {
    Some(match value {
        "roman" | "0" => Slant::Roman,
        "italic" | "100" => Slant::Italic,
        "oblique" | "110" => Slant::Oblique,
        _ => return None,
    })
}

fn bundled() -> &'static [Entry] {
    static BUNDLED: OnceLock<Vec<Entry>> = OnceLock::new();
    BUNDLED.get_or_init(|| {
        let mut dirs: Vec<PathBuf> = std::env::var_os("OPENSCAD_FONT_PATH")
            .map(|paths| std::env::split_paths(&paths).collect())
            .unwrap_or_default();
        dirs.extend(c2f_eval::app_resources().map(|resources| resources.join("fonts")));
        dirs.extend(BUNDLED_DIRS.iter().map(PathBuf::from));
        scan(&dirs)
    })
}

fn system() -> &'static [Entry] {
    static SYSTEM: OnceLock<Vec<Entry>> = OnceLock::new();
    SYSTEM.get_or_init(|| {
        let mut dirs: Vec<PathBuf> = SYSTEM_DIRS.iter().map(PathBuf::from).collect();
        if let Some(home) = std::env::var_os("HOME") {
            dirs.extend(HOME_DIRS.iter().map(|d| Path::new(&home).join(d)));
        }
        scan(&dirs)
    })
}

/// Every face of the font files under `dirs`, recursively.
fn scan(dirs: &[PathBuf]) -> Vec<Entry> {
    let mut files = Vec::new();
    for dir in dirs {
        collect_files(dir, 0, &mut files);
    }
    files.sort();
    files.dedup();
    files.iter().flat_map(|path| scan_file(path)).collect()
}

fn collect_files(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = read.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            if depth < 8 {
                collect_files(&path, depth + 1, out);
            }
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc" | "otc"))
        {
            out.push(path);
        }
    }
}

fn read_at(file: &mut File, offset: u64, len: usize) -> Option<Vec<u8>> {
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut buf = vec![0; len];
    file.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The faces of one file, reading only the table directories and the
/// `name` and `OS/2` tables (system font files can be huge).
fn scan_file(path: &Path) -> Vec<Entry> {
    let Ok(mut file) = File::open(path) else { return Vec::new() };
    let Some(header) = read_at(&mut file, 0, 12) else { return Vec::new() };
    let offsets: Vec<u32> = if &header[..4] == b"ttcf" {
        let count = be32(&header, 8).unwrap_or(0).min(256);
        let Some(table) = read_at(&mut file, 12, 4 * count as usize) else { return Vec::new() };
        (0..count as usize).filter_map(|i| be32(&table, 4 * i)).collect()
    } else {
        vec![0]
    };
    offsets
        .iter()
        .enumerate()
        .filter_map(|(index, &offset)| scan_face(&mut file, path, index as u32, offset.into()))
        .collect()
}

fn scan_face(file: &mut File, path: &Path, index: u32, offset: u64) -> Option<Entry> {
    let directory = read_at(file, offset, 12)?;
    let count = be16(&directory, 4)? as usize;
    let records = read_at(file, offset + 12, 16 * count)?;
    let mut tables = HashMap::new();
    for record in records.chunks_exact(16) {
        tables.insert(<[u8; 4]>::try_from(&record[..4]).ok()?, (be32(record, 8)?, be32(record, 12)?));
    }
    // Outline fonts only.
    if !(tables.contains_key(b"glyf") || tables.contains_key(b"CFF ") || tables.contains_key(b"CFF2")) {
        return None;
    }
    let &(name_offset, name_len) = tables.get(b"name")?;
    let name_data = read_at(file, name_offset.into(), name_len as usize)?;
    let names = ttf_parser::name::Table::parse(&name_data)?;
    let mut families = Vec::new();
    let mut styles = Vec::new();
    for name in names.names {
        // Some Apple fonts name their family on the Macintosh platform only.
        let mac_roman = name.platform_id == ttf_parser::PlatformId::Macintosh && name.encoding_id == 0;
        let Some(text) = name.to_string().or_else(|| mac_roman.then(|| decode_mac_roman(name.name))) else {
            continue;
        };
        match name.name_id {
            ttf_parser::name_id::FAMILY | ttf_parser::name_id::TYPOGRAPHIC_FAMILY => families.push(normalize(&text)),
            ttf_parser::name_id::SUBFAMILY | ttf_parser::name_id::TYPOGRAPHIC_SUBFAMILY => {
                styles.push(text.trim().to_lowercase())
            }
            _ => {}
        }
    }
    if families.is_empty() {
        return None;
    }
    families.sort();
    families.dedup();
    styles.sort();
    styles.dedup();
    let (mut weight, mut slant) = (400, Slant::Roman);
    if let Some(&(os2_offset, os2_len)) = tables.get(b"OS/2") {
        if let Some(os2) = read_at(file, os2_offset.into(), (os2_len as usize).min(64)) {
            weight = be16(&os2, 4).unwrap_or(400);
            let selection = be16(&os2, 62).unwrap_or(0);
            if selection & 0x200 != 0 {
                slant = Slant::Oblique;
            } else if selection & 1 != 0 {
                slant = Slant::Italic;
            }
        }
    }
    Some(Entry { path: path.to_path_buf(), index, families, styles, weight, slant })
}

/// The upper half of the Mac OS Roman encoding.
const MAC_ROMAN: &str = "ÄÅÇÉÑÖÜáàâäãåçéèêëíìîïñóòôöõúùûü†°¢£§•¶ß®©™´¨≠ÆØ∞±≤≥¥µ∂∑∏π∫ªºΩæø\
¿¡¬√ƒ≈∆«»…\u{a0}ÀÃÕŒœ–—“”‘’÷◊ÿŸ⁄€‹›ﬁﬂ‡·‚„‰ÂÊÁËÈÍÎÏÌÓÔ\u{f8ff}ÒÚÛÙıˆ˜¯˘˙˚¸˝˛ˇ";

fn decode_mac_roman(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| if b < 0x80 { char::from(b) } else { MAC_ROMAN.chars().nth(usize::from(b - 0x80)).unwrap_or('?') }).collect()
}

/// File contents, read once per process.
fn read_cached(path: &Path) -> Option<Arc<Vec<u8>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<Vec<u8>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(data) = cache.lock().ok()?.get(path) {
        return Some(Arc::clone(data));
    }
    let data = Arc::new(std::fs::read(path).ok()?);
    cache.lock().ok()?.insert(path.to_path_buf(), Arc::clone(&data));
    Some(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_parse_like_fontconfig() {
        let r = parse_name("Liberation Sans:style=Bold Italic");
        assert_eq!(r.families, ["Liberation Sans"]);
        assert_eq!(r.styles, ["bold italic"]);
        let r = parse_name("Foo\\:Bar, Liberation Serif:bold:slant=italic");
        assert_eq!(r.families, ["Foo:Bar", "Liberation Serif"]);
        assert_eq!((r.weight, r.slant), (Some(700), Some(Slant::Italic)));
        assert_eq!(parse_name("").families, Vec::<String>::new());
        assert_eq!(weight_value("200"), Some(700));
        assert_eq!(weight_value("80"), Some(400));
        assert_eq!(normalize("Liberation Sans"), normalize("liberationsans"));
    }

    #[test]
    fn mac_roman_names_decode() {
        assert_eq!(MAC_ROMAN.chars().count(), 128);
        assert_eq!(decode_mac_roman(b"Caf\x8e \xa9"), "Café ©");
    }
}
