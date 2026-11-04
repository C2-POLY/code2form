//! `c2f project`: create a project, say what one holds, or make one of a
//! folder that is not one yet.

use std::path::PathBuf;
use std::process::ExitCode;

use c2f_project::Project;

pub const USAGE: &str = "\
usage: c2f project new [--name <name>] <folder>
       c2f project info <folder | file.c2fproj>
       c2f project adopt [--name <name>] <folder>

  new     create a project in a new folder: its .c2fproj file, a brief
          to fill in, and its folders models/, scans/, circuit/,
          firmware/ and exports/ (named after the folder by default)
  info    what the project holds: its brief, designs (the first is the
          main one), circuit, firmware, scans and exports, and the files
          it names that are missing; fails on a badly written file
  adopt   make an existing folder a project, its files where they are:
          its designs, circuit, firmware, scans and brief";

pub fn run(args: &[String]) -> ExitCode {
    let mut name = None;
    let mut rest = Vec::new();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--name" => name = words.next().cloned(),
            _ => rest.push(word.clone()),
        }
    }
    let (Some(command), Some(path), None) = (rest.first(), rest.get(1), rest.get(2)) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let path = PathBuf::from(path);
    match command.as_str() {
        "new" => {
            let name = name.unwrap_or_else(|| path.file_name().map_or_else(|| "Project".into(), |n| n.to_string_lossy().into_owned()));
            done(c2f_project::create(&path, &name))
        }
        "adopt" => done(c2f_project::adopt(&path, name.as_deref())),
        "info" => match Project::open(&path) {
            Ok(project) => {
                print!("{}", info(&project));
                ExitCode::SUCCESS
            }
            Err(problems) => {
                for problem in problems {
                    eprintln!("error: {problem}");
                }
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn done(result: Result<Project, String>) -> ExitCode {
    match result {
        Ok(project) => {
            println!("wrote: {}", project.file.display());
            print!("{}", info(&project));
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// What a project holds, a line each.
fn info(project: &Project) -> String {
    let mut out = format!("project: {}\nfolder: {}\n", project.name, project.dir.display());
    let line = |label: &str, value: &str| format!("{label}: {value}\n");
    if let Some(brief) = &project.brief {
        out += &line("brief", brief);
    }
    for (i, design) in project.designs.iter().enumerate() {
        out += &line(if i == 0 { "design (main)" } else { "design" }, design);
    }
    if let Some(circuit) = &project.circuit {
        out += &line("circuit", circuit);
    }
    let firmware = project.relative(&project.firmware_dir()).unwrap_or_else(|| project.firmware_dir().display().to_string());
    out += &line("firmware", &format!("{firmware}{}", if project.has_firmware() { "" } else { " (not written yet)" }));
    for scan in &project.scans {
        out += &line("scan", scan);
    }
    out += &line("exports", &project.relative(&project.exports_dir()).unwrap_or_default());
    for missing in project.missing() {
        out += &line("missing", &missing);
    }
    out
}
