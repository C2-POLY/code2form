//! `c2f pins` and `c2f check` on a part's electronics annotations.

use std::path::PathBuf;
use std::process::{Command, Output};

/// Runs `c2f <command>` on `src`, saved to a file of its own.
fn c2f(command: &str, name: &str, src: &str) -> Output {
    let path: PathBuf = std::env::temp_dir().join(format!("c2f-pins-{}-{name}.c2f", std::process::id()));
    std::fs::write(&path, src).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_c2f")).arg(command).arg(&path).output().expect("running c2f");
    let _ = std::fs::remove_file(&path);
    output
}

#[test]
fn pins_prints_each_annotation_with_its_line() {
    let src = "\
// A breakout board.
// @pin VIN [1, 1, 1.6] power_in 5V
// @pin GND [3.54, 1, 1.6] ground
// @mount_hole [2, 8] d=2
cube([20, 10, 1.6]); // @port usb [0, 5, 1] -x [8, 3]
";
    let output = c2f("pins", "good", src);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "\
2: @pin VIN [1, 1, 1.6] up power_in 5V
3: @pin GND [3.54, 1, 1.6] up ground
4: @mount_hole [2, 8, 0] d=2 down
5: @port usb [0, 5, 1] -x [8, 3]
"
    );
}

#[test]
fn badly_written_annotations_fail_pins_and_check() {
    let src = "cube(1);\n// @pin A [0, 0, 0]\n// @pin A [1, 0, 0]\n";
    for command in ["pins", "check"] {
        let output = c2f(command, &format!("bad-{command}"), src);
        assert!(!output.status.success(), "{command}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("error: the part already has a pin named `A`"), "{stderr}");
        assert!(stderr.contains(":3:9"), "{stderr}");
    }
    assert!(c2f("check", "plain", "cube(1); // @todo\n").status.success());
}
