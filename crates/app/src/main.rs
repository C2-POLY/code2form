use std::io::{IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;

mod circuit;
mod mcp;

const USAGE: &str = "\
usage: c2f <command> [options] <file.c2f>
       c2f circuit <command> [--csv] [--margin <mm>] [--clearance <mm>] <file.c2fc>
       c2f mcp

commands:
  ast     print the parsed program as an AST dump (`-o x.ast` layout)
  csg     evaluate and print the CSG tree (`-o x.csg` layout)
          (echo output and warnings go to stderr)
  stl     evaluate, build the geometry and print it as an ASCII STL
  3mf     the same, as a 3MF package (binary: redirect it to a file)
  glb     the same, as glTF 2.0 for game engines (Godot, Unity): in
          meters with Y up, a material per color (binary too)
  obj     the same, as Wavefront OBJ, an object per part(); with -o,
          the materials of its colors go in a .mtl beside it
  step    the same, as STEP for CAD programs: faceted solids, each
          face a planar polygon, in their colors
  urdf    the model as a robot (ROS, Gazebo, MuJoCo): a link for the
          root and each part(), joined by part(joint = ...); needs -o,
          the links' meshes go in a folder beside it
  json    the parts of the model as JSON: each part()'s place, joint,
          size, volume and colors, and the design's parameters
  section the outline where a plane cuts the solid, as JSON (--axis,
          --at, --tolerance): to measure a mesh before drawing it again;
          through the axis of a turned part, its profile
  svg     evaluate, build the 2D geometry and print it as an SVG
  dxf     the same, as a DXF drawing; a 3D model seen from --view
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
          each other or into the enclosure, wires that run into it)
          do not
  fit     how each part fits in the enclosure: its least room to the
          enclosure and to the other parts, and warnings for a part
          under the clearance, a port no cutout_for opens, a mount
          hole with no standoff, a wire that runs into the enclosure
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
  --clearance <mm>  the room fit wants around each part (default 0.5)

options:
  -D name=value   sets a variable, as if assigned at the end of the file
                  (also written -Dname=value)
  -p file.json    reads customizer parameter sets from this file
  -P name         applies this parameter set (with -p)
  -o <file>       writes the export to this file rather than printing it
  --view <view>   dxf of a 3D model: its outline seen from top, bottom,
                  front, back, left or right, or from <yaw>,<pitch> in
                  degrees (default top)
  --density <kg/m3>  urdf: what the links weigh by volume (default 1240,
                  PLA's)
  --axis <x|y|z>  section: the axis the plane is across (default z)
  --at <mm>       section: where along it (default 0)
  --tolerance <mm>  section: points this close to a straight line are
                  left out (default 0.01)
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
    /// `-o`.
    output: Option<String>,
    /// `--view`, as yaw and pitch in degrees.
    view: Option<(f64, f64)>,
    /// `--density`.
    density: Option<f64>,
    /// `--axis`, `--at` and `--tolerance`, for `section`.
    section: (c2f_geom::Axis, f64, f64),
}

impl Options {
    fn parse(args: &[String]) -> Option<Options> {
        let (command, mut rest) = args.split_first()?;
        let (mut defines, mut parameter_file, mut parameter_set, mut path) = (Vec::new(), None, None, None);
        let mut gltf = c2f_geom::GltfOptions::default();
        let (mut still, mut animation_name) = (false, None);
        let (mut output, mut view, mut density) = (None, None, None);
        let mut section = (c2f_geom::Axis::Z, 0.0, 0.01);
        while let Some((arg, tail)) = rest.split_first() {
            rest = tail;
            let animation = gltf.animation.get_or_insert_with(Default::default);
            let positive = |value: &String| value.parse().ok().filter(|v: &f64| v.is_finite() && *v > 0.0);
            match arg.as_str() {
                "--scale" | "--duration" | "--fps" | "--animation" | "-o" | "--view" | "--density" | "--axis" | "--at"
                | "--tolerance" => {
                    let (value, tail) = rest.split_first()?;
                    rest = tail;
                    match arg.as_str() {
                        "--scale" => gltf.scale = positive(value)?,
                        "--duration" => animation.duration = positive(value)?,
                        "--fps" => animation.fps = positive(value)?,
                        "-o" => output = Some(value.clone()),
                        "--view" => view = Some(parse_view(value)?),
                        "--density" => density = Some(positive(value)?),
                        "--axis" => section.0 = c2f_geom::Axis::named(value)?,
                        "--at" => section.1 = value.parse().ok().filter(|v: &f64| v.is_finite())?,
                        "--tolerance" => section.2 = value.parse().ok().filter(|v: &f64| v.is_finite() && *v >= 0.0)?,
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
        Some(Options { command: command.clone(), path, defines, parameter_file, parameter_set, gltf, output, view, density, section })
    }

    /// The name of what is written: the output's stem, or the model's.
    fn stem(&self) -> String {
        let file = self.output.as_deref().unwrap_or(&self.path);
        Path::new(file).file_stem().map_or_else(|| "model".into(), |s| s.to_string_lossy().into_owned())
    }
}

/// `top`, `front`… or `yaw,pitch` in degrees.
fn parse_view(text: &str) -> Option<(f64, f64)> {
    if let Some(&(_, yaw, pitch)) = c2f_geom::VIEWS.iter().find(|(name, _, _)| *name == text) {
        return Some((f64::from(yaw), f64::from(pitch)));
    }
    let (yaw, pitch) = text.split_once(',')?;
    let (yaw, pitch): (f64, f64) = (yaw.trim().parse().ok()?, pitch.trim().parse().ok()?);
    (yaw.is_finite() && pitch.is_finite()).then_some((yaw, pitch))
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
    if !["ast", "check", "pins", "csg", "stl", "3mf", "glb", "obj", "step", "urdf", "json", "section", "svg", "dxf"]
        .contains(&options.command.as_str())
    {
        eprintln!("unknown command `{}`\n\n{USAGE}", options.command);
        return ExitCode::from(2);
    }
    if options.command == "urdf" && options.output.is_none() {
        eprintln!("error: a URDF comes with its links' meshes; give the file to write: c2f urdf -o robot.urdf {}", options.path);
        return ExitCode::from(2);
    }
    if let Some(export @ (Export::ThreeMf | Export::Glb)) = Export::named(&options.command) {
        if options.output.is_none() && std::io::stdout().is_terminal() {
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
        "section" => section(&program, &options),
        // `csg`, or an export format.
        command => csg(&mut program, &src, Export::named(command), &options),
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
    Obj,
    Step,
    /// Written from the parts, with their meshes beside it.
    Urdf,
    /// The parts, from the evaluated nodes.
    Json,
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
            "obj" => Some(Export::Obj),
            "step" | "stp" => Some(Export::Step),
            "urdf" => Some(Export::Urdf),
            "json" => Some(Export::Json),
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
            Export::Obj => "OBJ",
            Export::Step => "STEP",
            Export::Urdf => "URDF",
            Export::Json => "JSON",
            Export::Svg => "SVG",
            Export::Dxf => "DXF",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Export::Stl => "stl",
            Export::ThreeMf => "3mf",
            Export::Glb => "glb",
            Export::Obj => "obj",
            Export::Step => "step",
            Export::Urdf => "urdf",
            Export::Json => "json",
            Export::Svg => "svg",
            Export::Dxf => "dxf",
        }
    }

    /// The file in this format, or why there is none. `name` names a
    /// STEP's product; `view` draws a 3D model's outline in a DXF.
    fn write(self, rendered: &c2f_geom::Rendered, name: &str, view: Option<(f64, f64)>) -> Result<Vec<u8>, String> {
        let (bytes, other) = match self {
            Export::Stl => (rendered.stl().map(String::into_bytes), "2D"),
            Export::ThreeMf => (rendered.three_mf(), "2D"),
            Export::Step => {
                let options = c2f_geom::StepOptions { name: name.into(), ..Default::default() };
                (rendered.step(&options).map(String::into_bytes), "2D")
            }
            Export::Glb | Export::Obj | Export::Urdf | Export::Json => {
                return Err(format!("a {} is written from the evaluated nodes, not a rendering", self.name()));
            }
            Export::Svg => (rendered.svg().map(String::into_bytes), "3D"),
            Export::Dxf => match view {
                Some((yaw, pitch)) => (rendered.view_dxf(yaw, pitch).map(String::into_bytes), "3D"),
                // A 3D model is seen from the top.
                None if !rendered.is_2d => (rendered.view_dxf(-90.0, 90.0).map(String::into_bytes), "2D"),
                None => (rendered.dxf().map(String::into_bytes), "3D"),
            },
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
fn csg(program: &mut c2f_eval::Program, src: &str, export: Option<Export>, options: &Options) -> ExitCode {
    let output = c2f_eval::evaluate(program);
    for line in &output.echo {
        eprintln!("{line}");
    }
    for warning in &output.warnings {
        eprintln!("WARNING: {warning}");
    }
    if let Some(Export::Glb) = export {
        // A program stopped by an error has no glTF: the error says why.
        if output.error.is_none() && print_gltf(program, &options.gltf, options.output.as_deref()) != ExitCode::SUCCESS {
            return ExitCode::FAILURE;
        }
    } else if let Some(export @ (Export::Obj | Export::Urdf | Export::Json)) = export {
        if output.error.is_none() && print_parts(program, src, &output, export, options) != ExitCode::SUCCESS {
            return ExitCode::FAILURE;
        }
    } else if let Some(export) = export {
        let rendered = render(program, &output);
        for warning in &rendered.warnings {
            eprintln!("WARNING: {warning}");
        }
        if print_export(&rendered, export, &options.stem(), options.view, options.output.as_deref()) != ExitCode::SUCCESS {
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
/// `name` names a STEP's product, `view` draws a DXF, `output` is `-o`.
fn print_export(
    rendered: &c2f_geom::Rendered,
    export: Export,
    name: &str,
    view: Option<(f64, f64)>,
    output: Option<&str>,
) -> ExitCode {
    match export.write(rendered, name, view) {
        Ok(bytes) => {
            if let Err(err) = emit(output, &bytes) {
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
fn print_gltf(program: &mut c2f_eval::Program, options: &c2f_geom::GltfOptions, output: Option<&str>) -> ExitCode {
    match c2f_geom::program_glb(program, options) {
        Ok(glb) => {
            for warning in &glb.warnings {
                eprintln!("WARNING: {warning}");
            }
            if let Err(err) = emit(output, &glb.bytes) {
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

/// Prints where a plane cuts the solid, as JSON.
fn section(program: &c2f_eval::Program, options: &Options) -> ExitCode {
    let output = c2f_eval::evaluate(program);
    for warning in &output.warnings {
        eprintln!("WARNING: {warning}");
    }
    if let Some(error) = &output.error {
        eprintln!("ERROR: {error}");
        return ExitCode::FAILURE;
    }
    let rendered = render(program, &output);
    for warning in &rendered.warnings {
        eprintln!("WARNING: {warning}");
    }
    let (axis, at, tolerance) = options.section;
    match rendered.section(axis, at) {
        Some(cut) => {
            let json = serde_json::to_string_pretty(&cut.json(axis, at, tolerance)).unwrap_or_default();
            println!("{json}");
            ExitCode::SUCCESS
        }
        None => {
            eprintln!("error: the model is empty or 2D: nothing to cut");
            ExitCode::FAILURE
        }
    }
}

/// Why the parts were not written: the model has none (a warning, as for
/// the other exports), or a file could not be written (a failure).
enum Unwritten {
    Model(String),
    Io(String),
}

/// Writes the parts as OBJ (and its `.mtl` with `-o`), as a robot (its
/// meshes beside it) or as JSON, then their size on stderr.
fn print_parts(program: &c2f_eval::Program, src: &str, output: &c2f_eval::Output, export: Export, options: &Options) -> ExitCode {
    let base_dir = program.files[0].path.parent().unwrap_or(Path::new(""));
    let name = options.stem();
    let target = options.output.as_deref().map(Path::new);
    let write = |path: &Path, bytes: &[u8]| write_file(path, bytes).map_err(Unwritten::Io);
    let print = |bytes: &[u8]| emit(options.output.as_deref(), bytes).map_err(|err| Unwritten::Io(format!("cannot write the {}: {err}", export.name())));
    let written = match export {
        Export::Obj => c2f_geom::obj(&output.nodes, base_dir, &c2f_geom::ObjOptions { name: name.clone(), ..Default::default() })
            .map_err(Unwritten::Model)
            .and_then(|obj| {
                match target {
                    Some(target) => write(&target.with_file_name(&obj.mtl_file), obj.mtl.as_bytes())?,
                    None => eprintln!("WARNING: the colors' {} is only written with -o", obj.mtl_file),
                }
                print(obj.obj.as_bytes())?;
                Ok((obj.warnings, format!("{} triangles", obj.triangles)))
            }),
        Export::Urdf => {
            let target = target.expect("checked before evaluating");
            let folder = format!("{name}_meshes");
            let urdf_options =
                c2f_geom::UrdfOptions { name: name.clone(), mesh_folder: folder, density: options.density.unwrap_or(1240.0), ..Default::default() };
            c2f_geom::urdf(&output.nodes, base_dir, &urdf_options).map_err(Unwritten::Model).and_then(|urdf| {
                let dir = target.parent().unwrap_or(Path::new(""));
                for (file, bytes) in &urdf.meshes {
                    let path = dir.join(file);
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)
                            .map_err(|err| Unwritten::Io(format!("cannot create {}: {err}", parent.display())))?;
                    }
                    write(&path, bytes)?;
                }
                write(target, urdf.urdf.as_bytes())?;
                Ok((urdf.warnings, format!("{} links, {} moving joints", urdf.links, urdf.moving)))
            })
        }
        _ => {
            let parameters = program
                .parameters(src)
                .into_iter()
                .filter(|p| p.group != c2f_eval::customizer::HIDDEN_GROUP)
                .map(|p| (p.name, p.default))
                .collect();
            let parts_options = c2f_geom::PartsOptions { name: name.clone(), parameters, ..Default::default() };
            c2f_geom::parts_json(&output.nodes, base_dir, &parts_options).map_err(Unwritten::Model).and_then(|(json, warnings)| {
                print(json.as_bytes())?;
                let parts = json.matches("\"parent\":").count();
                Ok((warnings, format!("{parts} parts")))
            })
        }
    };
    match written {
        Ok((warnings, size)) => {
            for warning in &warnings {
                eprintln!("WARNING: {warning}");
            }
            eprintln!("{size}");
            ExitCode::SUCCESS
        }
        Err(Unwritten::Model(why)) => {
            eprintln!("WARNING: {why}");
            ExitCode::SUCCESS
        }
        Err(Unwritten::Io(why)) => {
            eprintln!("error: {why}");
            ExitCode::FAILURE
        }
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|err| format!("cannot write {}: {err}", path.display()))
}

/// Writes `bytes` to the `-o` file, or to stdout.
fn emit(output: Option<&str>, bytes: &[u8]) -> std::io::Result<()> {
    match output {
        Some(path) => std::fs::write(path, bytes),
        None => print_bytes(bytes),
    }
}

/// Writes `bytes` to stdout, flushed here: the flush at exit would drop a
/// write error.
fn print_bytes(bytes: &[u8]) -> std::io::Result<()> {
    let mut out = std::io::stdout().lock();
    out.write_all(bytes).and_then(|()| out.flush())
}
