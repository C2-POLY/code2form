// The 3D view: the engine's mesh drawn with Metal, turned with one
// finger, zoomed with a pinch, moved with two fingers; a double tap frames
// it again.

import MetalKit
import SwiftUI

import Code2FormCore

struct ModelView: UIViewRepresentable {
    let rendering: Rendering

    func makeCoordinator() -> Renderer { Renderer() }

    func makeUIView(context: Context) -> MTKView {
        let view = MTKView(frame: .zero, device: context.coordinator.device)
        view.delegate = context.coordinator
        view.colorPixelFormat = .bgra8Unorm
        view.depthStencilPixelFormat = .depth32Float
        view.sampleCount = 4
        view.clearColor = MTLClearColor(red: 0.16, green: 0.17, blue: 0.19, alpha: 1)
        // Drawn when something changes, not every frame.
        view.enableSetNeedsDisplay = true
        view.isPaused = true

        let renderer = context.coordinator
        let orbit = UIPanGestureRecognizer(target: renderer, action: #selector(Renderer.orbit(_:)))
        orbit.maximumNumberOfTouches = 1
        let pan = UIPanGestureRecognizer(target: renderer, action: #selector(Renderer.pan(_:)))
        pan.minimumNumberOfTouches = 2
        let pinch = UIPinchGestureRecognizer(target: renderer, action: #selector(Renderer.pinch(_:)))
        let frame = UITapGestureRecognizer(target: renderer, action: #selector(Renderer.frame(_:)))
        frame.numberOfTapsRequired = 2
        for gesture in [orbit, pan, pinch, frame] as [UIGestureRecognizer] {
            gesture.delegate = renderer
            view.addGestureRecognizer(gesture)
        }
        renderer.show(rendering, in: view)
        return view
    }

    func updateUIView(_ view: MTKView, context: Context) {
        context.coordinator.show(rendering, in: view)
    }
}

final class Renderer: NSObject, MTKViewDelegate, UIGestureRecognizerDelegate {
    let device = MTLCreateSystemDefaultDevice()!
    private lazy var queue = device.makeCommandQueue()!
    private var pipeline: MTLRenderPipelineState?
    private lazy var depth: MTLDepthStencilState = {
        let descriptor = MTLDepthStencilDescriptor()
        descriptor.depthCompareFunction = .less
        descriptor.isDepthWriteEnabled = true
        return device.makeDepthStencilState(descriptor: descriptor)!
    }()

    private var buffers: (positions: MTLBuffer, normals: MTLBuffer, colors: MTLBuffer)?
    private var vertexCount = 0
    private var bounds: (lo: SIMD3<Float>, hi: SIMD3<Float>)?
    /// The mesh on screen, so that a new rendering frames the view only
    /// when the model's size changes.
    private var shown: Mesh?
    private var camera = Camera()
    /// Framed at the next drawing, once the view has its size.
    private var needsFraming = false

    /// Shows a new rendering; frames it when its size changed.
    func show(_ rendering: Rendering, in view: MTKView) {
        guard shown != rendering.mesh else { return }
        shown = rendering.mesh
        let mesh = rendering.mesh
        vertexCount = mesh.positions.count / 12
        buffers = vertexCount == 0 ? nil : (buffer(mesh.positions), buffer(mesh.normals), buffer(mesh.colors))
        let previous = bounds
        bounds = rendering.bounds.map { b in
            (SIMD3(Float(b.min[0]), Float(b.min[1]), Float(b.min[2])),
             SIMD3(Float(b.max[0]), Float(b.max[1]), Float(b.max[2])))
        }
        if previous == nil || previous.map({ $0.lo != bounds?.lo || $0.hi != bounds?.hi }) == true {
            needsFraming = true
        }
        view.setNeedsDisplay()
    }

    private func buffer(_ data: Data) -> MTLBuffer {
        data.withUnsafeBytes { device.makeBuffer(bytes: $0.baseAddress!, length: data.count, options: .storageModeShared)! }
    }

    private func reset(_ view: MTKView) {
        guard let bounds else { return }
        let size = view.bounds.size
        camera.reset(lo: bounds.lo, hi: bounds.hi, aspect: size.height > 0 ? Float(size.width / size.height) : 1)
    }

    private func makePipeline(_ view: MTKView) -> MTLRenderPipelineState? {
        guard let library = device.makeDefaultLibrary() else { return nil }
        let descriptor = MTLRenderPipelineDescriptor()
        descriptor.vertexFunction = library.makeFunction(name: "model_vertex")
        descriptor.fragmentFunction = library.makeFunction(name: "model_fragment")
        descriptor.colorAttachments[0].pixelFormat = view.colorPixelFormat
        descriptor.depthAttachmentPixelFormat = view.depthStencilPixelFormat
        descriptor.rasterSampleCount = view.sampleCount
        return try? device.makeRenderPipelineState(descriptor: descriptor)
    }

    // MARK: Drawing

    func mtkView(_ view: MTKView, drawableSizeWillChange size: CGSize) {
        view.setNeedsDisplay()
    }

    func draw(in view: MTKView) {
        if pipeline == nil { pipeline = makePipeline(view) }
        if needsFraming, view.bounds.height > 0 {
            reset(view)
            needsFraming = false
        }
        guard let pass = view.currentRenderPassDescriptor, let drawable = view.currentDrawable,
              let commands = queue.makeCommandBuffer(),
              let encoder = commands.makeRenderCommandEncoder(descriptor: pass) else { return }
        if let pipeline, let buffers, let bounds {
            let size = view.drawableSize
            let radius = length(bounds.hi - bounds.lo) / 2 + length(camera.target - (bounds.lo + bounds.hi) / 2)
            var uniforms = (camera.view, camera.projection(aspect: Float(size.width / max(size.height, 1)), radius: radius))
            encoder.setRenderPipelineState(pipeline)
            encoder.setDepthStencilState(depth)
            encoder.setVertexBuffer(buffers.positions, offset: 0, index: 0)
            encoder.setVertexBuffer(buffers.normals, offset: 0, index: 1)
            encoder.setVertexBuffer(buffers.colors, offset: 0, index: 2)
            encoder.setVertexBytes(&uniforms, length: MemoryLayout<simd_float4x4>.stride * 2, index: 3)
            encoder.drawPrimitives(type: .triangle, vertexStart: 0, vertexCount: vertexCount)
        }
        encoder.endEncoding()
        commands.present(drawable)
        commands.commit()
    }

    // MARK: Gestures

    @objc func orbit(_ gesture: UIPanGestureRecognizer) {
        let t = gesture.translation(in: gesture.view)
        gesture.setTranslation(.zero, in: gesture.view)
        camera.orbit(SIMD2(Float(t.x), Float(t.y)))
        gesture.view?.setNeedsDisplay()
    }

    @objc func pan(_ gesture: UIPanGestureRecognizer) {
        guard let view = gesture.view else { return }
        let t = gesture.translation(in: view)
        gesture.setTranslation(.zero, in: view)
        camera.pan(SIMD2(Float(t.x), Float(t.y)), height: Float(view.bounds.height))
        view.setNeedsDisplay()
    }

    @objc func pinch(_ gesture: UIPinchGestureRecognizer) {
        guard gesture.scale > 0 else { return }
        camera.zoom(Float(1 / gesture.scale))
        gesture.scale = 1
        gesture.view?.setNeedsDisplay()
    }

    @objc func frame(_ gesture: UITapGestureRecognizer) {
        guard let view = gesture.view as? MTKView else { return }
        reset(view)
        view.setNeedsDisplay()
    }

    /// Two fingers move and pinch at once.
    func gestureRecognizer(_ gesture: UIGestureRecognizer,
                           shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
        gesture is UIPinchGestureRecognizer || other is UIPinchGestureRecognizer
    }
}
