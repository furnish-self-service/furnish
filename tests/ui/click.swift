// A mouse click at a point on the screen: `click <x> <y>` (tests/self-update-macos.sh). The menu
// bar icon opens its menu on a real click only; the accessibility API's press doesn't reach it.
import CoreGraphics
import Foundation

let point = CGPoint(x: Double(CommandLine.arguments[1])!, y: Double(CommandLine.arguments[2])!)
for type in [CGEventType.mouseMoved, .leftMouseDown, .leftMouseUp] {
    CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: point, mouseButton: .left)!.post(tap: .cghidEventTap)
    usleep(100_000)
}
