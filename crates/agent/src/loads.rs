//! The static loads on a turning shaft, the torque its motor must give and
//! the margins against the motor's and the bearings' sheets
//! ([`c2f_circuit::loads`]), from the JSON the MCP `loads` tool and
//! `c2f loads` take.

use std::path::{Path, PathBuf};

use c2f_circuit::loads::{self, Report, Scenario};
use c2f_lang::FileScope;
use serde_json::Value;

/// The report of a scenario as JSON: the parts it names are found as is,
/// in its `working_dir` (else in `dir`), then in the library folders; in
/// the scope it carries ([`crate::guard::SCOPE_KEY`]), if any.
pub fn evaluate(args: &Value, dir: Option<&Path>) -> Result<Report, String> {
    evaluate_in(args, dir, crate::guard::file_scope(args).as_ref())
}

/// [`evaluate`] with the parts found in `scope` alone when there is one,
/// never from the current directory.
pub fn evaluate_in(args: &Value, dir: Option<&Path>, scope: Option<&FileScope>) -> Result<Report, String> {
    let mut args = args.clone();
    if let Some(object) = args.as_object_mut() {
        object.remove(crate::guard::SCOPE_KEY);
    }
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
    loads::analyse(&loads::with_sheets_in(&scenario, &dirs, scope)?)
}
