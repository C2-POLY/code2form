//! Geometry: turns the CSG tree from `c2f-eval` into meshes, with the
//! reference implementation's tessellation and Manifold's booleans, and
//! into triangles a viewer can draw; it also writes solids as STL, 3MF and
//! glTF (for game engines), and 2D models as SVG and DXF; and it draws
//! pictures of a model as PNG, without a window. It also finds where
//! placed solids collide, and how the parts of a circuit fit in their
//! enclosure, and what a model weighs and how it stands.

mod build;
mod collide;
mod explode;
mod fit;
mod export;
mod gltf;
mod image;
mod import;
mod inventory;
mod mass;
mod mesh;
mod offset;
mod parts;
mod pieces;
mod primitives;
mod section;
mod step;
mod text;
mod urdf;
mod wavefront;

pub use build::{DEFAULT_COLOR, Model, render, render_in};
pub use explode::{Explode, Exploded, exploded};
pub use collide::{Placement, TOUCHING, collisions};
pub use fit::{CLEARANCE, Crossing, Gap, PartFit, SEARCH, crossings, fit};
pub use gltf::{Glb, GltfAnimation, GltfOptions, glb, program_glb};
pub use image::{VIEWS, encode_png, sheet_png};
pub use import::objects as mesh_objects;
pub use inventory::{PartsOptions, parts_json};
pub use mass::{
    BASE_TOLERANCE, Component, DEFAULT_INFILL, DEFAULT_WALL, Extra, MATERIALS, MassOptions, MassReport, Material, PartMass, Print,
    Stability, mass_properties,
};
pub use mesh::{Rendered, RenderMesh, Stats};
pub use primitives::fragments;
pub use section::{Axis, Section};
pub use step::StepOptions;
pub use text::set_font_folders;
pub use urdf::{Urdf, UrdfOptions, urdf};
pub use wavefront::{Obj, ObjOptions, obj};
