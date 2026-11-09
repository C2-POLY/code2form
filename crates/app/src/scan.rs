//! `c2f scan level`: set a scan straight, as the studio and the app do.

use std::path::PathBuf;
use std::process::ExitCode;

pub const USAGE: &str = "\
usage: c2f scan level [-o <out.stl>] <scan>

  level   set a scan straight: take off its floating bits, stand it on
          the flat face it rests on (within 25° of down), turn its
          footprint along X and Y, and centre it on z = 0; written as a
          binary STL in mm, over the scan unless -o names another file";

pub fn run(args: &[String]) -> ExitCode {
    let mut out = None;
    let mut rest = Vec::new();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "-o" => out = words.next().map(PathBuf::from),
            _ => rest.push(word.clone()),
        }
    }
    let (Some("level"), Some(path), None) = (rest.first().map(String::as_str), rest.get(1), rest.get(2)) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let path = PathBuf::from(path);
    let out = out.unwrap_or_else(|| path.clone());
    match c2f_geom::level_file(&path, &out) {
        Ok(leveled) => {
            let [x, y, z] = leveled.size;
            println!("{}: {x:.1} × {y:.1} × {z:.1} mm", out.display());
            if leveled.base {
                println!("stood on its base, tipped {:.1}°", leveled.tilt);
            } else {
                println!("no flat face within 25° of down: down kept as scanned");
            }
            println!("turned {:.1}° about Z", leveled.turn);
            if leveled.islands > 0 {
                println!("{} floating bit(s) taken off", leveled.islands);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("c2f scan: {e}");
            ExitCode::FAILURE
        }
    }
}
