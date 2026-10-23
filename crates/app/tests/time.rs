//! `c2f csg -D '$t=T'` evaluates the program at time `T`, as the reference
//! implementation does.

use std::path::Path;
use std::process::Command;

#[test]
fn define_sets_the_time_as_the_reference_does() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/oracle/time");
    for t in ["0.25", "0.7"] {
        let output = Command::new(env!("CARGO_BIN_EXE_c2f"))
            .args(["csg", "-D", &format!("$t={t}"), "turret.scad"])
            .current_dir(&dir)
            .output()
            .expect("running c2f");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let golden = std::fs::read_to_string(dir.join(format!("turret.{t}.csg"))).unwrap();
        assert_eq!(String::from_utf8(output.stdout).unwrap(), golden, "$t = {t}");
        let echo = std::fs::read_to_string(dir.join(format!("turret.{t}.echo"))).unwrap();
        assert_eq!(String::from_utf8(output.stderr).unwrap(), echo, "$t = {t}");
    }
}
