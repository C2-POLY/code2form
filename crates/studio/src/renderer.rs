//! OpenGL drawing of the 3D view, run from an egui paint callback.
//!
//! Meshes are uploaded once, when a new scene arrives, and drawn every
//! frame: a background gradient, the grid, the opaque model, the axes and
//! the outlines of the print's pauses, a circuit's wires and pin dots,
//! then the translucent parts with depth writes off, the pause planes last.
//!
//! A section cuts the model, or a circuit's enclosure, along a plane in the
//! fragment shader: moving it, or the enclosure's opacity, only changes
//! uniforms, never the meshes uploaded.

use eframe::egui_glow::ShaderVersion;
use eframe::glow::{self, HasContext};
use glam::{Mat4, Vec3};
use c2f_geom::RenderMesh;

use crate::theme::Palette;
use crate::worker::Scene;

/// Vertex layout of meshes: position, normal, RGBA.
const MESH_FLOATS: usize = 10;
/// Vertex layout of lines: position, RGBA.
const LINE_FLOATS: usize = 7;

const ATTR_POSITION: u32 = 0;
const ATTR_NORMAL: u32 = 1;
const ATTR_COLOR: u32 = 2;

/// The print's pauses: a translucent plane and a bright outline at each
/// pause height, in cyan, which stands out from the default yellow model.
const PAUSE_PLANE: [f32; 4] = [0.20, 0.78, 1.0, 0.30];
const PAUSE_OUTLINE: [f32; 4] = [0.45, 0.90, 1.0, 1.0];

/// No cut: everything is drawn.
const NO_CUT: f32 = 1e30;
/// Keeps what lies exactly at the cut, such as a pause plane at its height.
const CUT_TOLERANCE: f32 = 1e-4;
/// No section: no point lies beyond this plane.
const NO_SECTION: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// A cut through the model across an axis, to look inside: what lies
/// beyond `at` along `axis` (0 for x, 1 for y, 2 for z) is not drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Section {
    pub axis: usize,
    pub at: f32,
}

impl Section {
    pub const AXES: [&'static str; 3] = ["X", "Y", "Z"];

    /// The plane as the shader takes it, `[n, d]`: a point `p` is cut
    /// away when `dot(n, p) > d`.
    pub fn plane(self) -> [f32; 4] {
        let mut plane = [0.0; 4];
        plane[self.axis.min(2)] = 1.0;
        plane[3] = self.at + CUT_TOLERANCE;
        plane
    }

    /// Whether `p` is drawn, as the shader decides it: checks the plane.
    #[cfg(test)]
    fn keeps(self, p: [f32; 3]) -> bool {
        let [x, y, z, d] = self.plane();
        x * p[0] + y * p[1] + z * p[2] <= d
    }

    /// A section as a screenshot names it: `z:20`, an axis and where.
    pub fn parse(text: &str) -> Option<Self> {
        let (axis, at) = text.split_once(':')?;
        let axis = Self::AXES.iter().position(|a| a.eq_ignore_ascii_case(axis.trim()))?;
        let at = at.trim().parse::<f32>().ok().filter(|v| v.is_finite())?;
        Some(Section { axis, at })
    }
}

/// What to draw this frame.
#[derive(Clone, Copy)]
pub struct Frame {
    pub view: Mat4,
    pub projection: Mat4,
    pub show_grid: bool,
    pub show_axes: bool,
    pub show_pauses: bool,
    /// The layer view: what lies above this height is not drawn.
    pub cut: Option<f32>,
    /// What a section cuts away: the model, or a circuit's enclosure but
    /// not its parts or wires.
    pub section: Option<Section>,
    /// How opaque a circuit's enclosure is, from 0 to 1.
    pub enclosure_opacity: f32,
    /// The color of the parts without `color()`, in place of
    /// `c2f_geom::DEFAULT_COLOR`.
    pub model_color: [f32; 3],
    /// The theme's colors: the background's and the grid's.
    pub palette: &'static Palette,
}

/// The XY grid, sized to the model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    /// Distance between minor lines.
    pub step: f32,
    /// Minor lines on each side of the origin.
    pub lines: i32,
    /// Every n-th line is a major one.
    pub major_every: i32,
}

impl Grid {
    /// A grid covering the XY footprint of `bounds` around the origin, with
    /// lines at 1, 2 or 5 times a power of ten.
    pub fn for_bounds(bounds: Option<(Vec3, Vec3)>) -> Self {
        let reach = bounds.map_or(10.0, |(lo, hi)| {
            let footprint = lo.x.abs().max(lo.y.abs()).max(hi.x.abs()).max(hi.y.abs());
            footprint.max((hi - lo).max_element() * 0.5)
        });
        let half = (reach * 1.5).max(1e-3);
        let raw = half / 20.0;
        let power = 10f32.powf(raw.log10().floor());
        let (mantissa, major_every) = match raw / power {
            m if m <= 1.0 => (1.0, 10),
            m if m <= 2.0 => (2.0, 5),
            m if m <= 5.0 => (5.0, 2),
            _ => (10.0, 10),
        };
        let step = mantissa * power;
        let lines = ((half / step).ceil() as i32).clamp(1, 400);
        Grid { step, lines, major_every }
    }

    pub fn half_size(&self) -> f32 {
        self.step * self.lines as f32
    }
}

struct GpuBuffer {
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    count: i32,
}

struct MeshProgram {
    program: glow::Program,
    view: Option<glow::UniformLocation>,
    projection: Option<glow::UniformLocation>,
    pass: Option<glow::UniformLocation>,
    cut: Option<glow::UniformLocation>,
    cut_face: Option<glow::UniformLocation>,
    section: Option<glow::UniformLocation>,
    alpha: Option<glow::UniformLocation>,
    /// Where the mesh's vertices go in the model's space, and how its
    /// normals turn: the identity but for a handle's preview.
    model: Option<glow::UniformLocation>,
    normal: Option<glow::UniformLocation>,
    default_color: Option<glow::UniformLocation>,
    model_color: Option<glow::UniformLocation>,
}

struct LineProgram {
    program: glow::Program,
    view: Option<glow::UniformLocation>,
    projection: Option<glow::UniformLocation>,
    fade: Option<glow::UniformLocation>,
    bias: Option<glow::UniformLocation>,
    cut: Option<glow::UniformLocation>,
}

struct BackgroundProgram {
    program: glow::Program,
    top: Option<glow::UniformLocation>,
    bottom: Option<glow::UniformLocation>,
    vignette: Option<glow::UniformLocation>,
    dither: Option<glow::UniformLocation>,
    vao: glow::VertexArray,
}

/// Which fragments of a mesh a draw keeps.
#[derive(Clone, Copy)]
enum Pass {
    Opaque = 0,
    Translucent = 1,
    All = 2,
}

/// How a draw cuts and shades what it draws.
#[derive(Clone, Copy)]
struct Style {
    section: Option<Section>,
    /// The inside of a cut solid shows as the cut face.
    cap: bool,
    /// Scales the vertices' alpha.
    alpha: f32,
    /// Moves the vertices, and turns the normals with them: the identity
    /// but for a handle's preview.
    model: Mat4,
}

impl Style {
    const PLAIN: Style = Style { section: None, cap: false, alpha: 1.0, model: Mat4::IDENTITY };
    const SOLID: Style = Style { section: None, cap: true, alpha: 1.0, model: Mat4::IDENTITY };
}

pub struct Renderer {
    mesh: MeshProgram,
    line: LineProgram,
    background: BackgroundProgram,
    solid: Option<GpuBuffer>,
    highlights: Option<GpuBuffer>,
    backgrounds: Option<GpuBuffer>,
    /// The objects selected or hovered in a design, drawn over the model.
    selection: Option<GpuBuffer>,
    /// How the selection is moved as drawn: where a handle's drag has it,
    /// applied on the GPU so that a drag uploads nothing.
    selection_transform: Mat4,
    pause_planes: Option<GpuBuffer>,
    pause_outlines: Option<GpuBuffer>,
    /// A circuit's wires and pins, drawn over its parts.
    wires: Option<GpuBuffer>,
    pins: Option<GpuBuffer>,
    grid_lines: Option<GpuBuffer>,
    axes: Option<GpuBuffer>,
    grid: Option<Grid>,
    /// Whether the grid uploaded has the dark theme's colors.
    grid_dark: bool,
    /// Waiting for the next paint to reach the GPU.
    pending_scene: Option<Scene>,
    pending_grid: Option<Grid>,
    pending_overlay: Option<(RenderMesh, RenderMesh)>,
    pending_selection: Option<RenderMesh>,
    /// The solid as uploaded, to upload again in other colors.
    solid_mesh: RenderMesh,
    pending_colors: Option<Vec<[f32; 4]>>,
    pending_positions: Option<Vec<[f32; 3]>>,
    /// A design's solid to draw instead of the scene's: its parts apart,
    /// or back together, in the exploded view.
    pending_solid: Option<RenderMesh>,
    /// The scene is a circuit's, and where its enclosure is in the solid:
    /// a section cuts only that.
    circuit: bool,
    enclosure: std::ops::Range<usize>,
}

impl Renderer {
    pub fn new(gl: &glow::Context) -> Result<Self, String> {
        let version = ShaderVersion::get(gl);
        if !version.is_new_shader_interface() {
            return Err(tr!(
                "OpenGL shading language {:?} is too old for the 3D view",
                "Le langage de shaders OpenGL {:?} est trop ancien pour la vue 3D",
                version
            ));
        }
        let header = if version.is_embedded() {
            format!("{}precision highp float;\n", version.version_declaration())
        } else {
            version.version_declaration().to_string()
        };
        unsafe {
            let mesh = compile(
                gl,
                &header,
                MESH_VERTEX,
                MESH_FRAGMENT,
                &[(ATTR_POSITION, "a_position"), (ATTR_NORMAL, "a_normal"), (ATTR_COLOR, "a_color")],
            )?;
            let line = compile(
                gl,
                &header,
                LINE_VERTEX,
                LINE_FRAGMENT,
                &[(ATTR_POSITION, "a_position"), (ATTR_COLOR, "a_color")],
            )?;
            let background = compile(gl, &header, BACKGROUND_VERTEX, BACKGROUND_FRAGMENT, &[])?;
            let background_vao = gl.create_vertex_array()?;
            Ok(Renderer {
                mesh: MeshProgram {
                    view: gl.get_uniform_location(mesh, "u_view"),
                    projection: gl.get_uniform_location(mesh, "u_projection"),
                    pass: gl.get_uniform_location(mesh, "u_pass"),
                    cut: gl.get_uniform_location(mesh, "u_cut"),
                    cut_face: gl.get_uniform_location(mesh, "u_cut_face"),
                    section: gl.get_uniform_location(mesh, "u_section"),
                    alpha: gl.get_uniform_location(mesh, "u_alpha"),
                    model: gl.get_uniform_location(mesh, "u_model"),
                    normal: gl.get_uniform_location(mesh, "u_normal"),
                    default_color: gl.get_uniform_location(mesh, "u_default_color"),
                    model_color: gl.get_uniform_location(mesh, "u_model_color"),
                    program: mesh,
                },
                line: LineProgram {
                    view: gl.get_uniform_location(line, "u_view"),
                    projection: gl.get_uniform_location(line, "u_projection"),
                    fade: gl.get_uniform_location(line, "u_fade"),
                    bias: gl.get_uniform_location(line, "u_bias"),
                    cut: gl.get_uniform_location(line, "u_cut"),
                    program: line,
                },
                background: BackgroundProgram {
                    top: gl.get_uniform_location(background, "u_top"),
                    bottom: gl.get_uniform_location(background, "u_bottom"),
                    vignette: gl.get_uniform_location(background, "u_vignette"),
                    dither: gl.get_uniform_location(background, "u_dither"),
                    program: background,
                    vao: background_vao,
                },
                solid: None,
                highlights: None,
                backgrounds: None,
                selection: None,
                selection_transform: Mat4::IDENTITY,
                pause_planes: None,
                pause_outlines: None,
                wires: None,
                pins: None,
                grid_lines: None,
                axes: None,
                grid: None,
                grid_dark: true,
                pending_scene: None,
                pending_grid: None,
                pending_overlay: None,
                pending_selection: None,
                solid_mesh: RenderMesh::default(),
                pending_colors: None,
                pending_positions: None,
                pending_solid: None,
                circuit: false,
                enclosure: 0..0,
            })
        }
    }

    /// Replaces the meshes on the next paint.
    pub fn set_scene(&mut self, scene: Scene) {
        self.pending_solid = None;
        self.pending_scene = Some(scene);
    }

    /// Recolors the solid on the next paint, one color per vertex, as a
    /// circuit greys out the parts behind a selection.
    pub fn set_solid_colors(&mut self, colors: Vec<[f32; 4]>) {
        self.pending_colors = Some(colors);
    }

    /// Moves the solid's vertices on the next paint, as a part dragged in
    /// a circuit moves before it renders again.
    pub fn set_solid_positions(&mut self, positions: Vec<[f32; 3]>) {
        self.pending_positions = Some(positions);
    }

    /// Draws `solid` instead of the scene's solid from the next paint, until
    /// the next scene: a design's parts apart in the exploded view.
    pub fn set_solid(&mut self, solid: RenderMesh) {
        self.pending_solid = Some(solid);
    }

    /// Replaces the highlight of the selected objects on the next paint.
    pub fn set_selection(&mut self, mesh: RenderMesh) {
        self.pending_selection = Some(mesh);
    }

    /// Draws the highlight of the selected objects moved by `transform`,
    /// as a handle's drag previews it, without uploading it again.
    pub fn set_selection_transform(&mut self, transform: Mat4) {
        self.selection_transform = transform;
    }

    /// Replaces a circuit's wires and pin dots on the next paint.
    pub fn set_overlay(&mut self, wires: RenderMesh, pins: RenderMesh) {
        self.pending_overlay = Some((wires, pins));
    }

    pub fn set_grid(&mut self, grid: Grid) {
        if self.grid != Some(grid) {
            self.grid = Some(grid);
            self.pending_grid = Some(grid);
        }
    }

    fn upload(&mut self, gl: &glow::Context, palette: &Palette) {
        if let Some(scene) = self.pending_scene.take() {
            unsafe {
                for buffer in [
                    self.solid.take(),
                    self.highlights.take(),
                    self.backgrounds.take(),
                    self.pause_planes.take(),
                    self.pause_outlines.take(),
                ]
                .into_iter()
                .flatten()
                {
                    delete(gl, buffer);
                }
                self.solid = upload_mesh(gl, &scene.solid);
                self.highlights = upload_mesh(gl, &scene.highlights);
                self.backgrounds = upload_mesh(gl, &scene.backgrounds);
                if let Some(footprint) = pause_footprint(&scene) {
                    let heights: Vec<f32> = scene.pauses.iter().map(|p| p.z as f32).collect();
                    self.pause_planes = upload_mesh(gl, &pause_planes(&heights, footprint));
                    self.pause_outlines = upload_lines(gl, &pause_outlines(&heights, footprint));
                }
                self.circuit = !scene.parts.is_empty();
                self.enclosure = scene.enclosure.clone();
                // Only a circuit recolors its solid.
                self.solid_mesh = if scene.parts.is_empty() { RenderMesh::default() } else { scene.solid };
            }
        }
        if let Some(solid) = self.pending_solid.take() {
            unsafe {
                if let Some(buffer) = self.solid.take() {
                    delete(gl, buffer);
                }
                self.solid = upload_mesh(gl, &solid);
            }
        }
        let colors = self.pending_colors.take().filter(|c| c.len() == self.solid_mesh.positions.len());
        let positions = self.pending_positions.take().filter(|p| p.len() == self.solid_mesh.positions.len());
        if (colors.is_some() || positions.is_some()) && !self.solid_mesh.positions.is_empty() {
            if let Some(colors) = colors {
                self.solid_mesh.colors = colors;
            }
            if let Some(positions) = positions {
                self.solid_mesh.positions = positions;
            }
            unsafe {
                if let Some(buffer) = self.solid.take() {
                    delete(gl, buffer);
                }
                self.solid = upload_mesh(gl, &self.solid_mesh);
            }
        }
        if let Some(mesh) = self.pending_selection.take() {
            unsafe {
                if let Some(buffer) = self.selection.take() {
                    delete(gl, buffer);
                }
                self.selection = upload_mesh(gl, &mesh);
            }
        }
        if let Some((wires, pins)) = self.pending_overlay.take() {
            unsafe {
                for buffer in [self.wires.take(), self.pins.take()].into_iter().flatten() {
                    delete(gl, buffer);
                }
                self.wires = upload_mesh(gl, &wires);
                self.pins = upload_mesh(gl, &pins);
            }
        }
        if let Some(grid) = self.pending_grid.take() {
            unsafe {
                for buffer in [self.grid_lines.take(), self.axes.take()].into_iter().flatten() {
                    delete(gl, buffer);
                }
                self.grid_lines = upload_lines(gl, &grid_vertices(grid, palette));
                self.axes = upload_lines(gl, &axis_vertices(grid.half_size()));
            }
        }
    }

    pub fn paint(&mut self, gl: &glow::Context, frame: &Frame) {
        if frame.palette.dark != self.grid_dark {
            self.grid_dark = frame.palette.dark;
            self.pending_grid = self.pending_grid.or(self.grid);
        }
        self.upload(gl, frame.palette);
        unsafe {
            // egui leaves the scissor box on our viewport, so this only
            // clears the 3D view.
            gl.depth_mask(true);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::DEPTH_BUFFER_BIT);
            gl.disable(glow::CULL_FACE);

            // Background gradient.
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::BLEND);
            gl.use_program(Some(self.background.program));
            gl.uniform_3_f32_slice(self.background.top.as_ref(), &frame.palette.view_top);
            gl.uniform_3_f32_slice(self.background.bottom.as_ref(), &frame.palette.view_bottom);
            gl.uniform_1_f32(self.background.vignette.as_ref(), frame.palette.view_vignette);
            gl.uniform_1_f32(self.background.dither.as_ref(), frame.palette.view_dither);
            gl.bind_vertex_array(Some(self.background.vao));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);

            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.enable(glow::BLEND);
            gl.blend_func_separate(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA, glow::ONE, glow::ONE_MINUS_SRC_ALPHA);

            let fade = self.grid.map_or(0.0, |g| g.half_size());

            // The grid never hides the model: no depth writes.
            if frame.show_grid {
                if let Some(grid) = &self.grid_lines {
                    gl.depth_mask(false);
                    self.draw_lines(gl, frame, grid, fade, 0.0, false);
                }
            }

            // The opaque part of the model.
            gl.depth_mask(true);
            gl.disable(glow::BLEND);
            if let Some(solid) = &self.solid {
                self.draw_solid(gl, frame, solid, Pass::Opaque);
            }
            for overlay in [&self.wires, &self.pins].into_iter().flatten() {
                self.draw_mesh(gl, frame, overlay, Pass::Opaque, Style::PLAIN);
            }
            gl.enable(glow::BLEND);

            if frame.show_axes {
                if let Some(axes) = &self.axes {
                    gl.depth_mask(true);
                    self.draw_lines(gl, frame, axes, 0.0, 2e-4, false);
                }
            }
            if frame.show_pauses {
                if let Some(outlines) = &self.pause_outlines {
                    gl.depth_mask(false);
                    self.draw_lines(gl, frame, outlines, 0.0, 2e-4, true);
                }
            }

            // Translucent parts last, without depth writes: back faces, then
            // front faces, so closed shapes blend in a stable order.
            gl.depth_mask(false);
            gl.enable(glow::CULL_FACE);
            for culled in [glow::FRONT, glow::BACK] {
                gl.cull_face(culled);
                if let Some(solid) = &self.solid {
                    self.draw_solid(gl, frame, solid, Pass::Translucent);
                }
                // `#` and `%` parts often share faces with the model (the
                // walls of a hole they cut): pull them forward to win.
                gl.enable(glow::POLYGON_OFFSET_FILL);
                gl.polygon_offset(-1.0, -4.0);
                // A design's are cut with it; a circuit's belong to its parts.
                let style = Style { section: frame.section.filter(|_| !self.circuit), ..Style::PLAIN };
                if let Some(backgrounds) = &self.backgrounds {
                    self.draw_mesh(gl, frame, backgrounds, Pass::All, style);
                }
                if let Some(highlights) = &self.highlights {
                    self.draw_mesh(gl, frame, highlights, Pass::All, style);
                }
                if let Some(selection) = &self.selection {
                    gl.polygon_offset(-2.0, -8.0);
                    // A handle's drag moves it as it is drawn.
                    self.draw_mesh(gl, frame, selection, Pass::All, Style { model: self.selection_transform, ..style });
                    gl.polygon_offset(-1.0, -4.0);
                }
                gl.disable(glow::POLYGON_OFFSET_FILL);
                if frame.show_pauses {
                    if let Some(planes) = &self.pause_planes {
                        self.draw_mesh(gl, frame, planes, Pass::All, Style::PLAIN);
                    }
                }
            }

            // Leave the state as egui expects it.
            gl.disable(glow::CULL_FACE);
            gl.cull_face(glow::BACK);
            gl.depth_mask(true);
            gl.disable(glow::DEPTH_TEST);
            gl.bind_vertex_array(None);
            gl.use_program(None);
        }
    }

    /// Draws the solid: a design's cut by the section; a circuit's parts
    /// whole, and its enclosure cut and as opaque as the frame says.
    unsafe fn draw_solid(&self, gl: &glow::Context, frame: &Frame, solid: &GpuBuffer, pass: Pass) {
        let all = 0..solid.count as usize;
        unsafe {
            if !self.circuit {
                let style = Style { section: frame.section, ..Style::SOLID };
                return self.draw_range(gl, frame, solid, all, pass, style);
            }
            let enclosure = self.enclosure.start.min(all.end)..self.enclosure.end.min(all.end);
            self.draw_range(gl, frame, solid, 0..enclosure.start, pass, Style::SOLID);
            if frame.enclosure_opacity > 0.0 {
                // The scene's enclosure is ENCLOSURE_ALPHA see-through.
                let alpha = frame.enclosure_opacity / crate::circuit::ENCLOSURE_ALPHA;
                // Only an opaque enclosure shows its cut face: through a
                // see-through one, its inner walls would all turn red.
                let cap = frame.enclosure_opacity >= 0.999;
                let style = Style { section: frame.section, cap, alpha, ..Style::SOLID };
                self.draw_range(gl, frame, solid, enclosure.clone(), pass, style);
            }
            self.draw_range(gl, frame, solid, enclosure.end..all.end, pass, Style::SOLID);
        }
    }

    unsafe fn draw_mesh(&self, gl: &glow::Context, frame: &Frame, buffer: &GpuBuffer, pass: Pass, style: Style) {
        unsafe { self.draw_range(gl, frame, buffer, 0..buffer.count as usize, pass, style) }
    }

    /// Draws vertices `range` of a mesh, cut at the layer view's height
    /// and by the style's section; the style's `cap` colors the inside of
    /// a cut model as the cut face.
    unsafe fn draw_range(
        &self,
        gl: &glow::Context,
        frame: &Frame,
        buffer: &GpuBuffer,
        range: std::ops::Range<usize>,
        pass: Pass,
        style: Style,
    ) {
        if range.is_empty() {
            return;
        }
        unsafe {
            // Normals turn by the inverse transpose, for a scale along one
            // axis to tilt them as it tilts the faces.
            let model = style.model;
            let normal = glam::Mat3::from_mat4(model).inverse().transpose();
            let normal = if normal.is_finite() { normal } else { glam::Mat3::IDENTITY };
            // Uniforms go to the program in use: set them only once it is
            // ours, or the background's or the lines' program gets them and
            // the last selection's move stays on every mesh.
            gl.use_program(Some(self.mesh.program));
            gl.uniform_matrix_4_f32_slice(self.mesh.model.as_ref(), false, &model.to_cols_array());
            gl.uniform_matrix_3_f32_slice(self.mesh.normal.as_ref(), false, &normal.to_cols_array());
            gl.uniform_1_f32(self.mesh.cut.as_ref(), frame.cut.map_or(NO_CUT, |z| z + CUT_TOLERANCE));
            let cut = frame.cut.is_some() || style.section.is_some();
            gl.uniform_1_i32(self.mesh.cut_face.as_ref(), i32::from(style.cap && cut));
            gl.uniform_4_f32_slice(self.mesh.section.as_ref(), &style.section.map_or(NO_SECTION, Section::plane));
            gl.uniform_1_f32(self.mesh.alpha.as_ref(), style.alpha);
            gl.uniform_matrix_4_f32_slice(self.mesh.view.as_ref(), false, &frame.view.to_cols_array());
            gl.uniform_matrix_4_f32_slice(self.mesh.projection.as_ref(), false, &frame.projection.to_cols_array());
            gl.uniform_1_i32(self.mesh.pass.as_ref(), pass as i32);
            gl.uniform_3_f32_slice(self.mesh.default_color.as_ref(), &c2f_geom::DEFAULT_COLOR[..3]);
            gl.uniform_3_f32_slice(self.mesh.model_color.as_ref(), &frame.model_color);
            gl.bind_vertex_array(Some(buffer.vao));
            gl.draw_arrays(glow::TRIANGLES, range.start as i32, range.len() as i32);
        }
    }

    /// Draws lines; `cut` hides those above the layer view's height.
    unsafe fn draw_lines(&self, gl: &glow::Context, frame: &Frame, buffer: &GpuBuffer, fade: f32, bias: f32, cut: bool) {
        unsafe {
            gl.use_program(Some(self.line.program));
            let cut = frame.cut.filter(|_| cut).map_or(NO_CUT, |z| z + CUT_TOLERANCE);
            gl.uniform_1_f32(self.line.cut.as_ref(), cut);
            gl.uniform_matrix_4_f32_slice(self.line.view.as_ref(), false, &frame.view.to_cols_array());
            gl.uniform_matrix_4_f32_slice(self.line.projection.as_ref(), false, &frame.projection.to_cols_array());
            gl.uniform_1_f32(self.line.fade.as_ref(), fade);
            gl.uniform_1_f32(self.line.bias.as_ref(), bias);
            gl.bind_vertex_array(Some(buffer.vao));
            gl.draw_arrays(glow::LINES, 0, buffer.count);
        }
    }

    pub fn destroy(&mut self, gl: &glow::Context) {
        unsafe {
            for buffer in [
                self.solid.take(),
                self.highlights.take(),
                self.backgrounds.take(),
                self.selection.take(),
                self.pause_planes.take(),
                self.pause_outlines.take(),
                self.wires.take(),
                self.pins.take(),
                self.grid_lines.take(),
                self.axes.take(),
            ]
            .into_iter()
            .flatten()
            {
                delete(gl, buffer);
            }
            gl.delete_program(self.mesh.program);
            gl.delete_program(self.line.program);
            gl.delete_program(self.background.program);
            gl.delete_vertex_array(self.background.vao);
        }
    }
}

/// The rectangle the pause planes cover: the model's XY footprint with a
/// margin, so their outlines show around it. `None` without pauses.
pub fn pause_footprint(scene: &Scene) -> Option<([f32; 2], [f32; 2])> {
    if scene.pauses.is_empty() {
        return None;
    }
    let (lo, hi) = scene.solid.bounds()?;
    let margin = 0.08 * (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1e-3);
    Some(([lo[0] - margin, lo[1] - margin], [hi[0] + margin, hi[1] + margin]))
}

/// The corners of the pause rectangle at height `z`, in order around it.
pub fn pause_corners(z: f32, (lo, hi): ([f32; 2], [f32; 2])) -> [[f32; 3]; 4] {
    [[lo[0], lo[1], z], [hi[0], lo[1], z], [hi[0], hi[1], z], [lo[0], hi[1], z]]
}

fn pause_planes(heights: &[f32], footprint: ([f32; 2], [f32; 2])) -> RenderMesh {
    let mut mesh = RenderMesh::default();
    for &z in heights {
        let [a, b, c, d] = pause_corners(z, footprint);
        mesh.push_triangle([a, b, c], [PAUSE_PLANE; 3]);
        mesh.push_triangle([a, c, d], [PAUSE_PLANE; 3]);
    }
    mesh
}

fn pause_outlines(heights: &[f32], footprint: ([f32; 2], [f32; 2])) -> Vec<f32> {
    let mut out = Vec::with_capacity(heights.len() * 8 * LINE_FLOATS);
    for &z in heights {
        let corners = pause_corners(z, footprint);
        for i in 0..4 {
            for corner in [corners[i], corners[(i + 1) % 4]] {
                out.extend(corner);
                out.extend(PAUSE_OUTLINE);
            }
        }
    }
    out
}

fn grid_vertices(grid: Grid, palette: &Palette) -> Vec<f32> {
    let half = grid.half_size();
    let mut out = Vec::with_capacity((grid.lines as usize * 2 + 1) * 4 * LINE_FLOATS);
    for i in -grid.lines..=grid.lines {
        let at = i as f32 * grid.step;
        let color = if i % grid.major_every == 0 { palette.grid_major } else { palette.grid_minor };
        for [a, b] in [[[at, -half, 0.0], [at, half, 0.0]], [[-half, at, 0.0], [half, at, 0.0]]] {
            out.extend(a);
            out.extend(color);
            out.extend(b);
            out.extend(color);
        }
    }
    out
}

fn axis_vertices(length: f32) -> Vec<f32> {
    let axes = [
        ([1.0, 0.0, 0.0], [0.94, 0.33, 0.35]),
        ([0.0, 1.0, 0.0], [0.40, 0.82, 0.38]),
        ([0.0, 0.0, 1.0], [0.36, 0.56, 1.00]),
    ];
    let mut out = Vec::with_capacity(axes.len() * 4 * LINE_FLOATS);
    for (dir, rgb) in axes {
        let end = |s: f32| [dir[0] * s, dir[1] * s, dir[2] * s];
        // Positive half bright, negative half dim.
        for (to, alpha) in [(length, 1.0), (-length, 0.35)] {
            out.extend([0.0, 0.0, 0.0]);
            out.extend(rgb);
            out.push(alpha);
            out.extend(end(to));
            out.extend(rgb);
            out.push(alpha);
        }
    }
    out
}

unsafe fn upload_mesh(gl: &glow::Context, mesh: &RenderMesh) -> Option<GpuBuffer> {
    let n = mesh.positions.len();
    if n == 0 {
        return None;
    }
    let mut data = Vec::with_capacity(n * MESH_FLOATS);
    for i in 0..n {
        data.extend(mesh.positions[i]);
        data.extend(mesh.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]));
        data.extend(mesh.colors.get(i).copied().unwrap_or(c2f_geom::DEFAULT_COLOR));
    }
    unsafe { upload(gl, &data, &[(ATTR_POSITION, 3, 0), (ATTR_NORMAL, 3, 3), (ATTR_COLOR, 4, 6)], MESH_FLOATS) }
}

unsafe fn upload_lines(gl: &glow::Context, data: &[f32]) -> Option<GpuBuffer> {
    unsafe { upload(gl, data, &[(ATTR_POSITION, 3, 0), (ATTR_COLOR, 4, 3)], LINE_FLOATS) }
}

/// Uploads interleaved vertices; `attributes` are (location, size, offset)
/// in floats.
unsafe fn upload(
    gl: &glow::Context,
    data: &[f32],
    attributes: &[(u32, i32, usize)],
    stride: usize,
) -> Option<GpuBuffer> {
    let count = i32::try_from(data.len() / stride).ok()?;
    if count == 0 {
        return None;
    }
    unsafe {
        let vao = gl.create_vertex_array().ok()?;
        let vbo = gl.create_buffer().ok()?;
        gl.bind_vertex_array(Some(vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck::cast_slice(data), glow::STATIC_DRAW);
        let stride_bytes = (stride * 4) as i32;
        for &(location, size, offset) in attributes {
            gl.enable_vertex_attrib_array(location);
            gl.vertex_attrib_pointer_f32(location, size, glow::FLOAT, false, stride_bytes, (offset * 4) as i32);
        }
        gl.bind_vertex_array(None);
        gl.bind_buffer(glow::ARRAY_BUFFER, None);
        Some(GpuBuffer { vao, vbo, count })
    }
}

unsafe fn delete(gl: &glow::Context, buffer: GpuBuffer) {
    unsafe {
        gl.delete_vertex_array(buffer.vao);
        gl.delete_buffer(buffer.vbo);
    }
}

unsafe fn compile(
    gl: &glow::Context,
    header: &str,
    vertex: &str,
    fragment: &str,
    attributes: &[(u32, &str)],
) -> Result<glow::Program, String> {
    unsafe {
        let program = gl.create_program()?;
        let mut shaders = Vec::new();
        for (kind, source) in [(glow::VERTEX_SHADER, vertex), (glow::FRAGMENT_SHADER, fragment)] {
            let shader = gl.create_shader(kind)?;
            gl.shader_source(shader, &format!("{header}{source}"));
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                let log = gl.get_shader_info_log(shader);
                gl.delete_shader(shader);
                return Err(tr!("shader compilation failed: {}", "échec de la compilation du shader : {}", log));
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        for &(location, name) in attributes {
            gl.bind_attrib_location(program, location, name);
        }
        gl.link_program(program);
        let linked = gl.get_program_link_status(program);
        let log = gl.get_program_info_log(program);
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        if !linked {
            gl.delete_program(program);
            return Err(tr!("shader linking failed: {}", "échec de l’édition de liens du shader : {}", log));
        }
        Ok(program)
    }
}

const MESH_VERTEX: &str = r#"
in vec3 a_position;
in vec3 a_normal;
in vec4 a_color;
uniform mat4 u_view;
uniform mat4 u_projection;
uniform mat4 u_model;
uniform mat3 u_normal;
uniform vec3 u_default_color;
uniform vec3 u_model_color;
out vec3 v_position;
out vec3 v_normal;
out vec4 v_color;
out vec3 v_world;
void main() {
    vec4 world = u_model * vec4(a_position, 1.0);
    v_world = world.xyz;
    vec4 p = u_view * world;
    v_position = p.xyz;
    v_normal = mat3(u_view) * (u_normal * a_normal);
    // The parts without color() in the color picked in the settings.
    bool plain = all(lessThan(abs(a_color.rgb - u_default_color), vec3(0.002)));
    v_color = plain ? vec4(u_model_color, a_color.a) : a_color;
    gl_Position = u_projection * p;
}
"#;

const MESH_FRAGMENT: &str = r#"
in vec3 v_position;
in vec3 v_normal;
in vec4 v_color;
in vec3 v_world;
uniform int u_pass;
uniform float u_cut;
uniform int u_cut_face;
uniform vec4 u_section;
uniform float u_alpha;
out vec4 f_color;
void main() {
    if (v_world.z > u_cut) discard;
    if (dot(u_section.xyz, v_world) > u_section.w) discard;
    float alpha = min(v_color.a * u_alpha, 1.0);
    if (u_pass == 0 && alpha < 0.999) discard;
    if (u_pass == 1 && alpha >= 0.999) discard;
    // A cut model shows the inside of its walls: drawn flat, in red, they
    // read as the cut face, where the cut runs through solid.
    if (u_cut_face == 1 && !gl_FrontFacing) {
        f_color = vec4(0.86, 0.34, 0.30, alpha);
        return;
    }
    vec3 n = normalize(v_normal);
    vec3 to_eye = normalize(-v_position);
    // Two-sided: light whichever side faces the camera.
    if (dot(n, to_eye) < 0.0) n = -n;
    // A headlight slightly above and right of the camera, and a fill light.
    vec3 key = normalize(vec3(0.35, 0.45, 1.0));
    vec3 fill = normalize(vec3(-0.75, -0.35, 0.30));
    float diffuse = max(dot(n, key), 0.0);
    float filled = max(dot(n, fill), 0.0);
    vec3 color = v_color.rgb * (0.30 + 0.62 * diffuse + 0.22 * filled);
    vec3 halfway = normalize(key + to_eye);
    color += vec3(0.16) * pow(max(dot(n, halfway), 0.0), 48.0);
    f_color = vec4(min(color, vec3(1.0)), alpha);
}
"#;

const LINE_VERTEX: &str = r#"
in vec3 a_position;
in vec4 a_color;
uniform mat4 u_view;
uniform mat4 u_projection;
uniform float u_bias;
out vec4 v_color;
out vec2 v_world;
out float v_height;
void main() {
    v_color = a_color;
    v_world = a_position.xy;
    v_height = a_position.z;
    vec4 p = u_projection * u_view * vec4(a_position, 1.0);
    // Pull lines slightly towards the camera so they win over coplanar faces.
    p.z -= u_bias * p.w;
    gl_Position = p;
}
"#;

const LINE_FRAGMENT: &str = r#"
in vec4 v_color;
in vec2 v_world;
in float v_height;
uniform float u_fade;
uniform float u_cut;
out vec4 f_color;
void main() {
    if (v_height > u_cut) discard;
    float alpha = v_color.a;
    if (u_fade > 0.0) {
        float d = length(v_world) / u_fade;
        alpha *= 1.0 - smoothstep(0.45, 1.0, d);
    }
    f_color = vec4(v_color.rgb, alpha);
}
"#;

const BACKGROUND_VERTEX: &str = r#"
out vec2 v_uv;
void main() {
    // One triangle covering the viewport.
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    v_uv = p;
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const BACKGROUND_FRAGMENT: &str = r#"
in vec2 v_uv;
uniform vec3 u_top;
uniform vec3 u_bottom;
uniform float u_vignette;
// Noise of up to a step of the screen's colors, against the bands of a
// light gradient.
uniform float u_dither;
out vec4 f_color;
void main() {
    vec3 color = mix(u_bottom, u_top, smoothstep(0.0, 1.0, v_uv.y));
    vec2 q = v_uv - vec2(0.5);
    color *= 1.0 - u_vignette * dot(q, q);
    float noise = fract(sin(dot(gl_FragCoord.xy, vec2(12.9898, 78.233))) * 43758.5453);
    color += (noise - 0.5) / 255.0 * u_dither;
    f_color = vec4(color, 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_section_cuts_away_what_lies_beyond_it() {
        let section = Section { axis: 2, at: 20.0 };
        assert_eq!(section.plane(), [0.0, 0.0, 1.0, 20.0 + CUT_TOLERANCE]);
        assert!(section.keeps([100.0, -50.0, 19.0]));
        assert!(section.keeps([0.0, 0.0, 20.0]), "what lies at the cut stays");
        assert!(!section.keeps([0.0, 0.0, 21.0]));
        let across_x = Section { axis: 0, at: -5.0 };
        assert!(across_x.keeps([-6.0, 99.0, 99.0]) && !across_x.keeps([0.0, 0.0, 0.0]));
        // No section keeps everything.
        let [x, y, z, d] = NO_SECTION;
        assert!(x * 1e9 + y * 1e9 + z * 1e9 <= d);
    }

    #[test]
    fn a_section_reads_as_an_axis_and_where() {
        assert_eq!(Section::parse("z:20"), Some(Section { axis: 2, at: 20.0 }));
        assert_eq!(Section::parse("X: -12.5"), Some(Section { axis: 0, at: -12.5 }));
        assert_eq!(Section::parse("y:3"), Some(Section { axis: 1, at: 3.0 }));
        assert_eq!(Section::parse("w:3"), None);
        assert_eq!(Section::parse("z"), None);
        assert_eq!(Section::parse("z:inf"), None);
    }
}
