// The captured photos rebuilt into a mesh on the device, then written as a
// binary STL in millimetres with Z up, the way Code2Form models stand.

import ModelIO
import RealityKit
import simd

import Code2FormCore

/// A saved scan: its STL in Files, and the engine's rendering of it.
struct Scan {
    let file: URL
    let rendering: Rendering
}

enum Rebuild {
    /// Rebuilds the photos of `folder`, reporting the fraction done, and
    /// saves the mesh in the app's Scans folder.
    @MainActor
    static func run(_ folder: ScanFolder, progress: @escaping @MainActor (Double) -> Void) async throws -> Scan {
        var configuration = PhotogrammetrySession.Configuration()
        configuration.checkpointDirectory = folder.checkpoint
        let session = try PhotogrammetrySession(input: folder.images, configuration: configuration)
        try await withTaskCancellationHandler {
            try session.process(requests: [.modelFile(url: folder.model)])
            for try await output in session.outputs {
                switch output {
                case .requestProgress(_, let fraction):
                    progress(fraction)
                case .requestError(_, let error):
                    throw error
                case .processingCancelled:
                    throw CancellationError()
                case .processingComplete:
                    return
                default:
                    break
                }
            }
        } onCancel: {
            session.cancel()
        }

        let file = try scansFolder().appendingPathComponent("scan-\(stamp()).stl")
        try await Task.detached {
            try STL.write(try Triangles(usdz: folder.model).standing(), to: file)
        }.value
        let rendering = try await Task.detached {
            // The engine reads the STL as the studio will: through import().
            let request = Request(
                source: "import(\"\(file.lastPathComponent)\");",
                path: file.deletingLastPathComponent().appendingPathComponent("preview.c2f").path,
                parameters: []
            )
            return try Engines.make().render(request: request)
        }.value
        return Scan(file: file, rendering: rendering)
    }

    /// On My iPhone › Code2Form › Scans.
    private static func scansFolder() throws -> URL {
        let documents = try FileManager.default.url(for: .documentDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        let folder = documents.appendingPathComponent("Scans", isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        return folder
    }

    private static func stamp() -> String {
        let format = DateFormatter()
        format.locale = Locale(identifier: "en_US_POSIX")
        format.dateFormat = "yyyy-MM-dd-HHmmss"
        return format.string(from: Date())
    }
}

/// A triangle soup, in the units and axes it was read in.
struct Triangles {
    var corners: [SIMD3<Float>]

    /// Reads every mesh of a USDZ, placed by its transforms.
    init(usdz url: URL) throws {
        let asset = MDLAsset(url: url)
        corners = []
        for case let mesh as MDLMesh in asset.childObjects(of: MDLMesh.self) {
            let place = MDLTransform.globalTransform(with: mesh, atTime: 0)
            guard let positions = mesh.vertexAttributeData(forAttributeNamed: MDLVertexAttributePosition, as: .float3) else { continue }
            let base = positions.dataStart
            let vertex = { (index: Int) -> SIMD3<Float> in
                let p = base.advanced(by: index * positions.stride).assumingMemoryBound(to: Float.self)
                let placed = place * SIMD4(p[0], p[1], p[2], 1)
                return SIMD3(placed.x, placed.y, placed.z)
            }
            for case let submesh as MDLSubmesh in mesh.submeshes ?? [] where submesh.geometryType == .triangles {
                let indices = submesh.indexBuffer(asIndexType: .uInt32).map()
                let raw = indices.bytes.assumingMemoryBound(to: UInt32.self)
                for i in 0..<submesh.indexCount {
                    corners.append(vertex(Int(raw[i])))
                }
            }
        }
        guard !corners.isEmpty else { throw CocoaError(.fileReadCorruptFile) }
    }

    /// From Apple's metres with Y up to millimetres with Z up, centred on
    /// the Z axis and standing on the XY plane.
    func standing() -> Triangles {
        var turned = corners.map { SIMD3($0.x, -$0.z, $0.y) * 1000 }
        let lo = turned.reduce(SIMD3(repeating: .infinity)) { simd_min($0, $1) }
        let hi = turned.reduce(SIMD3(repeating: -.infinity)) { simd_max($0, $1) }
        let shift = SIMD3((lo.x + hi.x) / 2, (lo.y + hi.y) / 2, lo.z)
        for i in turned.indices { turned[i] -= shift }
        return Triangles(corners: turned)
    }

    private init(corners: [SIMD3<Float>]) {
        self.corners = corners
    }
}

enum STL {
    /// Writes `triangles` as a binary STL.
    static func write(_ triangles: Triangles, to url: URL) throws {
        let count = triangles.corners.count / 3
        var data = Data(capacity: 84 + count * 50)
        var header = Data("Code2Form scan, millimetres".utf8)
        header.count = 80
        data.append(header)
        append(UInt32(count), to: &data)
        for t in 0..<count {
            let a = triangles.corners[3 * t], b = triangles.corners[3 * t + 1], c = triangles.corners[3 * t + 2]
            let normal = cross(b - a, c - a)
            let unit = length(normal) > 0 ? normalize(normal) : SIMD3<Float>(0, 0, 0)
            for v in [unit, a, b, c] {
                append(v.x.bitPattern, to: &data)
                append(v.y.bitPattern, to: &data)
                append(v.z.bitPattern, to: &data)
            }
            append(UInt16(0), to: &data)
        }
        try data.write(to: url, options: .atomic)
    }

    private static func append<T: FixedWidthInteger>(_ value: T, to data: inout Data) {
        withUnsafeBytes(of: value.littleEndian) { data.append(contentsOf: $0) }
    }
}
