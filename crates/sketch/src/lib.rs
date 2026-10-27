//! Sketches, as in a parametric CAD program: a 2D profile drawn on a plane
//! with lines, arcs and circles, held by constraints (horizontal,
//! parallel, tangent...) and dimensions (lengths, radii, angles), which a
//! solver meets by moving the geometry. A sketch becomes code: a module
//! `sketch_<name>()` drawing its profile with `polygon()`, its named
//! dimensions as Customizer parameters, then extrusions and revolutions
//! of it. The code holds the whole sketch in `// @sketch` comments, so
//! that the editor reopens it with its constraints.
//!
//! This crate is the editor's core, without any window: the model
//! ([`Sketch`]), the solver ([`solve`]), snapping ([`snap()`]), profiles
//! ([`profile()`]), the code ([`to_code`], [`extrude_code`],
//! [`revolve_code`], [`update_source`]) and reading it back
//! ([`from_code`]).
//!
//! ```
//! use c2f_sketch::{Constraint, Dimension, Point, Sketch, Plane, solve, to_code, from_code};
//!
//! let mut sketch = Sketch::new(Plane::Xy);
//! let [bottom, right, ..] = sketch.add_rectangle(Point::new(0.0, 0.0), Point::new(38.0, 21.0));
//! sketch.add_constraint(Constraint::Fixed(c2f_sketch::PointId(0)));
//! sketch.add_constraint(Constraint::Length(bottom, Dimension::named("plate_width", 40.0)));
//! sketch.add_constraint(Constraint::Length(right, Dimension::named("plate_depth", 20.0)));
//! let report = solve(&mut sketch);
//! assert!(report.is_fully_constrained());
//!
//! let code = to_code(&sketch, "plate");
//! assert!(code.contains("plate_width = 40;"));
//! assert!(code.contains("[plate_width, plate_depth]"));
//! let reopened = from_code(&code, "sketch_plate").unwrap();
//! assert_eq!(reopened.constraints, sketch.constraints);
//! ```
//!
//! # Decisions
//!
//! **The solver is written here rather than taken from a crate.** Of the
//! pure-Rust 2D constraint solvers on crates.io (October 2026), `ezpz`
//! (Zoo's, behind KCL) is maintained but built for its own language's
//! model and brings its dependencies; `arael-sketch-solver` sits on a
//! symbolic optimization framework with procedural macros; `sketch-solver`
//! and `acs-solver` are young and little used. None reports which points
//! remain free nor which constraints conflict in the form an editor needs
//! to color the geometry, which is half the work. Sketches are small
//! (tens of entities), so a dense damped least-squares solver with exact
//! derivatives is a few hundred lines, needs no dependency, solves a
//! 50-entity sketch in well under 5 ms, and leaves its numerics in our
//! hands: minimal movement from the drawn geometry, held points while
//! dragging, the rank analysis behind degrees of freedom and conflicts.
//! See [`solver`] for how it works.
//!
//! **The sketch is stored in the generated module, as `// @sketch`
//! comment lines**, one per point, entity and constraint (see [`mod@format`]).
//! The design stays one self-contained, readable `.c2f` file that diffs
//! well and that the reference implementation renders unchanged; a side
//! file would drift from the code it describes. The lines follow the
//! syntax of the electronics annotations (`// @pin ...`), which ignore
//! other `@` words. No serde: the format is small enough to read by hand,
//! and a line-oriented text reads better than JSON in a comment.
//!
//! **Dimension variables are top-level parameters** written above the
//! file's first module, the only place the Customizer reads them; each
//! named dimension is one, and the polygon's points are expressions of
//! them when that is exact (see [`codegen`]). Reopening a sketch reads
//! the variables' values from the code, so a dimension edited in the
//! code or the Customizer is the one the reopened sketch is solved with.
//!
//! # Limitations
//!
//! - Points follow a variable in the code only when their coordinates
//!   are affine in it (lengths and distances on a profile whose
//!   directions are held). Angles, and radii that change an arc's sweep,
//!   apply when the sketch is reopened and generated again. A loop whose
//!   geometry is not fully constrained is written in numbers: how its
//!   free points would move is the solver's choice, not the sketch's.
//! - A variable whose value in the code is an expression (`w = 2 * t;`)
//!   is not read back: the sketch keeps its stored value.
//! - Profiles are polygons: arcs and circles are cut into segments
//!   (`$fn`-like, [`DEFAULT_SEGMENTS`] per turn) when the code is
//!   generated, so the code's circles do not follow `$fn`.
//! - Lines and arcs that cross each other are not split: a loop is a
//!   chain of entities meeting end to end.
//! - Tangency keeps the side (and, between circles, inside or outside)
//!   the geometry starts on; a perpendicular or angle constraint may be
//!   met with a line reversed (at 180° more).

pub mod codegen;
pub mod format;
mod model;
pub mod profile;
pub mod snap;
pub mod solver;

pub use codegen::{
    Code, ExtrudeMode, Parameter, extrude_code, insert_parameters, module_name, placement, revolve_code, to_code,
    to_code_parts, update_source,
};
pub use format::{from_code, sketches_in};
pub use model::{
    ConstraintId, Constraint, Dimension, Entity, EntityId, Geometry, Plane, Point, PointId, Sketch, SketchError,
    is_identifier,
};
pub use profile::{DEFAULT_SEGMENTS, Loop, Profile, profile};
pub use snap::{Snap, SnapKind, snap};
pub use solver::{SolveOptions, SolveReport, Status, solve, solve_with};
