import AVFoundation
import CoreMedia
import ImageIO
import ReplayKit
@testable import Scrawl
import Testing
import UIKit
import UniformTypeIdentifiers

/// The movies the Mac builds bundles from, written from synthetic frames. One at a time: a
/// broadcast session marks itself as the one recording, in the app group.
@Suite(.serialized)
struct MovieTests {
  func folder() throws -> URL {
    let folder = FileManager.default.temporaryDirectory.appending(path: "movies-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
    return folder
  }

  @Test func framesFitTheLongEdgeInEvenPixels() {
    #expect(BroadcastRecorder.outputSize(width: 1206, height: 2622, maxLongEdge: 1920) == CGSize(width: 884, height: 1920))
    #expect(BroadcastRecorder.outputSize(width: 2622, height: 1206, maxLongEdge: 1920) == CGSize(width: 1920, height: 884))
    #expect(BroadcastRecorder.outputSize(width: 750, height: 1334, maxLongEdge: 1920) == CGSize(width: 750, height: 1334))
    #expect(BroadcastRecorder.outputSize(width: 1321, height: 2869, maxLongEdge: 4000) == CGSize(width: 1322, height: 2870))
  }

  @Test func theScreenInPointsComesFromItsPixels() {
    #expect(BroadcastSession.screen(pixels: CGSize(width: 1206, height: 2622))! == (402, 874, 3))
    #expect(BroadcastSession.screen(pixels: CGSize(width: 750, height: 1334))! == (375, 667, 2))
    #expect(BroadcastSession.screen(pixels: CGSize(width: 828, height: 1792))! == (414, 896, 2))
    #expect(BroadcastSession.screen(pixels: nil) == nil)
  }

  @Test func turnedFramesPlayUpright() {
    #expect(BroadcastRecorder.transform(for: nil) == .identity)
    #expect(BroadcastRecorder.transform(for: .up) == .identity)
    #expect(BroadcastRecorder.transform(for: .right) == CGAffineTransform(rotationAngle: .pi / 2))
    #expect(BroadcastRecorder.isLandscape(.left) && !BroadcastRecorder.isLandscape(.up))
  }

  @Test func aBroadcastBecomesAScaledH264MovieWithTheMicrophone() async throws {
    let folder = try folder()
    defer { try? FileManager.default.removeItem(at: folder) }
    let url = folder.appending(path: "broadcast.mp4")
    let recorder = BroadcastRecorder(url: url)
    let start = CMTime(value: 5_000, timescale: 1_000)
    for frame in 0 ..< 90 {
      // ReplayKit can deliver 60 a second; the movie keeps 30.
      let time = CMTimeAdd(start, CMTime(value: CMTimeValue(frame) * 1_000, timescale: 60_000))
      recorder.appendVideo(Synthetic.video(width: 1206, height: 2622, luma: UInt8(16 + frame % 200), at: time, orientation: frame < 60 ? .up : .left))
      recorder.appendAudio(Synthetic.audio(frames: 735, at: CMTimeAdd(start, CMTime(value: CMTimeValue(frame) * 735, timescale: 44_100))))
      // Roughly live, as frames arrive: a real-time input drops what the encoder isn't ready for.
      try await Task.sleep(for: .milliseconds(4))
    }
    #expect(recorder.sourceSize == CGSize(width: 1206, height: 2622))
    let movie = try recorder.finish()
    #expect(movie.size == CGSize(width: 884, height: 1920))
    #expect(movie.audio)
    #expect((38 ... 45).contains(movie.frames))
    #expect(abs(movie.duration - 1.5) < 0.1)
    #expect(recorder.rotations.count == 1)
    #expect(recorder.rotations.first.map { BroadcastRecorder.isLandscape($0.orientation) } == true)

    let asset = AVURLAsset(url: url)
    let video = try #require(try await asset.loadTracks(withMediaType: .video).first)
    #expect(try await video.load(.naturalSize) == CGSize(width: 884, height: 1920))
    let audio = try #require(try await asset.loadTracks(withMediaType: .audio).first)
    let format = try #require(try await audio.load(.formatDescriptions).first)
    #expect(CMAudioFormatDescriptionGetStreamBasicDescription(format)?.pointee.mSampleRate == 48_000)
  }

  @Test func narrationOverAStillScreenIsKept() async throws {
    let folder = try folder()
    defer { try? FileManager.default.removeItem(at: folder) }
    let url = folder.appending(path: "still.mp4")
    let recorder = BroadcastRecorder(url: url)
    // ReplayKit sends frames only while the screen changes: half a second of them, then
    // nothing while the talking goes on for another second.
    for chunk in 0 ..< 45 {
      if chunk < 15 {
        recorder.appendVideo(Synthetic.video(width: 1206, height: 2622, luma: UInt8(40 + chunk), at: CMTime(value: CMTimeValue(chunk), timescale: 30), orientation: .up))
      }
      recorder.appendAudio(Synthetic.audio(frames: 1470, at: CMTime(value: CMTimeValue(chunk) * 1470, timescale: 44_100)))
      try await Task.sleep(for: .milliseconds(4))
    }
    let movie = try recorder.finish()
    #expect(abs(movie.duration - 1.5) < 0.05)
    let asset = AVURLAsset(url: url)
    #expect(abs(try await asset.load(.duration).seconds - 1.5) < 0.05)
    let audio = try #require(try await asset.loadTracks(withMediaType: .audio).first)
    #expect(try await audio.load(.timeRange).end.seconds > 1.4)
  }

  @Test func withTheMicrophoneOffTheMovieHasNoAudioTrack() async throws {
    let folder = try folder()
    defer { try? FileManager.default.removeItem(at: folder) }
    let url = folder.appending(path: "silent.mp4")
    let recorder = BroadcastRecorder(url: url)
    for frame in 0 ..< 10 {
      recorder.appendVideo(Synthetic.video(width: 1206, height: 2622, luma: 80, at: CMTime(value: CMTimeValue(frame), timescale: 30), orientation: nil))
    }
    let movie = try recorder.finish()
    #expect(!movie.audio)
    #expect(try await AVURLAsset(url: url).loadTracks(withMediaType: .audio).isEmpty)
  }

  @Test func aBroadcastSessionWritesTheMovieAndTimelineAndKeepsThemUntilPaired() async throws {
    let store = CaptureStore(root: try folder(), pings: false)
    defer { try? FileManager.default.removeItem(at: store.root) }
    let session = try BroadcastSession(store: store, pairing: nil, destination: Destination(session: "ses_1", title: "SCRAWL"))
    #expect(store.load(session.capture.id)?.state == .recording)
    #expect(ActiveBroadcast.current() == session.capture.id)
    let start = CMTime(value: 1, timescale: 1)
    for frame in 0 ..< 45 {
      let time = CMTimeAdd(start, CMTime(value: CMTimeValue(frame), timescale: 30))
      session.video(Synthetic.video(width: 1206, height: 2622, luma: UInt8(30 + frame), at: time, orientation: .up))
      session.microphone(Synthetic.audio(frames: 1470, at: CMTimeAdd(start, CMTime(value: CMTimeValue(frame) * 1470, timescale: 44_100))))
      if frame == 20 { session.flag() }
      try await Task.sleep(for: .milliseconds(8))
    }
    session.finish()
    session.finish()

    let capture = try #require(store.load(session.capture.id))
    #expect(capture.state == .notPaired)
    #expect(abs((capture.duration ?? 0) - 1.5) < 0.1)
    #expect(ActiveBroadcast.current() == nil)
    let json = try #require(try JSONSerialization.jsonObject(with: Data(contentsOf: store.timeline(capture.id))) as? [String: Any])
    #expect(json["capture"] as? String == "broadcast")
    #expect(json["session"] as? String == "ses_1")
    #expect(json["audio"] as? Bool == true)
    #expect((json["markers"] as? [Double])?.count == 1)
    #expect((json["screen"] as? [String: Double]) == ["width": 402, "height": 874, "scale": 3])
    #expect(FileManager.default.fileExists(atPath: store.movie(capture.id).path))
  }

  @Test func aBroadcastStoppedBeforeItsFirstFrameIsForgotten() throws {
    let store = CaptureStore(root: try folder(), pings: false)
    defer { try? FileManager.default.removeItem(at: store.root) }
    let session = try BroadcastSession(store: store, pairing: nil, destination: nil)
    session.finish()
    #expect(store.all().isEmpty)
  }

  @Test func stoppingBeforeTheFirstFrameLeavesNothing() throws {
    let folder = try folder()
    defer { try? FileManager.default.removeItem(at: folder) }
    let recorder = BroadcastRecorder(url: folder.appending(path: "empty.mp4"))
    #expect(throws: BroadcastRecorder.RecorderError.self) { try recorder.finish() }
    #expect(!FileManager.default.fileExists(atPath: folder.appending(path: "empty.mp4").path))
  }

  @Test func screenshotsBecomeAMovieOfTwoSecondsEach() async throws {
    let folder = try folder()
    defer { try? FileManager.default.removeItem(at: folder) }
    let files = try [UIColor.red, .green, .blue].enumerated().map { index, color in
      let file = folder.appending(path: "shot-\(index).png")
      try Synthetic.png(color: color, size: CGSize(width: 1206, height: 2622)).write(to: file)
      return file
    }
    let url = folder.appending(path: "stills.mp4")
    let movie = try await StillsMovie.write(files, to: url)
    #expect(movie.duration == 6)
    #expect(movie.size == CGSize(width: 884, height: 1920))
    let asset = AVURLAsset(url: url)
    #expect(abs(try await asset.load(.duration).seconds - 6) < 0.05)
    // The middle screenshot is on screen at 3 s.
    let generator = AVAssetImageGenerator(asset: asset)
    generator.requestedTimeToleranceBefore = .zero
    generator.requestedTimeToleranceAfter = .zero
    let (frame, _) = try await generator.image(at: CMTime(seconds: 3, preferredTimescale: 600))
    let pixel = try #require(Synthetic.centerPixel(frame))
    #expect(pixel.green > 150 && pixel.red < 80 && pixel.blue < 80)
  }

  @Test func sharedScreenshotsAreFlaggedAndNamed() async throws {
    let store = CaptureStore(root: try folder(), pings: false)
    defer { try? FileManager.default.removeItem(at: store.root) }
    let files = try (0 ..< 2).map { index in
      let file = store.root.appending(path: "shot-\(index).png")
      try Synthetic.png(color: .white, size: CGSize(width: 1206, height: 2622)).write(to: file)
      return file
    }
    let capture = try await SharedCapture.screenshots(files, note: "The badge is cut off", destination: Destination(session: "ses_1", title: "SCRAWL"), store: store)
    #expect(capture.state == .waiting)
    #expect(capture.screenshots == 2)
    #expect(capture.note == "The badge is cut off")
    let json = try #require(try JSONSerialization.jsonObject(with: Data(contentsOf: store.timeline(capture.id))) as? [String: Any])
    #expect(json["capture"] as? String == "shared")
    #expect(json["session"] as? String == "ses_1")
    #expect(json["note"] as? String == "The badge is cut off")
    #expect(json["markers"] as? [Double] == [0.5, 2.5])
    #expect((json["screens"] as? [[String: Any]])?.map { $0["name"] as? String } == ["Screenshot 1 of 2", "Screenshot 2 of 2"])
  }
}

/// Buffers and images shaped like ReplayKit's and a phone's screenshots.
enum Synthetic {
  static func video(width: Int, height: Int, luma: UInt8, at time: CMTime, orientation: CGImagePropertyOrientation?) -> CMSampleBuffer {
    var pixels: CVPixelBuffer?
    CVPixelBufferCreate(nil, width, height, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
                        [kCVPixelBufferIOSurfacePropertiesKey as String: [String: Any]()] as CFDictionary, &pixels)
    let buffer = pixels!
    CVPixelBufferLockBaseAddress(buffer, [])
    for plane in 0 ..< 2 {
      let bytes = CVPixelBufferGetBytesPerRowOfPlane(buffer, plane) * CVPixelBufferGetHeightOfPlane(buffer, plane)
      memset(CVPixelBufferGetBaseAddressOfPlane(buffer, plane), plane == 0 ? Int32(luma) : 128, bytes)
    }
    CVPixelBufferUnlockBaseAddress(buffer, [])
    var format: CMVideoFormatDescription?
    CMVideoFormatDescriptionCreateForImageBuffer(allocator: nil, imageBuffer: buffer, formatDescriptionOut: &format)
    var timing = CMSampleTimingInfo(duration: .invalid, presentationTimeStamp: time, decodeTimeStamp: .invalid)
    var sample: CMSampleBuffer?
    CMSampleBufferCreateReadyWithImageBuffer(allocator: nil, imageBuffer: buffer, formatDescription: format!, sampleTiming: &timing, sampleBufferOut: &sample)
    if let orientation {
      CMSetAttachment(sample!, key: RPVideoSampleOrientationKey as CFString, value: NSNumber(value: orientation.rawValue), attachmentMode: kCMAttachmentMode_ShouldPropagate)
    }
    return sample!
  }

  /// 44.1 kHz stereo 16-bit, as the microphone arrives.
  static func audio(frames: Int, at time: CMTime) -> CMSampleBuffer {
    var description = AudioStreamBasicDescription(
      mSampleRate: 44_100, mFormatID: kAudioFormatLinearPCM,
      mFormatFlags: kLinearPCMFormatFlagIsSignedInteger | kLinearPCMFormatFlagIsPacked,
      mBytesPerPacket: 4, mFramesPerPacket: 1, mBytesPerFrame: 4, mChannelsPerFrame: 2,
      mBitsPerChannel: 16, mReserved: 0
    )
    var format: CMAudioFormatDescription?
    CMAudioFormatDescriptionCreate(allocator: nil, asbd: &description, layoutSize: 0, layout: nil, magicCookieSize: 0, magicCookie: nil, extensions: nil, formatDescriptionOut: &format)
    let samples = (0 ..< frames * 2).map { Int16(sin(Double($0 / 2) / 44_100 * 440 * 2 * .pi) * 6_000) }
    let length = samples.count * 2
    var block: CMBlockBuffer?
    CMBlockBufferCreateWithMemoryBlock(allocator: nil, memoryBlock: nil, blockLength: length, blockAllocator: nil, customBlockSource: nil, offsetToData: 0, dataLength: length, flags: 0, blockBufferOut: &block)
    samples.withUnsafeBytes { raw in
      _ = CMBlockBufferReplaceDataBytes(with: raw.baseAddress!, blockBuffer: block!, offsetIntoDestination: 0, dataLength: length)
    }
    var sample: CMSampleBuffer?
    CMAudioSampleBufferCreateReadyWithPacketDescriptions(allocator: nil, dataBuffer: block!, formatDescription: format!, sampleCount: frames, presentationTimeStamp: time, packetDescriptions: nil, sampleBufferOut: &sample)
    return sample!
  }

  static func png(color: UIColor, size: CGSize) -> Data {
    let format = UIGraphicsImageRendererFormat()
    format.scale = 1
    return UIGraphicsImageRenderer(size: size, format: format).pngData { context in
      color.setFill()
      context.fill(CGRect(origin: .zero, size: size))
    }
  }

  static func centerPixel(_ image: CGImage) -> (red: Int, green: Int, blue: Int)? {
    var pixel = [UInt8](repeating: 0, count: 4)
    guard let context = CGContext(data: &pixel, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
                                  space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
    else { return nil }
    context.draw(image, in: CGRect(x: -CGFloat(image.width) / 2, y: -CGFloat(image.height) / 2, width: CGFloat(image.width), height: CGFloat(image.height)))
    return (Int(pixel[0]), Int(pixel[1]), Int(pixel[2]))
  }
}
