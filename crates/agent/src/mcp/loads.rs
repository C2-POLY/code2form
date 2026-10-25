//! The MCP tool that weighs a design's moving parts: `loads` computes, for
//! a shaft a motor turns, the torque it needs, what each support takes,
//! how far the shaft bends and the margins against the motor's and the
//! bearings' sheets ([`c2f_circuit::loads`]), so that an agent reviewing
//! a design decides on a front bearing, a reduction or another motor with
//! numbers.

use serde_json::{Value, json};

use super::{Tool, pretty, schema_of, text};

pub fn tools() -> Vec<Value> {
    let point = json!({ "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 });
    vec![json!({
        "name": "loads",
        "title": "Loads on a shaft and its motor",
        "description": "Static engineering estimates for a shaft a motor turns (a watch winder's cradle, a turntable, a \
                        pan axis), gravity along -z, positions in millimeters. From the loads (masses and where their \
                        centers sit) and the supports along the axis (`motor`, ball `bearing`, printed `bushing`), it \
                        gives the torque the shaft needs (`torque.needed_mNm`: the loads' weight in the worst position \
                        of a turn plus friction), each support's radial and axial reaction and moment (a motor holding \
                        the shaft alone carries an overhung load: the moment over its bearings' span adds to its front \
                        bearing's load), the shaft's deflection at each load, and the margins: the motor's torque \
                        through `ratio` over the torque needed, its maximum radial and axial loads over what it takes, \
                        a bearing's static load over its equivalent load. Warnings for each margin under \
                        `target_margin` (2) and for an overhung motor; `suggested_ratio`, the reduction that reaches \
                        the target when the torque falls short; `summary`, sentences to quote. The motor's and the \
                        bearings' figures come from the scenario or from their part's sheet (`// @spec torque: 34 \
                        mN·m`, `stall torque`, `holding torque`, `speed`, `max radial load`, `max axial load`, \
                        `static load`).",
        "inputSchema": schema_of(json!({
            "axis": {
                "type": "object",
                "description": "The line the shaft turns about: through `at`, along `direction` (horizontal, tilted or vertical).",
                "properties": { "at": point, "direction": point },
                "required": ["at", "direction"],
            },
            "loads": {
                "type": "array",
                "description": "What turns with the shaft: each `name`, `mass_g` and `at`, its center of mass.",
                "items": {
                    "type": "object",
                    "properties": { "name": { "type": "string" }, "mass_g": { "type": "number" }, "at": point },
                    "required": ["name", "mass_g", "at"],
                },
            },
            "supports": {
                "type": "array",
                "description": "Where the shaft is held: `at_mm` along the axis from its point; `kind` motor (its own \
                                output bearings), bearing or bushing (a printed hole); `part`, a file whose sheet gives \
                                its figures; `coupling` true for a motor joined by a flexible coupling, which holds \
                                nothing up; `static_load_N`, a bearing's C0; `span_mm`, the length over which it \
                                holds the shaft alone (8).",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "at_mm": { "type": "number" },
                        "kind": { "type": "string", "enum": ["motor", "bearing", "bushing"] },
                        "part": { "type": "string" },
                        "coupling": { "type": "boolean", "default": false },
                        "static_load_N": { "type": "number" },
                        "span_mm": { "type": "number" },
                    },
                    "required": ["name", "at_mm", "kind"],
                },
            },
            "motor": {
                "type": "object",
                "description": "The motor's figures, at its output shaft; a `part` (else the motor support's) gives \
                                those left out. Without `torque_mNm`, half `stall_torque_mNm`, else `holding_torque_mNm`.",
                "properties": {
                    "part": { "type": "string" },
                    "torque_mNm": { "type": "number" },
                    "stall_torque_mNm": { "type": "number" },
                    "holding_torque_mNm": { "type": "number" },
                    "speed_rpm": { "type": "number" },
                    "max_radial_load_N": { "type": "number" },
                    "max_axial_load_N": { "type": "number" },
                },
            },
            "shaft": {
                "type": "object",
                "description": "Its diameter `d_mm` and its `material` (steel, stainless, brass, aluminium, PLA, PETG) \
                                or Young's modulus `E_GPa`; 5 mm steel when left out.",
                "properties": {
                    "d_mm": { "type": "number" },
                    "material": { "type": "string" },
                    "E_GPa": { "type": "number" },
                },
                "required": ["d_mm"],
            },
            "ratio": { "type": "number", "default": 1, "description": "Turns of the motor for one of the shaft." },
            "stages": { "type": "integer", "description": "The reduction's stages, 90% efficient each (by default one per 6:1)." },
            "efficiency": { "type": "number", "description": "The drive's efficiency, instead of the stages'." },
            "target_margin": { "type": "number", "default": 2, "description": "The margin each figure should have." },
            "working_dir": { "type": "string", "description": "A folder the parts are read from before the library." },
        }), &["axis", "supports"]),
    })]
}

/// The tool named `name`, if it is this one.
pub fn tool(name: &str) -> Option<Tool> {
    Some(match name {
        "loads" => loads,
        _ => return None,
    })
}

fn loads(args: &Value) -> Result<Vec<Value>, String> {
    let report = crate::loads::evaluate(args, None)?;
    let report = serde_json::to_value(&report).map_err(|err| format!("internal error: {err}"))?;
    Ok(vec![text(pretty(&report))])
}
