//! A Z-up orbit camera.

use glam::{Mat4, Vec2, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// The point the camera orbits around and looks at.
    pub target: Vec3,
    pub distance: f32,
    /// Angle around Z of the camera position, from +X, in radians.
    pub yaw: f32,
    /// Elevation above the XY plane, in radians.
    pub pitch: f32,
    /// Vertical field of view, in radians.
    pub fov_y: f32,
}

const DEFAULT_YAW: f32 = -60.0_f32.to_radians();
const DEFAULT_PITCH: f32 = 28.0_f32.to_radians();
const MAX_PITCH: f32 = 89.5_f32.to_radians();
const MIN_DISTANCE: f32 = 1e-3;
const MAX_DISTANCE: f32 = 1e7;

impl Default for Camera {
    fn default() -> Self {
        Camera {
            target: Vec3::ZERO,
            distance: 140.0,
            yaw: DEFAULT_YAW,
            pitch: DEFAULT_PITCH,
            fov_y: 35.0_f32.to_radians(),
        }
    }
}

impl Camera {
    /// From the target towards the camera.
    pub fn back(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(cp * cy, cp * sy, sp)
    }

    pub fn eye(&self) -> Vec3 {
        self.target + self.back() * self.distance
    }

    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye(), self.target, Vec3::Z)
    }

    /// The camera's right and up directions in world space.
    fn basis(&self) -> (Vec3, Vec3) {
        let forward = -self.back();
        let right = forward.cross(Vec3::Z).normalize_or(Vec3::X);
        let up = right.cross(forward);
        (right, up)
    }

    /// A projection that keeps everything within `radius` of the target
    /// between the clipping planes.
    pub fn projection(&self, aspect: f32, radius: f32) -> Mat4 {
        let far = (self.distance + radius) * 1.05;
        let near = (self.distance - radius).max(far * 1e-3).max(1e-5);
        Mat4::perspective_rh_gl(self.fov_y, aspect.max(1e-3), near, far)
    }

    pub fn orbit(&mut self, delta: Vec2) {
        let speed = 0.45_f32.to_radians();
        self.yaw = (self.yaw - delta.x * speed).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + delta.y * speed).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Moves the view by `delta` points in a viewport `height` points high,
    /// so the point under the cursor follows it at the target's depth.
    pub fn pan(&mut self, delta: Vec2, height: f32) {
        let per_point = 2.0 * self.distance * (self.fov_y * 0.5).tan() / height.max(1.0);
        let (right, up) = self.basis();
        self.target += (-right * delta.x + up * delta.y) * per_point;
    }

    /// Scales the distance by `factor`, keeping `focus` (a point at the
    /// target's depth) fixed on screen.
    pub fn zoom(&mut self, factor: f32, focus: Option<Vec3>) {
        let distance = (self.distance * factor).clamp(MIN_DISTANCE, MAX_DISTANCE);
        let factor = distance / self.distance;
        if let Some(focus) = focus {
            self.target = focus + (self.target - focus) * factor;
        }
        self.distance = distance;
    }

    /// The point at the target's depth under `ndc` (-1..1, y up).
    pub fn point_at(&self, ndc: Vec2, aspect: f32) -> Vec3 {
        let half_height = (self.fov_y * 0.5).tan();
        let (right, up) = self.basis();
        let forward = -self.back();
        let dir = forward + right * (ndc.x * half_height * aspect) + up * (ndc.y * half_height);
        // `dir` has unit length along `forward`, so this lands on the plane
        // through the target facing the camera.
        self.eye() + dir * self.distance
    }

    /// Frames the box `lo`..`hi` in a viewport of the given aspect ratio.
    pub fn frame(&mut self, lo: Vec3, hi: Vec3, aspect: f32) {
        let center = (lo + hi) * 0.5;
        let radius = ((hi - lo).length() * 0.5).max(0.5);
        let half_y = self.fov_y * 0.5;
        let half_x = (half_y.tan() * aspect.max(0.1)).atan();
        let half = half_y.min(half_x);
        self.target = center;
        self.distance = (radius / half.sin() * 1.08).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }

    /// Back to the default angles, framing the box.
    pub fn reset(&mut self, lo: Vec3, hi: Vec3, aspect: f32) {
        self.yaw = DEFAULT_YAW;
        self.pitch = DEFAULT_PITCH;
        self.frame(lo, hi, aspect);
    }

    /// Looks along a fixed direction (yaw, pitch in degrees).
    pub fn look_from(&mut self, yaw_degrees: f32, pitch_degrees: f32) {
        self.yaw = yaw_degrees.to_radians();
        self.pitch = pitch_degrees.to_radians().clamp(-MAX_PITCH, MAX_PITCH);
    }
}
