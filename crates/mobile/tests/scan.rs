//! A scan set straight by the app, as the studio sets it straight.

use c2f_mobile::level_scan;

/// A 60 × 40 × 20 mm box, turned 30° about Z, as a binary STL.
fn turned_box() -> Vec<u8> {
    let (s, c) = 30f64.to_radians().sin_cos();
    let corner = |i: usize| {
        let p = [if i & 1 == 1 { 60.0 } else { 0.0 }, if i & 2 == 2 { 40.0 } else { 0.0 }, if i & 4 == 4 { 20.0 } else { 0.0 }];
        [p[0] * c - p[1] * s + 7.0, p[0] * s + p[1] * c - 3.0, p[2] + 5.0]
    };
    let quads = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
    let mut out = vec![b' '; 80];
    out.extend(12u32.to_le_bytes());
    for [a, b, c, d] in quads {
        for t in [[a, b, c], [a, c, d]] {
            out.extend([0u8; 12]);
            for i in t {
                for x in corner(i) {
                    out.extend((x as f32).to_le_bytes());
                }
            }
            out.extend([0, 0]);
        }
    }
    out
}

#[test]
fn a_scan_is_set_straight_in_place_as_the_studio_does() {
    let dir = std::env::temp_dir().join(format!("c2f-mobile-level-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (phone, studio) = (dir.join("phone.stl"), dir.join("studio.stl"));
    std::fs::write(&phone, turned_box()).unwrap();
    std::fs::write(&studio, turned_box()).unwrap();
    let report = level_scan(phone.to_string_lossy().into_owned()).unwrap();
    assert!((report.turn.abs() - 30.0).abs() < 0.5, "{report:?}");
    assert!((report.size[0] - 60.0).abs() < 0.01 && (report.size[1] - 40.0).abs() < 0.01, "{report:?}");
    c2f_geom::level_file(&studio, &studio).unwrap();
    assert_eq!(std::fs::read(&phone).unwrap(), std::fs::read(&studio).unwrap(), "the same file both ways");
    assert!(level_scan(dir.join("none.stl").to_string_lossy().into_owned()).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
