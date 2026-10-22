//! `$t`, the animation's time: 0 unless set, and whether a program reads it.

fn evaluate(src: &str, t: Option<f64>) -> c2f_eval::Output {
    let mut program = c2f_eval::Program::load(src, std::path::Path::new("t.scad"), &[]).expect("parses");
    if let Some(t) = t {
        program.t = t;
    }
    c2f_eval::evaluate(&program)
}

#[test]
fn t_is_zero_unless_set() {
    assert_eq!(evaluate("echo($t);", None).echo, ["ECHO: 0"]);
    assert_eq!(evaluate("echo($t);", Some(0.25)).echo, ["ECHO: 0.25"]);
}

#[test]
fn an_assignment_in_the_program_wins() {
    // As `-D '$t=0.5'` appends it.
    assert_eq!(evaluate("echo($t); $t = 0.5;", Some(0.25)).echo, ["ECHO: 0.5"]);
}

#[test]
fn modules_and_functions_see_it() {
    let output = evaluate("function angle() = 360 * $t; module m(a = angle()) echo(a); m();", Some(0.5));
    assert_eq!(output.echo, ["ECHO: 180"]);
    assert!(output.animated);
}

#[test]
fn a_program_that_never_reads_it_is_not_animated() {
    assert!(!evaluate("cube(1); t = 3; echo(t);", None).animated);
    // Read in a branch not taken at this time: not seen.
    assert!(!evaluate("if (false) echo($t);", None).animated);
    assert!(evaluate("if ($t > 2) cube(1);", None).animated);
}
