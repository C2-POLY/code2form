// The engine called from Swift on the iPad simulator: a render, the
// customizer's parameters, an error at its line, an export.

import Foundation
import XCTest

import Code2FormCore

final class EngineTests: XCTestCase {
    /// A box whose width is a parameter, in a file of a temporary folder.
    let box = """
        // The box's width.
        width = 20; // [10:50]
        /* [Hidden] */
        wall = 2;
        cube([width, 10, 5]);
        """

    var folder: URL!
    var engine: Engine!

    override func setUpWithError() throws {
        folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        engine = try Engine(fonts: [], libraries: [])
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: folder)
    }

    func request(_ source: String, _ parameters: [ParameterValue] = []) -> Request {
        Request(source: source, path: folder.appendingPathComponent("part.c2f").path, parameters: parameters)
    }

    func testRendersAModelWithItsMeshAndSize() throws {
        let rendering = try engine.render(request: request(box))
        XCTAssertNil(rendering.error)
        XCTAssertEqual(rendering.volume, 1000, accuracy: 1e-9)
        XCTAssertEqual(rendering.bounds, Bounds(min: [0, 0, 0], max: [20, 10, 5]))
        // A box is 12 triangles, three vertices each, in 4-byte floats.
        XCTAssertEqual(rendering.mesh.positions.count, 12 * 3 * 3 * 4)
        XCTAssertEqual(rendering.mesh.colors.count, 12 * 3 * 4 * 4)
        let floats = rendering.mesh.positions.withUnsafeBytes { Array($0.bindMemory(to: Float.self)) }
        XCTAssertEqual(floats.max(), 20)
    }

    func testListsTheParametersAndRendersWithAValue() throws {
        let parameters = try engine.parameters(source: box, path: folder.appendingPathComponent("part.c2f").path)
        XCTAssertEqual(parameters.map(\.name), ["width"])
        XCTAssertEqual(parameters[0].value, "20")
        XCTAssertEqual(parameters[0].description, "The box's width.")
        XCTAssertEqual(parameters[0].widget, .slider(min: 10, max: 50, step: 1))

        let wider = try engine.render(request: request(box, [ParameterValue(name: "width", value: "40")]))
        XCTAssertEqual(wider.bounds?.max, [40, 10, 5])
    }

    func testGivesASyntaxErrorAtItsLine() throws {
        let rendering = try engine.render(request: request("cube(10);\nsphere(5;\n"))
        let problem = try XCTUnwrap(rendering.error)
        XCTAssertEqual(problem.line, 2)
        XCTAssertEqual(problem.column, 9)
        XCTAssertTrue(rendering.mesh.positions.isEmpty)

        XCTAssertThrowsError(try engine.parameters(source: "x = ;", path: "part.c2f")) { error in
            guard case EngineError.Program(let problem) = error else { return XCTFail("\(error)") }
            XCTAssertEqual(problem.line, 1)
        }
    }

    func testLeavesOutAValueOfAnotherType() throws {
        let rendering = try engine.render(request: request(box, [ParameterValue(name: "width", value: "\"wide\"")]))
        XCTAssertEqual(rendering.bounds?.max, [20, 10, 5])
        XCTAssertEqual(rendering.warnings.count, 1)
    }

    /// Swift's background threads have 512 KiB of stack, which deep models
    /// once overflowed.
    func testRendersADeepModelFromABackgroundThread() throws {
        let deep = request("module nest(n) { if (n > 0) translate([0.001, 0, 0]) nest(n - 1); else cube(1); }\nnest(1000);")
        let done = expectation(description: "rendered")
        var volume = 0.0
        DispatchQueue.global().async {
            volume = (try? self.engine.render(request: deep).volume) ?? -1
            done.fulfill()
        }
        wait(for: [done], timeout: 60)
        XCTAssertEqual(volume, 1, accuracy: 1e-9)
    }

    func testExportsA3MF() throws {
        let output = folder.appendingPathComponent("box.3mf")
        let written = try engine.export(request: request(box), format: .threeMf, output: output.path)
        let data = try Data(contentsOf: output)
        XCTAssertEqual(UInt64(data.count), written)
        XCTAssertEqual(Array(data.prefix(2)), Array("PK".utf8))
    }
}
