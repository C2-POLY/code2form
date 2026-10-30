// A Z-up orbit camera, as the studio's: it turns around a target, from
// the same default angles.

import simd

struct Camera {
    /// The point the camera orbits around and looks at.
    var target = SIMD3<Float>(0, 0, 0)
    var distance: Float = 140
    /// Angle around Z of the camera position, from +X, in radians.
    var yaw: Float = -60 * .pi / 180
    /// Elevation above the XY plane, in radians.
    var pitch: Float = 28 * .pi / 180
    /// Vertical field of view, in radians.
    let fovY: Float = 35 * .pi / 180

    private static let maxPitch: Float = 89.5 * .pi / 180
    private static let distances: ClosedRange<Float> = 1e-3...1e7

    /// From the target towards the camera.
    var back: SIMD3<Float> {
        SIMD3(cos(pitch) * cos(yaw), cos(pitch) * sin(yaw), sin(pitch))
    }

    var eye: SIMD3<Float> { target + back * distance }

    /// The camera's right and up directions in world space.
    private var basis: (right: SIMD3<Float>, up: SIMD3<Float>) {
        let forward = -back
        var right = cross(forward, SIMD3(0, 0, 1))
        right = length(right) > 1e-6 ? normalize(right) : SIMD3(1, 0, 0)
        return (right, cross(right, forward))
    }

    var view: simd_float4x4 {
        let (right, up) = basis
        let forward = -back
        let e = eye
        return simd_float4x4(columns: (
            SIMD4(right.x, up.x, -forward.x, 0),
            SIMD4(right.y, up.y, -forward.y, 0),
            SIMD4(right.z, up.z, -forward.z, 0),
            SIMD4(-dot(right, e), -dot(up, e), dot(forward, e), 1)
        ))
    }

    /// A projection, for Metal's depth from 0 to 1, that keeps everything
    /// within `radius` of the target between the clipping planes.
    func projection(aspect: Float, radius: Float) -> simd_float4x4 {
        let far = (distance + radius) * 1.05
        let near = max(distance - radius, far * 1e-3, 1e-5)
        let y = 1 / tan(fovY / 2)
        let x = y / max(aspect, 1e-3)
        let z = far / (near - far)
        return simd_float4x4(columns: (
            SIMD4(x, 0, 0, 0),
            SIMD4(0, y, 0, 0),
            SIMD4(0, 0, z, -1),
            SIMD4(0, 0, z * near, 0)
        ))
    }

    /// Turns around the target by `delta` points of drag.
    mutating func orbit(_ delta: SIMD2<Float>) {
        let speed: Float = 0.45 * .pi / 180
        yaw = (yaw - delta.x * speed).truncatingRemainder(dividingBy: 2 * .pi)
        pitch = min(max(pitch + delta.y * speed, -Self.maxPitch), Self.maxPitch)
    }

    /// Moves the view by `delta` points in a viewport `height` points high,
    /// so the point under the fingers follows them at the target's depth.
    mutating func pan(_ delta: SIMD2<Float>, height: Float) {
        let perPoint = 2 * distance * tan(fovY / 2) / max(height, 1)
        let (right, up) = basis
        target += (-right * delta.x + up * delta.y) * perPoint
    }

    /// Scales the distance by `factor`.
    mutating func zoom(_ factor: Float) {
        distance = min(max(distance * factor, Self.distances.lowerBound), Self.distances.upperBound)
    }

    /// Back to the default angles, framing the box `lo`...`hi`.
    mutating func reset(lo: SIMD3<Float>, hi: SIMD3<Float>, aspect: Float) {
        yaw = -60 * .pi / 180
        pitch = 28 * .pi / 180
        target = (lo + hi) / 2
        let radius = max(length(hi - lo) / 2, 0.5)
        let halfY = fovY / 2
        let halfX = atan(tan(halfY) * max(aspect, 0.1))
        distance = min(max(radius / sin(min(halfY, halfX)) * 1.08, Self.distances.lowerBound), Self.distances.upperBound)
    }
}
