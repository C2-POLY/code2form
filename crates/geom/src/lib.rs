//! Geometry: turns the CSG tree from `c2f-eval` into meshes, with the
//! reference implementation's tessellation and Manifold's booleans, and
//! into triangles a viewer can draw; it also writes solids as STL and 3MF,
//! and 2D models as SVG and DXF.

mod build;
mod export;
mod import;
mod mesh;
mod offset;
mod primitives;
mod text;

pub use build::{Model, render, render_in};
pub use mesh::{Rendered, RenderMesh, Stats};
pub use primitives::fragments;
