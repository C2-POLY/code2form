// Draws Code2Form's icon and writes it as an .icns file:
//
//   swift scripts/make-icon.swift crates/studio/assets/AppIcon.icns
//
// A cube in the preview's colors, cut by a cylinder whose walls show the
// green of cut surfaces, on a dark tile like the app's background. Every
// size is drawn at its own resolution rather than scaled down.

import AppKit

let output = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "AppIcon.icns"

func color(_ hex: UInt32, _ alpha: CGFloat = 1) -> CGColor {
    CGColor(
        srgbRed: CGFloat((hex >> 16) & 0xff) / 255,
        green: CGFloat((hex >> 8) & 0xff) / 255,
        blue: CGFloat(hex & 0xff) / 255,
        alpha: alpha)
}

func polygon(_ ctx: CGContext, _ points: [CGPoint]) {
    ctx.beginPath()
    ctx.addLines(between: points)
    ctx.closePath()
}

func fill(_ ctx: CGContext, _ points: [CGPoint], top: UInt32, bottom: UInt32) {
    ctx.saveGState()
    polygon(ctx, points)
    ctx.clip()
    let ys = points.map(\.y)
    let gradient = CGGradient(
        colorsSpace: CGColorSpace(name: CGColorSpace.sRGB), colors: [color(top), color(bottom)] as CFArray,
        locations: [0, 1])!
    ctx.drawLinearGradient(
        gradient, start: CGPoint(x: 0, y: ys.max()!), end: CGPoint(x: 0, y: ys.min()!), options: [])
    ctx.restoreGState()
}

/// Draws the icon in a 1024-point square, y up.
func drawIcon(_ ctx: CGContext) {
    // The tile: macOS icons leave a 100-point margin and round the corners.
    let tile = CGRect(x: 100, y: 100, width: 824, height: 824)
    let tilePath = CGPath(roundedRect: tile, cornerWidth: 185, cornerHeight: 185, transform: nil)
    ctx.saveGState()
    ctx.setShadow(offset: CGSize(width: 0, height: -12), blur: 28, color: color(0x000000, 0.45))
    ctx.addPath(tilePath)
    ctx.setFillColor(color(0x161a22))
    ctx.fillPath()
    ctx.restoreGState()

    ctx.saveGState()
    ctx.addPath(tilePath)
    ctx.clip()
    let background = CGGradient(
        colorsSpace: CGColorSpace(name: CGColorSpace.sRGB), colors: [color(0x2c3850), color(0x12151c)] as CFArray,
        locations: [0, 1])!
    ctx.drawLinearGradient(background, start: CGPoint(x: 0, y: 924), end: CGPoint(x: 0, y: 100), options: [])
    // A soft blue glow behind the cube.
    let glow = CGGradient(
        colorsSpace: CGColorSpace(name: CGColorSpace.sRGB),
        colors: [color(0x4c8dff, 0.38), color(0x4c8dff, 0)] as CFArray, locations: [0, 1])!
    ctx.drawRadialGradient(
        glow, startCenter: CGPoint(x: 512, y: 470), startRadius: 0, endCenter: CGPoint(x: 512, y: 470),
        endRadius: 380, options: [])
    ctx.restoreGState()

    // Thin light rim.
    ctx.addPath(CGPath(roundedRect: tile.insetBy(dx: 2, dy: 2), cornerWidth: 183, cornerHeight: 183, transform: nil))
    ctx.setStrokeColor(color(0xffffff, 0.10))
    ctx.setLineWidth(4)
    ctx.strokePath()

    // The cube, in isometric view: hexagon around `c`, edges `s` long.
    let c = CGPoint(x: 512, y: 470)
    let s: CGFloat = 250
    let dx = s * 0.866, dy = s * 0.5
    let top = CGPoint(x: c.x, y: c.y + s), bottom = CGPoint(x: c.x, y: c.y - s)
    let upperRight = CGPoint(x: c.x + dx, y: c.y + dy), lowerRight = CGPoint(x: c.x + dx, y: c.y - dy)
    let upperLeft = CGPoint(x: c.x - dx, y: c.y + dy), lowerLeft = CGPoint(x: c.x - dx, y: c.y - dy)

    ctx.saveGState()
    ctx.setShadow(offset: CGSize(width: 0, height: -18), blur: 36, color: color(0x000000, 0.5))
    polygon(ctx, [top, upperRight, lowerRight, bottom, lowerLeft, upperLeft])
    ctx.setFillColor(color(0xb8961a))
    ctx.fillPath()
    ctx.restoreGState()

    fill(ctx, [c, upperLeft, top, upperRight], top: 0xffe680, bottom: 0xf6cf2e)  // top face
    fill(ctx, [c, upperLeft, lowerLeft, bottom], top: 0xe6be22, bottom: 0xcda515)  // left face
    fill(ctx, [c, upperRight, lowerRight, bottom], top: 0xbf9a16, bottom: 0x9c7c10)  // right face

    // The hole through the top face: an ellipse of the face's proportions.
    let center = CGPoint(x: c.x, y: c.y + s / 2)
    let a = s * 0.40, b = a / 1.732
    let opening = CGRect(x: center.x - a, y: center.y - b, width: 2 * a, height: 2 * b)
    ctx.saveGState()
    ctx.addEllipse(in: opening)
    ctx.clip()
    // The far wall, lit, in the green of cut surfaces...
    let wall = CGGradient(
        colorsSpace: CGColorSpace(name: CGColorSpace.sRGB), colors: [color(0x6f8f34), color(0xa6d45a)] as CFArray,
        locations: [0, 1])!
    ctx.drawLinearGradient(
        wall, start: CGPoint(x: 0, y: opening.maxY), end: CGPoint(x: 0, y: opening.minY), options: [])
    // ...and, lower down, the shade of the depth.
    ctx.addEllipse(in: opening.offsetBy(dx: 0, dy: -b * 1.35))
    ctx.setFillColor(color(0x3d5320))
    ctx.fillPath()
    ctx.restoreGState()
    ctx.addEllipse(in: opening)
    ctx.setStrokeColor(color(0x8a6d0c, 0.6))
    ctx.setLineWidth(3)
    ctx.strokePath()

    // Light catching the three edges that face the viewer.
    ctx.setLineCap(.round)
    ctx.setStrokeColor(color(0xffffff, 0.55))
    ctx.setLineWidth(5)
    ctx.beginPath()
    ctx.move(to: upperLeft)
    ctx.addLine(to: c)
    ctx.addLine(to: upperRight)
    ctx.move(to: c)
    ctx.addLine(to: bottom)
    ctx.strokePath()
}

func png(size: Int) -> Data {
    let ctx = CGContext(
        data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    ctx.interpolationQuality = .high
    ctx.scaleBy(x: CGFloat(size) / 1024, y: CGFloat(size) / 1024)
    drawIcon(ctx)
    return NSBitmapImageRep(cgImage: ctx.makeImage()!).representation(using: .png, properties: [:])!
}

let iconset = FileManager.default.temporaryDirectory.appendingPathComponent("AppIcon-\(getpid()).iconset")
try? FileManager.default.removeItem(at: iconset)
try FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)
for base in [16, 32, 128, 256, 512] {
    try png(size: base).write(to: iconset.appendingPathComponent("icon_\(base)x\(base).png"))
    try png(size: base * 2).write(to: iconset.appendingPathComponent("icon_\(base)x\(base)@2x.png"))
}
let iconutil = Process()
iconutil.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
iconutil.arguments = ["-c", "icns", iconset.path, "-o", output]
try iconutil.run()
iconutil.waitUntilExit()
try? FileManager.default.removeItem(at: iconset)
if iconutil.terminationStatus != 0 {
    FileHandle.standardError.write("iconutil failed\n".data(using: .utf8)!)
    exit(1)
}
// A large preview next to the icon, for looking at it.
if CommandLine.arguments.count > 2 {
    try png(size: 1024).write(to: URL(fileURLWithPath: CommandLine.arguments[2]))
}
