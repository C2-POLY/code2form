use std::io::{IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;

mod circuit;
mod mcp;

const USAGE: &str = "\
usage: c2f <command> [options] <file.c2f>
       c2f circuit <command> [--csv] [--margin <mm>] <file.c2fc>
       c2f mcp

commands:
  ast     print the parsed program as an AST dump (`-o x.ast` layout)
  csg     evaluate and print the CSG tree (`-o x.csg` layout)
          (echo output and warnings go to stderr)
  stl     evaluate, build the geometry and print it as an ASCII STL
  3mf     the same, as a 3MF package (binary: redirect it to a file)
  glb     the same, as glTF 2.0 for game engines (Godot, Unity): in
          meters with Y up, a material per color (binary too)
  svg     evaluate, build the 2D geometry and print it as an SVG
  dxf     the same, as a DXF drawing
  check   parse only and report syntax errors and badly written
          electronics annotations
  pins    print the pins, mount holes and ports a part declares
          (`// @pin`, `// @mount_hole`, `// @port`), one per line
  mcp     serve AI agents (Model Context Protocol) on stdin and stdout

circuit commands, on a `.c2fc` circuit:
  check   read the circuit, evaluate its parts and check its wiring:
          errors (a supply above what a pin takes, supplies tied
          together or to ground, a supply or ground left open, a
          wired nc pin) fail it; warnings (no common ground, a wired
          reserved pin, two outputs on a net, parts that run into
          each other or into the enclosure) do not
  wires   print the wiring list in the order to lay the wires: step,
          from pin to pin (with their labels), color, length and label;
          the length is the routed curve's plus a margin
  bom     print the bill of materials: the parts by file and params,
          their quantity and the instances' names
  stl     the parts, placed and evaluated with their params, as one
          ASCII STL (wires are not exported)
  3mf     the same, as a 3MF package (binary: redirect it to a file)
  --csv   wires and bom as comma-separated values
  --margin <mm>  added to each wire's length (default 10: two stripped
          ends of 5 mm)

options:
  -D name=value   sets a variable, as if assigned at the end of the file
                  (also written -Dname=value)
  -p file.json    reads customizer parameter sets from this file
  -P name         applies this parameter set (with -p)
  --scale <m>     glb: meters per model unit (default 0.001: a model in
                  millimeters keeps its size; 1 makes each unit a meter)
  --duration <s>  glb of a model that reads $t: the seconds its parts take
                  to go from $t = 0 to 1 (default 2)
  --fps <n>       glb: frames sampled a second (default 30)
  --animation <name>  glb: the animation's name (default the file's name)
  --no-loop       glb: an animation played once; it loops by default, its
                  name ending in -loop, which Godot reads
  --still         glb: the model at $t = 0, without animation";

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
    /// `--scale`, for `glb`.
    gltf: c2f_geom::GltfOptions,
}

impl Options {
    fn parse(args: &[String]) -> Option<Options> {
        let (command, mut rest) = args.split_first()?;
        let (mut defines, mut parameter_file, mut parameter_set, mut path) = (Vec::new(), None, None, None);
        let mut gltf = c2f_geom::GltfOptions::default();
        let (mut still, mut animation_name) = (false, None);
        while let Some((arg, tail)) = rest.split_first() {
            rest = tail;
            let animation = gltf.animation.get_or_insert_with(Default::default);
            let positive = |value: &String| value.parse().ok().filter(|v: &f64| v.is_finite() && *v > 0.0);
            match arg.as_str() {
                "--scale" | "--duration" | "--fps" | "--animation" => {
                    let (value, tail) = rest.split_first()?;
                    rest = tail;
                    match arg.as_str() {
                        "--scale" => gltf.scale = positive(value)?,
                        "--duration" => animation.duration = positive(value)?,
                        "--fps" => animation.fps = positive(value)?,
                        _ => animation_name = Some(value.clone()),
                    }
                    continue;
                }
                "--no-loop" => {
                    animation.looped = false;
                    continue;
                }
                "--still" => {
                    still = true;
                    continue;
                }
                _ => {}
            }
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
        let path = path?;
        if let Some(stem) = Path::new(&path).file_stem() {
            gltf.name = stem.to_string_lossy().into_owned();
        }
        if let Some(animation) = gltf.animation.as_mut() {
            animation.name = animation_name.unwrap_or_else(|| gltf.name.clone());
        }
        if still {
            gltf.animation = None;
        }
        Some(Options { command: command.clone(), path, defines, parameter_file, parameter_set, gltf })
    }
}

fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["mcp"] {
        return mcp::serve();
    }
    if args.first().is_some_and(|arg| arg == "circuit") {
        return circuit::run(&args[1..]);
    }
    let Some(options) = Options::parse(&args) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    if !["ast", "check", "pins", "csg", "stl", "3mf", "glb", "svg", "dxf"].contains(&options.command.as_str()) {
        eprintln!("unknown command `{}`\n\n{USAGE}", options.command);
        return ExitCode::from(2);
    }
    if let Some(export @ (Export::ThreeMf | Export::Glb)) = Export::named(&options.command) {
        if std::io::stdout().is_terminal() {
            let (name, ext) = (export.name(), export.extension());
            eprintln!("error: a {name} file is binary; redirect it to a file: c2f {ext} {} > model.{ext}", options.path);
            return ExitCode::from(2);
        }
    }
    let path = Path::new(&options.path);
    let mut src = match c2f_eval::read_source(path) {
        Ok(src) => src,
        Err(err) => {
            eprintln!("error: cannot read {}: {err}", options.path);
            return ExitCode::FAILURE;
        }
    };
    if options.command == "pins" {
        return pins(&src, &options.path);
    }
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
        "check" => match annotation_errors(&src, &options.path) {
            0 => ExitCode::SUCCESS,
            _ => ExitCode::FAILURE,
        },
        // `csg`, or an export format.
        command => csg(&mut program, Export::named(command), &options.gltf),
    }
}

/// Prints the annotations of a part in source order, each after its line
/// number, as `12: @pin D3 [2.54, 15.2, 1.6] up io 3.3V "GPIO4"`.
fn pins(src: &str, file_name: &str) -> ExitCode {
    let (annotations, _) = c2f_lang::annotations::read(src);
    let mut printed: Vec<(c2f_lang::Span, String)> = Vec::new();
    printed.extend(annotations.pins.iter().map(|a| (a.span, a.to_string())));
    printed.extend(annotations.mount_holes.iter().map(|a| (a.span, a.to_string())));
    printed.extend(annotations.ports.iter().map(|a| (a.span, a.to_string())));
    printed.sort_by_key(|(span, _)| span.start);
    let lines = c2f_lang::span::LineIndex::new(src);
    for (span, text) in printed {
        println!("{}: {text}", lines.line(span.start as usize) + 1);
    }
    match annotation_errors(src, file_name) {
        0 => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}

/// Reports the badly written annotations of a file; returns how many.
fn annotation_errors(src: &str, file_name: &str) -> usize {
    let (_, errors) = c2f_lang::annotations::read(src);
    for error in &errors {
        eprint!("{}", error.render(src, file_name));
    }
    errors.len()
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
    /// Written from the evaluated nodes rather than the rendering: see
    /// [`gltf`].
    Glb,
    Svg,
    Dxf,
}

impl Export {
    /// The format of a command or file extension.
    fn named(name: &str) -> Option<Export> {
        match name.to_ascii_lowercase().as_str() {
            "stl" => Some(Export::Stl),
            "3mf" => Some(Export::ThreeMf),
            "glb" => Some(Export::Glb),
            "svg" => Some(Export::Svg),
            "dxf" => Some(Export::Dxf),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Export::Stl => "STL",
            Export::ThreeMf => "3MF",
            Export::Glb => "glTF",
            Export::Svg => "SVG",
            Export::Dxf => "DXF",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Export::Stl => "stl",
            Export::ThreeMf => "3mf",
            Export::Glb => "glb",
            Export::Svg => "svg",
            Export::Dxf => "dxf",
        }
    }

    /// The file in this format, or why there is none.
    fn write(self, rendered: &c2f_geom::Rendered) -> Result<Vec<u8>, String> {
        let (bytes, other) = match self {
            Export::Stl => (rendered.stl().map(String::into_bytes), "2D"),
            Export::ThreeMf => (rendered.three_mf(), "2D"),
            Export::Glb => return Err("a glTF is written from the evaluated nodes, not a rendering".into()),
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
fn csg(program: &mut c2f_eval::Program, export: Option<Export>, gltf: &c2f_geom::GltfOptions) -> ExitCode {
    let output = c2f_eval::evaluate(program);
    for line in &output.echo {
        eprintln!("{line}");
    }
    for warning in &output.warnings {
        eprintln!("WARNING: {warning}");
    }
    if let (Some(Export::Glb), None) = (export, &output.error) {
        if print_gltf(program, gltf) != ExitCode::SUCCESS {
            return ExitCode::FAILURE;
        }
    } else if let Some(export) = export {
        let rendered = render(program, &output);
        for warning in &rendered.warnings {
            eprintln!("WARNING: {warning}");
        }
        if print_export(&rendered, export) != ExitCode::SUCCESS {
            return ExitCode::FAILURE;
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

/// Prints the geometry in the format `export`, then its size on stderr.
fn print_export(rendered: &c2f_geom::Rendered, export: Export) -> ExitCode {
    match export.write(rendered) {
        Ok(bytes) => {
            if let Err(err) = print_bytes(&bytes) {
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
    ExitCode::SUCCESS
}

/// Prints the program as a `.glb`, animated when it reads `$t`, then its
/// size on stderr.
fn print_gltf(program: &mut c2f_eval::Program, options: &c2f_geom::GltfOptions) -> ExitCode {
    match c2f_geom::program_glb(program, options) {
        Ok(glb) => {
            for warning in &glb.warnings {
                eprintln!("WARNING: {warning}");
            }
            if let Err(err) = print_bytes(&glb.bytes) {
                eprintln!("error: cannot write the glTF: {err}");
                return ExitCode::FAILURE;
            }
            match glb.frames {
                0 => eprintln!("{} triangles, {} vertices", glb.triangles, glb.vertices),
                frames => eprintln!("{} triangles, {} vertices, {frames} frames", glb.triangles, glb.vertices),
            }
        }
        Err(why) => eprintln!("WARNING: {why}"),
    }
    ExitCode::SUCCESS
}

/// Writes `bytes` to stdout, flushed here: the flush at exit would drop a
/// write error.
fn print_bytes(bytes: &[u8]) -> std::io::Result<()> {
    let mut out = std::io::stdout().lock();
    out.write_all(bytes).and_then(|()| out.flush())
}
