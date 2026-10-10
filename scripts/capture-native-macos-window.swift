#!/usr/bin/env swift

import AppKit
import CoreGraphics
import Darwin
import Foundation
import ImageIO

private let bundleIdentifier = "com.nexum.desktop"
private let primaryWindowTitle = "Nexum"

private struct Options {
    let appURL: URL
    let outputURL: URL
    let expectedAppearance: String
    let expectedPixels: (width: Int, height: Int)?
}

private struct Window {
    let id: CGWindowID
    let pid: pid_t
    let owner: String
    let title: String?
    let bounds: CGRect
}

private struct CaptureMetadata: Encodable {
    struct Bounds: Encodable {
        let x: Double
        let y: Double
        let width: Double
        let height: Double
    }

    let appBundlePath: String
    let bundleIdentifier: String
    let capturedAt: String
    let owner: String
    let pid: Int32
    let windowId: UInt32
    let windowTitle: String?
    let windowBoundsPoints: Bounds
    let systemAppearance: String
    let pixelWidth: Int
    let pixelHeight: Int
}

private func fail(_ message: String) -> Never {
    fputs("error: \(message)\n", stderr)
    exit(1)
}

private func pathExistsIncludingSymlink(_ path: String) -> Bool {
    var fileStatus = stat()
    return lstat(path, &fileStatus) == 0
}

private func usage() {
    print("""
    Usage: swift scripts/capture-native-macos-window.swift \\
      --app /absolute/path/Nexum.app --output /absolute/path/capture.png \\
      --appearance light|dark [--expect-pixels WIDTHxHEIGHT]

    Captures exactly one visible primary Nexum.app window titled Nexum, without resizing it.
    The PNG includes the native title bar and uses the display's native pixel scale.
    A JSON metadata file is written beside it as capture.png.json.
    The output files must not already exist. Screen Recording permission may be required.
    """)
}

private func parseOptions() -> Options {
    var values: [String: String] = [:]
    let args = Array(CommandLine.arguments.dropFirst())
    if args == ["--help"] || args == ["-h"] {
        usage()
        exit(0)
    }
    guard args.count.isMultiple(of: 2) else {
        usage()
        fail("each option needs a value")
    }
    for index in stride(from: 0, to: args.count, by: 2) {
        let key = args[index]
        guard ["--app", "--output", "--appearance", "--expect-pixels"].contains(key) else {
            fail("unknown option \(key)")
        }
        guard values[key] == nil else { fail("duplicate option \(key)") }
        values[key] = args[index + 1]
    }
    guard let app = values["--app"], app.hasPrefix("/") else {
        fail("--app must be an absolute path to Nexum.app")
    }
    guard let output = values["--output"], output.hasPrefix("/"), output.lowercased().hasSuffix(".png") else {
        fail("--output must be an absolute .png path")
    }
    guard let appearance = values["--appearance"], ["light", "dark"].contains(appearance) else {
        fail("--appearance must be light or dark")
    }

    var pixels: (width: Int, height: Int)?
    if let input = values["--expect-pixels"] {
        let parts = input.split(separator: "x", omittingEmptySubsequences: false)
        guard parts.count == 2,
              let width = Int(parts[0]), width > 0,
              let height = Int(parts[1]), height > 0 else {
            fail("--expect-pixels must be WIDTHxHEIGHT with positive integers")
        }
        pixels = (width, height)
    }

    return Options(
        appURL: URL(fileURLWithPath: app, isDirectory: true).resolvingSymlinksInPath().standardizedFileURL,
        outputURL: URL(fileURLWithPath: output).standardizedFileURL,
        expectedAppearance: appearance,
        expectedPixels: pixels
    )
}

private func windows(for pid: pid_t, owners: Set<String>) -> [Window] {
    guard let raw = CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID) as? [[String: Any]] else {
        fail("macOS did not return the on-screen window list; grant Screen Recording permission and retry")
    }
    return raw.compactMap { info in
        guard (info[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == pid,
              let owner = info[kCGWindowOwnerName as String] as? String,
              owners.contains(owner),
              (info[kCGWindowLayer as String] as? NSNumber)?.intValue == 0,
              let id = (info[kCGWindowNumber as String] as? NSNumber)?.uint32Value,
              let boundsInfo = info[kCGWindowBounds as String] as? [String: Any],
              let bounds = CGRect(dictionaryRepresentation: boundsInfo as CFDictionary),
              bounds.width > 0, bounds.height > 0 else {
            return nil
        }
        return Window(
            id: id,
            pid: pid,
            owner: owner,
            title: info[kCGWindowName as String] as? String,
            bounds: bounds
        )
    }
}

private func capture(_ window: Window, to outputURL: URL) {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
    process.arguments = ["-x", "-o", "-l\(window.id)", outputURL.path]
    let errorPipe = Pipe()
    process.standardError = errorPipe
    do {
        try process.run()
        process.waitUntilExit()
    } catch {
        fail("could not start screencapture: \(error)")
    }
    guard process.terminationStatus == 0 else {
        try? FileManager.default.removeItem(at: outputURL)
        let message = String(data: errorPipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8)?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        fail("screencapture failed (exit \(process.terminationStatus)): \(message)")
    }
}

private let options = parseOptions()
let files = FileManager.default
let metadataURL = URL(fileURLWithPath: options.outputURL.path + ".json")
guard files.fileExists(atPath: "/usr/sbin/screencapture") else {
    fail("/usr/sbin/screencapture is unavailable; run this on macOS")
}
guard files.fileExists(atPath: options.appURL.path),
      options.appURL.lastPathComponent == "Nexum.app",
      let bundle = Bundle(url: options.appURL),
      bundle.bundleIdentifier == bundleIdentifier,
      let owner = bundle.object(forInfoDictionaryKey: "CFBundleExecutable") as? String else {
    fail("--app must point to a built Nexum.app with bundle ID \(bundleIdentifier)")
}
var outputDirectoryIsDirectory: ObjCBool = false
guard files.fileExists(
    atPath: options.outputURL.deletingLastPathComponent().path,
    isDirectory: &outputDirectoryIsDirectory
), outputDirectoryIsDirectory.boolValue else {
    fail("output directory does not exist")
}
guard !pathExistsIncludingSymlink(options.outputURL.path),
      !pathExistsIncludingSymlink(metadataURL.path) else {
    fail("output PNG or metadata already exists; choose a new output path")
}

let actualAppearance: String
switch NSApplication.shared.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) {
case .aqua: actualAppearance = "light"
case .darkAqua: actualAppearance = "dark"
default: fail("could not resolve the current macOS appearance")
}
guard actualAppearance == options.expectedAppearance else {
    fail("macOS appearance is \(actualAppearance), expected \(options.expectedAppearance)")
}

let running = NSRunningApplication.runningApplications(withBundleIdentifier: bundleIdentifier)
    .filter { $0.bundleURL?.resolvingSymlinksInPath().standardizedFileURL == options.appURL }
guard running.count == 1, let app = running.first else {
    fail("expected one running process from \(options.appURL.path), found \(running.count)")
}
let allowedOwners = Set([
    owner,
    app.localizedName,
    bundle.object(forInfoDictionaryKey: "CFBundleName") as? String,
    bundle.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String
].compactMap { $0 })
// macOS accessibility/UI helpers can create tiny layer-0 auxiliary windows in
// the App process. The configured Tauri primary window has the title Nexum;
// still refuse a capture if more than one such primary window is visible.
private let candidates = windows(for: app.processIdentifier, owners: allowedOwners)
    .filter { $0.title == primaryWindowTitle }
guard candidates.count == 1, let window = candidates.first else {
    let ids = candidates.map { String($0.id) }.joined(separator: ", ")
    fail("expected one visible layer-0 Nexum primary window titled \(primaryWindowTitle) for PID \(app.processIdentifier), found \(candidates.count)\(ids.isEmpty ? "" : " (IDs: \(ids))")")
}

capture(window, to: options.outputURL)
guard files.fileExists(atPath: options.outputURL.path),
      let source = CGImageSourceCreateWithURL(options.outputURL as CFURL, nil),
      let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else {
    try? files.removeItem(at: options.outputURL)
    fail("screencapture did not produce a readable PNG")
}
let pixelWidth = image.width
let pixelHeight = image.height
if let expected = options.expectedPixels,
   (pixelWidth != expected.width || pixelHeight != expected.height) {
    try? files.removeItem(at: options.outputURL)
    fail("capture is \(pixelWidth)x\(pixelHeight) pixels, expected \(expected.width)x\(expected.height); no PNG was kept")
}

private let metadata = CaptureMetadata(
    appBundlePath: options.appURL.path,
    bundleIdentifier: bundleIdentifier,
    capturedAt: ISO8601DateFormatter().string(from: Date()),
    owner: window.owner,
    pid: Int32(window.pid),
    windowId: window.id,
    windowTitle: window.title,
    windowBoundsPoints: .init(
        x: Double(window.bounds.origin.x),
        y: Double(window.bounds.origin.y),
        width: Double(window.bounds.width),
        height: Double(window.bounds.height)
    ),
    systemAppearance: actualAppearance,
    pixelWidth: pixelWidth,
    pixelHeight: pixelHeight
)
do {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    try encoder.encode(metadata).write(to: metadataURL, options: .atomic)
} catch {
    try? files.removeItem(at: options.outputURL)
    fail("could not write capture metadata: \(error)")
}
print("Captured \(options.outputURL.path) (\(pixelWidth)x\(pixelHeight) pixels, \(Int(window.bounds.width))x\(Int(window.bounds.height)) window points, \(actualAppearance), PID \(window.pid), window \(window.id))")
print("Metadata: \(metadataURL.path)")
