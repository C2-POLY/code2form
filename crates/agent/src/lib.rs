//! Code2Form for AI agents: the tools an agent calls to write a model,
//! render it, look at it, export it and give it its electronics, the
//! prompts that lead it from an idea to a printed object, and the Model
//! Context Protocol server that offers them. `c2f mcp` and the studio's
//! own agent share them, so both call the very same tools.
//!
//! A [`Registry`] holds the tools: the [`Builtin`] ones first, then any a
//! host adds (the studio's live tools). A [`Server`] serves a registry over
//! JSON-RPC; an in-process engine calls the registry itself, with the
//! [`instructions`] and the [`prompts`]. [`conversation`] holds what the
//! studio's chat panel and its engines exchange.

use std::path::Path;

use serde_json::{Value, json};

pub mod circuit;
pub mod conversation;
pub mod engine;
pub mod keys;
pub mod loads;
pub mod mass;
mod mcp;
mod tools;

pub use mcp::{Builtin, Server};
pub use tools::{Registry, Tools};

/// What the agent is told when it connects: what the tools do and the way
/// through them.
pub fn instructions() -> &'static str {
    mcp::INSTRUCTIONS
}

/// The prompts, as `prompts/list` gives them: `{"prompts": [{name, title,
/// description, arguments}]}`.
pub fn prompts() -> Value {
    mcp::prompts::list()
}

/// The prompt `name` with its `arguments` (an object of strings) filled
/// in, as `prompts/get` gives it: `{description, messages}`; the error
/// says which prompt or argument is missing.
pub fn prompt(name: &str, arguments: &Value) -> Result<Value, String> {
    mcp::prompts::get(&json!({ "name": name, "arguments": arguments })).map_err(|(_, message)| message)
}

/// The geometry formats the CLI prints and the `export` tool writes:
/// meshes for a solid, drawings for a 2D model.
#[derive(Clone, Copy)]
pub enum Export {
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
    pub fn named(name: &str) -> Option<Export> {
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

    pub fn name(self) -> &'static str {
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

    pub fn extension(self) -> &'static str {
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
    pub fn write(self, rendered: &c2f_geom::Rendered, name: &str, view: Option<(f64, f64)>) -> Result<Vec<u8>, String> {
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
pub fn render(program: &c2f_eval::Program, output: &c2f_eval::Output) -> c2f_geom::Rendered {
    c2f_geom::render_in(&output.nodes, program.files[0].path.parent().unwrap_or(Path::new("")))
        .with_pauses(output.pauses.clone())
}
