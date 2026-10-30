// Scanning an object: Apple's guided capture, then the mesh rebuilt on the
// device and saved in millimetres for the studio.

import RealityKit
import SwiftUI

import Code2FormCore

/// Opens a scan, on devices that can capture objects (a LiDAR camera).
struct ScanButton: View {
    var body: some View {
        if ObjectCaptureSession.isSupported {
            Button {
                Cover.present { close in ScanFlow(dismiss: close) }
            } label: {
                Label("Scan an object", systemImage: "camera.metering.matrix")
            }
        }
    }
}

/// A scan from the first photo to the saved mesh.
struct ScanFlow: View {
    /// Closes the scan.
    let dismiss: () -> Void

    private enum Phase {
        case capturing
        case rebuilding(Double)
        case done(Scan)
        case failed(String)
    }

    @State private var phase = Phase.capturing
    /// Released before the rebuild: iOS cannot hold both at once.
    @State private var session: ObjectCaptureSession?
    @State private var folder = ScanFolder()
    @State private var rebuild: Task<Void, Never>?

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()
            switch phase {
            case .capturing:
                if let session {
                    CaptureView(session: session, cancel: cancel)
                }
            case .rebuilding(let fraction):
                VStack(spacing: 16) {
                    ProgressView(value: fraction) {
                        Text("Rebuilding the object")
                    } currentValueLabel: {
                        Text(fraction.formatted(.percent.precision(.fractionLength(0))))
                    }
                    .frame(maxWidth: 320)
                    Button("Cancel", role: .cancel, action: cancel)
                }
                .padding()
                .foregroundStyle(.white)
            case .done(let scan):
                ScanResultView(scan: scan) { dismiss() }
            case .failed(let message):
                VStack(spacing: 16) {
                    Label(message, systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.red)
                    Button("Close") { dismiss() }
                }
                .padding()
            }
        }
        .task {
            guard session == nil, case .capturing = phase else { return }
            guard ObjectCaptureSession.isSupported else {
                phase = .failed("This device cannot scan objects: it needs a LiDAR camera.")
                return
            }
            folder.create()
            let session = ObjectCaptureSession()
            var configuration = ObjectCaptureSession.Configuration()
            configuration.checkpointDirectory = folder.checkpoint
            session.start(imagesDirectory: folder.images, configuration: configuration)
            self.session = session
        }
        .onChange(of: session?.state) { _, state in
            switch state {
            case .completed:
                session = nil
                startRebuild()
            case .failed(let error):
                session = nil
                if case .capturing = phase {
                    folder.remove()
                    phase = .failed("The capture stopped: \(error.localizedDescription)")
                }
            default:
                break
            }
        }
        .statusBarHidden()
    }

    private func startRebuild() {
        phase = .rebuilding(0)
        let folder = folder
        rebuild = Task {
            do {
                let scan = try await Rebuild.run(folder) { fraction in
                    if case .rebuilding = phase { phase = .rebuilding(fraction) }
                }
                folder.remove()
                phase = .done(scan)
            } catch is CancellationError {
                folder.remove()
            } catch {
                folder.remove()
                phase = .failed("The object could not be rebuilt: \(error.localizedDescription)")
            }
        }
    }

    /// Stops the capture or the rebuild and leaves no photos behind.
    private func cancel() {
        session?.cancel()
        session = nil
        rebuild?.cancel()
        folder.remove()
        dismiss()
    }
}

/// The capture: Apple's view of the camera, and what to do next.
private struct CaptureView: View {
    let session: ObjectCaptureSession
    let cancel: () -> Void

    @State private var passDone = false

    var body: some View {
        ZStack {
            ObjectCaptureView(session: session)
                .ignoresSafeArea()
            VStack {
                HStack {
                    Button("Cancel", role: .cancel, action: cancel)
                        .buttonStyle(.borderedProminent)
                        .tint(.black.opacity(0.5))
                    Spacer()
                    if case .capturing = session.state {
                        Text("\(session.numberOfShotsTaken) / \(session.maximumNumberOfInputImages) photos")
                            .font(.footnote.monospacedDigit())
                            .padding(8)
                            .background(.black.opacity(0.5), in: Capsule())
                    }
                }
                Spacer()
                if let advice = advice {
                    Text(advice)
                        .font(.callout)
                        .padding(10)
                        .background(.black.opacity(0.6), in: RoundedRectangle(cornerRadius: 10))
                }
                controls
            }
            .foregroundStyle(.white)
            .padding()
        }
        .onChange(of: session.userCompletedScanPass) { _, done in
            if done { passDone = true }
        }
        .confirmationDialog("The pass is done", isPresented: $passDone, titleVisibility: .visible) {
            Button("Flip the object and scan again") { session.beginNewScanPassAfterFlip() }
            Button("Scan again from another height") { session.beginNewScanPass() }
            Button("Finish") { session.finish() }
        } message: {
            Text("A second pass, with the object on its side, captures its underside.")
        }
    }

    @ViewBuilder
    private var controls: some View {
        switch session.state {
        case .ready:
            prompt("Point at the object, on a plain surface with room around it.") {
                Button("Continue") { _ = session.startDetecting() }
            }
        case .detecting:
            prompt("Fit the box around the object.") {
                Button("Reset") { _ = session.resetDetection() }
                Button("Start the capture") { session.startCapturing() }
            }
        case .capturing:
            prompt("Walk slowly around the object.") {
                Button("Finish") { session.finish() }
            }
        case .initializing, .finishing:
            ProgressView().tint(.white)
        default:
            EmptyView()
        }
    }

    private func prompt(_ text: String, @ViewBuilder buttons: () -> some View) -> some View {
        VStack(spacing: 12) {
            Text(text).multilineTextAlignment(.center)
            HStack(spacing: 12) { buttons() }
                .buttonStyle(.borderedProminent)
        }
    }

    /// What the capture asks of the user now, if anything.
    private var advice: String? {
        let feedback = session.feedback
        if feedback.contains(.environmentTooDark) || feedback.contains(.environmentLowLight) { return "More light" }
        if feedback.contains(.movingTooFast) { return "Move more slowly" }
        if feedback.contains(.objectTooClose) { return "Step back" }
        if feedback.contains(.objectTooFar) { return "Move closer" }
        if feedback.contains(.outOfFieldOfView) { return "Aim at the object" }
        if feedback.contains(.objectNotDetected) { return "The object is not found" }
        if feedback.contains(.objectNotFlippable) { return "This object cannot be flipped: scan it as it stands" }
        return nil
    }
}

/// The scan's working files, removed once its mesh is saved.
struct ScanFolder {
    let root = FileManager.default.temporaryDirectory
        .appendingPathComponent("Scan-\(UUID().uuidString)", isDirectory: true)

    var images: URL { root.appendingPathComponent("Images", isDirectory: true) }
    var checkpoint: URL { root.appendingPathComponent("Checkpoint", isDirectory: true) }
    var model: URL { root.appendingPathComponent("model.usdz") }

    func create() {
        for folder in [images, checkpoint] {
            try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        }
    }

    func remove() {
        try? FileManager.default.removeItem(at: root)
    }
}
