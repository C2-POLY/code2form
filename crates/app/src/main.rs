use std::io::{IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;

mod mcp;

const USAGE: &str = "\
usage: c2f <command> [options] <file.c2f>
       c2f mcp

commands:
  ast     print the parsed program as an AST dump (`-o x.ast` layout)
  csg     evaluate and print the CSG tree (`-o x.csg` layout)
          (echo output and warnings go to stderr)
  stl     evaluate, build the geometry and print it as an ASCII STL
  3mf     the same, as a 3MF package (binary: redirect it to a file)
  svg     evaluate, build the 2D geometry and print it as an SVG
  dxf     the same, as a DXF drawing
  check   parse only and report syntax errors
  mcp     serve AI agents (Model Context Protocol) on stdin and stdout

options:
  -D name=value   sets a variable, as if assigned at the end of the file
                  (also written -Dname=value)
  -p file.json    reads customizer parameter sets from this file
  -P name         applies this parameter set (with -p)";

/// Reserved, not committed. Deeply nested programs recurse when printed.
const STACK: usize = 1 << 30;

fn main() -> ExitCode {
    std::thread::Builder::new()
        .name("c2f".into())
        .stack_size(STACK)
        .spawn(run)
        .expect("spawning the main thread")
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

struct Options {
    command: String,
    path: String,
    /// `-D` assignments.
    defines: Vec<String>,
    /// `-p` and `-P`.
    parameter_file: Option<String>,
    parameter_set: Option<String>,
}

impl Options {
    fn parse(args: &[String]) -> Option<Options> {
        let (command, mut rest) = args.split_first()?;
        let (mut defines, mut parameter_file, mut parameter_set, mut path) = (Vec::new(), None, None, None);
        while let Some((arg, tail)) = rest.split_first() {
            rest = tail;
            // `-D x=1` or `-Dx=1`: both are accepted.
            let (flag, value) = match ["-D", "-p", "-P"].into_iter().find(|f| arg.starts_with(f)) {
                Some(flag) if arg.len() > flag.len() => (Some(flag), arg[flag.len()..].to_string()),
                Some(flag) => {
                    let (value, tail) = rest.split_first()?;
                    rest = tail;
                    (Some(flag), value.clone())
                }
                None => (None, arg.clone()),
            };
            match flag {
                Some("-D") => defines.push(value),
                Some("-p") => parameter_file = Some(value),
                Some(_) => parameter_set = Some(value),
                None if path.is_none() => path = Some(value),
                None => return None,
            }
        }
        Some(Options { command: command.clone(), path: path?, defines, parameter_file, parameter_set })
    }
}

fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["mcp"] {
        return mcp::serve();
    }
    let Some(options) = Options::parse(&args) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    if !["ast", "check", "csg", "stl", "3mf", "svg", "dxf"].contains(&options.command.as_str()) {
        eprintln!("unknown command `{}`\n\n{USAGE}", options.command);
        return ExitCode::from(2);
    }
    if options.command == "3mf" && std::io::stdout().is_terminal() {
        eprintln!("error: a 3MF package is binary; redirect it to a file: c2f 3mf {} > model.3mf", options.path);
        return ExitCode::from(2);
    }
    let path = Path::new(&options.path);
    let mut src = match c2f_eval::read_source(path) {
        Ok(src) => src,
        Err(err) => {
            eprintln!("error: cannot read {}: {err}", options.path);
            return ExitCode::FAILURE;
        }
    };
    // Appended to the file after an end-of-text character, which the
    // lexer skips, as the reference implementation does.
    if !options.defines.is_empty() {
        src.push_str("\n\x03\n");
        for define in &options.defines {
            src.push_str(define);
            src.push_str(";\n");
        }
    }
    let mut program = match c2f_eval::Program::load(&src, path, &c2f_eval::default_library_paths()) {
        Ok(program) => program,
        Err(err) => {
            eprint!("{}", err.render());
            return ExitCode::FAILURE;
        }
    };
    if let Some(name) = &options.parameter_set {
        if let Err(err) = apply_parameter_set(&mut program, options.parameter_file.as_deref(), name) {
            eprintln!("ERROR: Cannot apply parameter set '{name}': {err}");
        }
    }
    match options.command.as_str() {
        // Includes are spliced in, as in the reference dump.
        "ast" => {
            print!("{}", c2f_lang::dump::ast_dump(&program.files[0].ast, &src));
            ExitCode::SUCCESS
        }
        "check" => ExitCode::SUCCESS,
        // `csg`, or an export format.
        command => csg(&program, Export::named(command)),
    }
}

fn apply_parameter_set(program: &mut c2f_eval::Program, file: Option<&str>, name: &str) -> Result<(), String> {
    let file = file.ok_or("no parameter file given (-p)")?;
    let json = std::fs::read_to_string(file).map_err(|err| format!("cannot read {file}: {err}"))?;
    let sets = c2f_eval::customizer::ParameterSets::parse(&json).map_err(|err| format!("{file}: {err}"))?;
    let set = sets.get(name).ok_or_else(|| format!("{file} has no such set"))?;
    program.apply_parameter_set(set)
}

/// The geometry formats the CLI prints: meshes for a solid, drawings for a
/// 2D model.
#[derive(Clone, Copy)]
enum Export {
    Stl,
    ThreeMf,
    Svg,
    Dxf,
}

impl Export {
    /// The format of a command or file extension.
    fn named(name: &str) -> Option<Export> {
        match name.to_ascii_lowercase().as_str() {
            "stl" => Some(Export::Stl),
            "3mf" => Some(Export::ThreeMf),
            "svg" => Some(Export::Svg),
            "dxf" => Some(Export::Dxf),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Export::Stl => "STL",
            Export::ThreeMf => "3MF",
            Export::Svg => "SVG",
            Export::Dxf => "DXF",
        }
    }

    /// The file in this format, or why there is none.
    fn write(self, rendered: &c2f_geom::Rendered) -> Result<Vec<u8>, String> {
        let (bytes, other) = match self {
            Export::Stl => (rendered.stl().map(String::into_bytes), "2D"),
            Export::ThreeMf => (rendered.three_mf(), "2D"),
            Export::Svg => (rendered.svg().map(String::into_bytes), "3D"),
            Export::Dxf => (rendered.dxf().map(String::into_bytes), "3D"),
        };
        bytes.ok_or_else(|| format!("the model is empty or {other}; no {} written", self.name()))
    }
}

/// Builds the geometry of an evaluated program, reading data files next to
/// its main file.
fn render(program: &c2f_eval::Program, output: &c2f_eval::Output) -> c2f_geom::Rendered {
    c2f_geom::render_in(&output.nodes, program.files[0].path.parent().unwrap_or(Path::new("")))
        .with_pauses(output.pauses.clone())
}

/// Prints the CSG tree, or with `export` the geometry in that format.
fn csg(program: &c2f_eval::Program, export: Option<Export>) -> ExitCode {
    let output = c2f_eval::evaluate(program);
    for line in &output.echo {
        eprintln!("{line}");
    }
    for warning in &output.warnings {
        eprintln!("WARNING: {warning}");
    }
    if let Some(export) = export {
        let rendered = render(program, &output);
        for warning in &rendered.warnings {
            eprintln!("WARNING: {warning}");
        }
        match export.write(&rendered) {
            Ok(bytes) => {
                // Flushed here: the flush at exit would drop a write error.
                let mut out = std::io::stdout().lock();
                if let Err(err) = out.write_all(&bytes).and_then(|()| out.flush()) {
                    eprintln!("error: cannot write the {}: {err}", export.name());
                    return ExitCode::FAILURE;
                }
            }
            Err(why) => eprintln!("WARNING: {why}"),
        }
        let s = rendered.stats;
        if rendered.is_2d {
            eprintln!("{} vertices, area {}", s.vertices, s.volume);
        } else {
            eprintln!("{} triangles, {} vertices, volume {}", s.triangles, s.vertices, s.volume);
        }
    } else {
        print!("{}", output.csg());
    }
    match &output.error {
        Some(error) => {
            eprintln!("ERROR: {error}");
            ExitCode::FAILURE
        }
        None => ExitCode::SUCCESS,
    }
}
