#!/usr/bin/env swift
import Foundation
import Vision
import CoreImage
import AppKit

guard CommandLine.arguments.count >= 2 else {
    fputs("usage: ocr-helper <image> [x y w h] or ocr-helper <image> --regions <json>\n", stderr)
    exit(1)
}

let imagePath = CommandLine.arguments[1]
guard let nsImage = NSImage(contentsOfFile: imagePath),
      let cgImage = nsImage.cgImage(forProposedRect: nil, context: nil, hints: nil) else {
    fputs("error: cannot load image \(imagePath)\n", stderr)
    exit(1)
}

let imageWidth = CGFloat(cgImage.width)
let imageHeight = CGFloat(cgImage.height)

struct Region: Codable {
    let id: String
    let x: Int
    let y: Int
    let w: Int
    let h: Int
}

struct OCRResult: Codable {
    let id: String
    let text: String
    let confidence: Float
}

var regions: [Region] = []

if CommandLine.arguments.count == 6,
   let x = Int(CommandLine.arguments[2]),
   let y = Int(CommandLine.arguments[3]),
   let w = Int(CommandLine.arguments[4]),
   let h = Int(CommandLine.arguments[5]) {
    regions = [Region(id: "crop", x: x, y: y, w: w, h: h)]
} else if CommandLine.arguments.count >= 3 && CommandLine.arguments[2] == "--regions" {
    let jsonStr = CommandLine.arguments[3]
    if let data = jsonStr.data(using: .utf8) {
        regions = (try? JSONDecoder().decode([Region].self, from: data)) ?? []
    }
} else if CommandLine.arguments.count == 2 {
    regions = [Region(id: "full", x: 0, y: 0, w: Int(imageWidth), h: Int(imageHeight))]
}

var results: [OCRResult] = []
let semaphore = DispatchSemaphore(value: 0)

for region in regions {
    let cropRect = CGRect(
        x: region.x,
        y: region.y,
        width: min(region.w, Int(imageWidth) - region.x),
        height: min(region.h, Int(imageHeight) - region.y)
    )

    guard cropRect.width > 0 && cropRect.height > 0,
          let cropped = cgImage.cropping(to: cropRect) else {
        results.append(OCRResult(id: region.id, text: "", confidence: 0))
        continue
    }

    let request = VNRecognizeTextRequest { request, error in
        defer { semaphore.signal() }
        guard let observations = request.results as? [VNRecognizedTextObservation] else {
            results.append(OCRResult(id: region.id, text: "", confidence: 0))
            return
        }
        let texts = observations.compactMap { obs -> (String, Float)? in
            guard let candidate = obs.topCandidates(1).first else { return nil }
            return (candidate.string, candidate.confidence)
        }
        let combined = texts.map { $0.0 }.joined(separator: " ")
        let avgConf = texts.isEmpty ? 0 : texts.map { $0.1 }.reduce(0, +) / Float(texts.count)
        results.append(OCRResult(id: region.id, text: combined, confidence: avgConf))
    }
    request.recognitionLevel = .accurate
    request.usesLanguageCorrection = true

    let handler = VNImageRequestHandler(cgImage: cropped, options: [:])
    try? handler.perform([request])
    semaphore.wait()
}

let encoder = JSONEncoder()
encoder.outputFormatting = .prettyPrinted
if let data = try? encoder.encode(results),
   let str = String(data: data, encoding: .utf8) {
    print(str)
}
