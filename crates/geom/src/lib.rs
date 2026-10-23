//! Geometry: turns the CSG tree from `c2f-eval` into meshes, with the
//! reference implementation's tessellation and Manifold's booleans, and
//! into triangles a viewer can draw; it also writes solids as STL, 3MF and
//! glTF (for game engines), and 2D models as SVG and DXF; and it draws
//! pictures of a model as PNG, without a window. It also finds where
//! placed solids collide.

mod build;
mod collide;
mod explode;
mod export;
mod gltf;
mod image;
mod import;
mod inventory;
mod mesh;
mod offset;
mod parts;
mod pieces;
mod primitives;
mod step;
mod text;
mod urdf;
mod wavefront;

pub use build::{DEFAULT_COLOR, Model, render, render_in};
pub use explode::{Explode, Exploded, exploded};
pub use collide::{Placement, TOUCHING, collisions};
pub use gltf::{Glb, GltfAnimation, GltfOptions, glb, program_glb};
pub use image::VIEWS;
pub use import::objects as mesh_objects;
pub use inventory::{PartsOptions, parts_json};
pub use mesh::{Rendered, RenderMesh, Stats};
pub use primitives::fragments;
pub use step::StepOptions;
pub use text::set_font_folders;
pub use urdf::{Urdf, UrdfOptions, urdf};
pub use wavefront::{Obj, ObjOptions, obj};
