// Adapted from Codenotch (github.com/vinzdg/codenotch), MIT License, Copyright (c) 2026 Vinz.
import AppKit

extension NSScreen {
  /// Unlike `CGDirectDisplayID`, this UUID survives restarts and reconnections.
  var displayIdentifier: String? {
    guard
      let number = deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber,
      let uuid = CGDisplayCreateUUIDFromDisplayID(number.uint32Value)?.takeRetainedValue()
    else { return nil }
    return CFUUIDCreateString(nil, uuid) as String
  }
}
