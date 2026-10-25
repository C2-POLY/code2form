//! The static loads on a turning shaft, the torque its motor must give and
//! the margins against the motor's and the bearings' sheets
//! ([`c2f_circuit::loads`]), from the JSON the MCP `loads` tool and
//! `c2f loads` take.

use std::path::{Path, PathBuf};

use c2f_circuit::loads::{self, Report, Scenario};
use serde_json::Value;

/// The report of a scenario as JSON: the parts it names are found as is,
/// in its `working_dir` (else in `dir`), then in the library folders.
pub fn evaluate(args: &Value, dir: Option<&Path>) -> Result<Report, String> {
    let mut args = args.clone();
    let working_dir = args.as_object_mut().and_then(|object| object.remove("working_dir"));
    let scenario: Scenario = serde_json::from_value(args).map_err(|err| format!("cannot read the scenario: {err}"))?;
    let dirs: Vec<PathBuf> = working_dir
        .as_ref()
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .or_else(|| dir.map(Path::to_path_buf))
        .into_iter()
        .chain(c2f_eval::default_library_paths())
        .collect();
    loads::analyse(&loads::with_sheets(&scenario, &dirs)?)
}
