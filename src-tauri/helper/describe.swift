// describe-helper — understands screenshot pictures with Apple's on-device model (Foundation Models).
//
// Protocol (line-based JSON over stdin/stdout, one long-running process):
//   startup → {"ready":true}  or  {"ready":false,"reason":"…"}
//   in:  an image path per line
//   out: {"ok":true,"kind":"photo","subjects":[…],"categories":[…],"summary":"…"}
//        or {"ok":false,"error":"…"}
// Everything runs on this Mac; nothing is sent anywhere.

import CoreGraphics
import Foundation
import FoundationModels
import ImageIO

/// What kind of screenshot it is. Only some kinds become search categories (see `categories`).
@Generable
enum Kind: String {
    case photo, illustration, app_screen, web_page, chat, document, receipt, map, chart, code, other
}

/// Topics of a real photo. Must match category keys in ui/objects.js.
@Generable
enum Topic: String {
    case coffee, tea, alcohol, drink, dessert, food, dog, cat, animal, people
    case vehicle, nature, plant, place, fashion, electronics, sport
}

@Generable
struct Description {
    @Guide(description: "What kind of screenshot this is. Use app_screen, web_page, chat, document, chart or code for interface and text screens even if they contain small icons or pictures.")
    var kind: Kind
    @Guide(description: "The app or website the screenshot was taken in, recognized from its interface, logo or address bar (for example a social network, a messenger, a video app, a web browser, a map, a code editor). Use the product's usual name. Empty string if you cannot tell.")
    var app: String
    @Guide(description: "Main things visible in the photo or main content, as short English nouns. At most 6.")
    var subjects: [String]
    @Guide(description: "Topics of a real photograph shown in the screenshot (people, food, places, animals, objects). Empty unless a real photo is clearly the main content. At most 2.")
    var topics: [Topic]
    @Guide(description: "One short English sentence describing the main content.")
    var summary: String
}

/// Search categories: photo topics (only when the screenshot really is a photo — the model
/// otherwise invents topics for schedules and app screens), plus kinds that are categories themselves.
func categories(_ d: Description) -> [String] {
    var out = d.kind == .photo ? d.topics.map(\.rawValue) : []
    switch d.kind {
    case .illustration: out.append("art")
    case .receipt: out.append("receipt")
    case .map: out.append("map")
    default: break
    }
    return Array(NSOrderedSet(array: out)) as? [String] ?? out
}

let instructions = """
You index screenshots for a personal photo organizer's search. Each image is a screenshot from a \
phone, tablet or computer. Ignore the status bar, navigation bars, app buttons, icons, captions and \
other interface chrome. Describe only the photo or main content. If the screenshot is mostly text or \
an app screen without a photo, describe what kind of screen it is (for example a chat, a receipt, a map).
"""

func emit(_ obj: [String: Any]) {
    guard let data = try? JSONSerialization.data(withJSONObject: obj),
          let line = String(data: data, encoding: .utf8) else { return }
    print(line)
    fflush(stdout)
}

/// Decode and downscale (the model does not need full-resolution pixels; smaller is faster).
func loadImage(_ path: String) -> CGImage? {
    guard let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil) else { return nil }
    let opts: [CFString: Any] = [
        kCGImageSourceCreateThumbnailFromImageAlways: true,
        kCGImageSourceCreateThumbnailWithTransform: true,
        kCGImageSourceThumbnailMaxPixelSize: 1280,
    ]
    return CGImageSourceCreateThumbnailAtIndex(src, 0, opts as CFDictionary)
}

@main
struct DescribeHelper {
    static func main() async {
        // The default guardrails sometimes block harmless images; this is a pure
        // image → description transformation, which is what the permissive mode is for.
        let model = SystemLanguageModel(guardrails: .permissiveContentTransformations)
        guard case .available = model.availability else {
            emit(["ready": false, "reason": "\(model.availability)"])
            return
        }
        emit(["ready": true])

        while let line = readLine() {
            let path = line.trimmingCharacters(in: .whitespacesAndNewlines)
            if path.isEmpty { continue }
            guard let image = loadImage(path) else {
                emit(["ok": false, "error": "cannot decode image"])
                continue
            }
            // A fresh session per image keeps every answer independent (no shared context).
            let session = LanguageModelSession(model: model, instructions: instructions)
            do {
                // Greedy decoding: the same image always gets the same answer.
                let r = try await session.respond(generating: Description.self, options: GenerationOptions(sampling: .greedy)) {
                    "What is this screenshot's photo or main content?"
                    Attachment(image)
                }
                let d = r.content
                emit([
                    "ok": true,
                    "subjects": d.subjects,
                    "kind": d.kind.rawValue,
                    "app": d.app.trimmingCharacters(in: .whitespacesAndNewlines),
                    "categories": categories(d),
                    "summary": d.summary,
                ])
            } catch {
                emit(["ok": false, "error": "\(error)"])
            }
        }
    }
}
