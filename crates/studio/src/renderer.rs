//! OpenGL drawing of the 3D view, run from an egui paint callback.
//!
//! Meshes are uploaded once, when a new scene arrives, and drawn every
//! frame: a background gradient, the grid, the opaque model, the axes,
//! then the translucent parts with depth writes off.

use eframe::egui_glow::ShaderVersion;
use eframe::glow::{self, HasContext};
use glam::{Mat4, Vec3};
use c2f_geom::RenderMesh;

use crate::worker::Scene;

/// Vertex layout of meshes: position, normal, RGBA.
const MESH_FLOATS: usize = 10;
/// Vertex layout of lines: position, RGBA.
const LINE_FLOATS: usize = 7;

const ATTR_POSITION: u32 = 0;
const ATTR_NORMAL: u32 = 1;
const ATTR_COLOR: u32 = 2;

const BACKGROUND_TOP: [f32; 3] = [0.215, 0.240, 0.290];
const BACKGROUND_BOTTOM: [f32; 3] = [0.078, 0.086, 0.106];

/// What to draw this frame.
#[derive(Clone, Copy)]
pub struct Frame {
    pub view: Mat4,
    pub projection: Mat4,
    pub show_grid: bool,
    pub show_axes: bool,
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
}

struct LineProgram {
    program: glow::Program,
    view: Option<glow::UniformLocation>,
    projection: Option<glow::UniformLocation>,
    fade: Option<glow::UniformLocation>,
    bias: Option<glow::UniformLocation>,
}

struct BackgroundProgram {
    program: glow::Program,
    top: Option<glow::UniformLocation>,
    bottom: Option<glow::UniformLocation>,
    vao: glow::VertexArray,
}

/// Which fragments of a mesh a draw keeps.
#[derive(Clone, Copy)]
enum Pass {
    Opaque = 0,
    Translucent = 1,
    All = 2,
}

pub struct Renderer {
    mesh: MeshProgram,
    line: LineProgram,
    background: BackgroundProgram,
    solid: Option<GpuBuffer>,
    highlights: Option<GpuBuffer>,
    backgrounds: Option<GpuBuffer>,
    grid_lines: Option<GpuBuffer>,
    axes: Option<GpuBuffer>,
    grid: Option<Grid>,
    /// Waiting for the next paint to reach the GPU.
    pending_scene: Option<Scene>,
    pending_grid: Option<Grid>,
}

impl Renderer {
    pub fn new(gl: &glow::Context) -> Result<Self, String> {
        let version = ShaderVersion::get(gl);
        if !version.is_new_shader_interface() {
            return Err(format!("OpenGL shading language {version:?} is too old for the 3D view"));
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
                    program: mesh,
                },
                line: LineProgram {
                    view: gl.get_uniform_location(line, "u_view"),
                    projection: gl.get_uniform_location(line, "u_projection"),
                    fade: gl.get_uniform_location(line, "u_fade"),
                    bias: gl.get_uniform_location(line, "u_bias"),
                    program: line,
                },
                background: BackgroundProgram {
                    top: gl.get_uniform_location(background, "u_top"),
                    bottom: gl.get_uniform_location(background, "u_bottom"),
                    program: background,
                    vao: background_vao,
                },
                solid: None,
                highlights: None,
                backgrounds: None,
                grid_lines: None,
                axes: None,
                grid: None,
                pending_scene: None,
                pending_grid: None,
            })
        }
    }

    /// Replaces the meshes on the next paint.
    pub fn set_scene(&mut self, scene: Scene) {
        self.pending_scene = Some(scene);
    }

    pub fn set_grid(&mut self, grid: Grid) {
        if self.grid != Some(grid) {
            self.grid = Some(grid);
            self.pending_grid = Some(grid);
        }
    }

    fn upload(&mut self, gl: &glow::Context) {
        if let Some(scene) = self.pending_scene.take() {
            unsafe {
                for buffer in [self.solid.take(), self.highlights.take(), self.backgrounds.take()].into_iter().flatten()
                {
                    delete(gl, buffer);
                }
                self.solid = upload_mesh(gl, &scene.solid);
                self.highlights = upload_mesh(gl, &scene.highlights);
                self.backgrounds = upload_mesh(gl, &scene.backgrounds);
            }
        }
        if let Some(grid) = self.pending_grid.take() {
            unsafe {
                for buffer in [self.grid_lines.take(), self.axes.take()].into_iter().flatten() {
                    delete(gl, buffer);
                }
                self.grid_lines = upload_lines(gl, &grid_vertices(grid));
                self.axes = upload_lines(gl, &axis_vertices(grid.half_size()));
            }
        }
    }

    pub fn paint(&mut self, gl: &glow::Context, frame: &Frame) {
        self.upload(gl);
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
            gl.uniform_3_f32_slice(self.background.top.as_ref(), &BACKGROUND_TOP);
            gl.uniform_3_f32_slice(self.background.bottom.as_ref(), &BACKGROUND_BOTTOM);
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
                    self.draw_lines(gl, frame, grid, fade, 0.0);
                }
            }

            // The opaque part of the model.
            gl.depth_mask(true);
            gl.disable(glow::BLEND);
            if let Some(solid) = &self.solid {
                self.draw_mesh(gl, frame, solid, Pass::Opaque);
            }
            gl.enable(glow::BLEND);

            if frame.show_axes {
                if let Some(axes) = &self.axes {
                    gl.depth_mask(true);
                    self.draw_lines(gl, frame, axes, 0.0, 2e-4);
                }
            }

            // Translucent parts last, without depth writes: back faces, then
            // front faces, so closed shapes blend in a stable order.
            gl.depth_mask(false);
            gl.enable(glow::CULL_FACE);
            for culled in [glow::FRONT, glow::BACK] {
                gl.cull_face(culled);
                if let Some(solid) = &self.solid {
                    self.draw_mesh(gl, frame, solid, Pass::Translucent);
                }
                // `#` and `%` parts often share faces with the model (the
                // walls of a hole they cut): pull them forward to win.
                gl.enable(glow::POLYGON_OFFSET_FILL);
                gl.polygon_offset(-1.0, -4.0);
                if let Some(backgrounds) = &self.backgrounds {
                    self.draw_mesh(gl, frame, backgrounds, Pass::All);
                }
                if let Some(highlights) = &self.highlights {
                    self.draw_mesh(gl, frame, highlights, Pass::All);
                }
                gl.disable(glow::POLYGON_OFFSET_FILL);
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

    unsafe fn draw_mesh(&self, gl: &glow::Context, frame: &Frame, buffer: &GpuBuffer, pass: Pass) {
        unsafe {
            gl.use_program(Some(self.mesh.program));
            gl.uniform_matrix_4_f32_slice(self.mesh.view.as_ref(), false, &frame.view.to_cols_array());
            gl.uniform_matrix_4_f32_slice(self.mesh.projection.as_ref(), false, &frame.projection.to_cols_array());
            gl.uniform_1_i32(self.mesh.pass.as_ref(), pass as i32);
            gl.bind_vertex_array(Some(buffer.vao));
            gl.draw_arrays(glow::TRIANGLES, 0, buffer.count);
        }
    }

    unsafe fn draw_lines(&self, gl: &glow::Context, frame: &Frame, buffer: &GpuBuffer, fade: f32, bias: f32) {
        unsafe {
            gl.use_program(Some(self.line.program));
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

fn grid_vertices(grid: Grid) -> Vec<f32> {
    let half = grid.half_size();
    let mut out = Vec::with_capacity((grid.lines as usize * 2 + 1) * 4 * LINE_FLOATS);
    for i in -grid.lines..=grid.lines {
        let at = i as f32 * grid.step;
        let color = if i % grid.major_every == 0 { [0.62, 0.68, 0.80, 0.30] } else { [0.55, 0.60, 0.70, 0.11] };
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
        data.extend(mesh.colors.get(i).copied().unwrap_or([0.976, 0.843, 0.173, 1.0]));
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
                return Err(format!("shader compilation failed: {log}"));
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
            return Err(format!("shader linking failed: {log}"));
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
out vec3 v_position;
out vec3 v_normal;
out vec4 v_color;
void main() {
    vec4 p = u_view * vec4(a_position, 1.0);
    v_position = p.xyz;
    v_normal = mat3(u_view) * a_normal;
    v_color = a_color;
    gl_Position = u_projection * p;
}
"#;

const MESH_FRAGMENT: &str = r#"
in vec3 v_position;
in vec3 v_normal;
in vec4 v_color;
uniform int u_pass;
out vec4 f_color;
void main() {
    float alpha = v_color.a;
    if (u_pass == 0 && alpha < 0.999) discard;
    if (u_pass == 1 && alpha >= 0.999) discard;
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
void main() {
    v_color = a_color;
    v_world = a_position.xy;
    vec4 p = u_projection * u_view * vec4(a_position, 1.0);
    // Pull lines slightly towards the camera so they win over coplanar faces.
    p.z -= u_bias * p.w;
    gl_Position = p;
}
"#;

const LINE_FRAGMENT: &str = r#"
in vec4 v_color;
in vec2 v_world;
uniform float u_fade;
out vec4 f_color;
void main() {
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
out vec4 f_color;
void main() {
    vec3 color = mix(u_bottom, u_top, smoothstep(0.0, 1.0, v_uv.y));
    vec2 q = v_uv - vec2(0.5);
    color *= 1.0 - 0.35 * dot(q, q);
    f_color = vec4(color, 1.0);
}
"#;
