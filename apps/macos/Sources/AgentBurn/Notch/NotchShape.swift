// Adapted from Codenotch (github.com/vinzdg/codenotch), MIT License, Copyright (c) 2026 Vinz.
import SwiftUI

/// A pill welded to the right edge of the screen, with inverse corners at both
/// ends that flare back out to the edge so it reads as part of the bezel rather
/// than a floating panel. `rect.maxX` is the screen edge.
struct NotchShape: Shape {
  func path(in rect: CGRect) -> Path {
    let curl = NotchModel.curl
    let corner = max(0, min(NotchModel.corner, rect.width / 2, (rect.height - 2 * curl) / 2))
    let flare = max(0, min(curl, rect.width - corner))
    var path = Path()
    path.move(to: CGPoint(x: rect.maxX, y: rect.minY))
    path.addArc(
      center: CGPoint(x: rect.maxX - flare, y: rect.minY), radius: flare,
      startAngle: .degrees(0), endAngle: .degrees(90), clockwise: false)
    path.addLine(to: CGPoint(x: rect.minX + corner, y: rect.minY + flare))
    path.addArc(
      center: CGPoint(x: rect.minX + corner, y: rect.minY + flare + corner), radius: corner,
      startAngle: .degrees(270), endAngle: .degrees(180), clockwise: true)
    path.addLine(to: CGPoint(x: rect.minX, y: rect.maxY - flare - corner))
    path.addArc(
      center: CGPoint(x: rect.minX + corner, y: rect.maxY - flare - corner), radius: corner,
      startAngle: .degrees(180), endAngle: .degrees(90), clockwise: true)
    path.addLine(to: CGPoint(x: rect.maxX - flare, y: rect.maxY - flare))
    path.addArc(
      center: CGPoint(x: rect.maxX - flare, y: rect.maxY), radius: flare,
      startAngle: .degrees(270), endAngle: .degrees(0), clockwise: false)
    path.closeSubpath()
    return path
  }
}
