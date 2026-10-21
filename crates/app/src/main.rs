use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "\
usage: c2f <command> [options] <file.c2f>

commands:
  ast     print the parsed program as an AST dump (`-o x.ast` layout)
  csg     evaluate and print the CSG tree (`-o x.csg` layout)
          (echo output and warnings go to stderr)
  stl     evaluate, build the geometry and print it as an ASCII STL
  check   parse only and report syntax errors

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
    let Some(options) = Options::parse(&args) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    if !["ast", "check", "csg", "stl"].contains(&options.command.as_str()) {
        eprintln!("unknown command `{}`\n\n{USAGE}", options.command);
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
        "csg" => csg(&program, false),
        _ => csg(&program, true),
    }
}

fn apply_parameter_set(program: &mut c2f_eval::Program, file: Option<&str>, name: &str) -> Result<(), String> {
    let file = file.ok_or("no parameter file given (-p)")?;
    let json = std::fs::read_to_string(file).map_err(|err| format!("cannot read {file}: {err}"))?;
    let sets = c2f_eval::customizer::ParameterSets::parse(&json).map_err(|err| format!("{file}: {err}"))?;
    let set = sets.get(name).ok_or_else(|| format!("{file} has no such set"))?;
    program.apply_parameter_set(set)
}

fn csg(program: &c2f_eval::Program, stl: bool) -> ExitCode {
    let output = c2f_eval::evaluate(program);
    for line in &output.echo {
        eprintln!("{line}");
    }
    for warning in &output.warnings {
        eprintln!("WARNING: {warning}");
    }
    if stl {
        let rendered = c2f_geom::render_in(&output.nodes, program.files[0].path.parent().unwrap_or(Path::new("")));
        for warning in &rendered.warnings {
            eprintln!("WARNING: {warning}");
        }
        match rendered.stl() {
            Some(text) => print!("{text}"),
            None => eprintln!("WARNING: the model is empty or 2D; no STL written"),
        }
        let s = rendered.stats;
        eprintln!("{} triangles, {} vertices, volume {}", s.triangles, s.vertices, s.volume);
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
