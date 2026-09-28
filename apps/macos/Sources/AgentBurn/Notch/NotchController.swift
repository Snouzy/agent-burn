import AppKit
import Observation
import SwiftUI

@Observable @MainActor final class NotchController {
  static let shared = NotchController()

  private(set) var openAgent: String?
  private var store: UsageStore?
  private var panel: NSPanel?
  private var ringCount = 0
  private var cardHeight: CGFloat?
  private var monitors: [Any] = []
  private var screenObserver: (any NSObjectProtocol)?
  private var pendingClose: Task<Void, Never>?

  private init() {}

  func attach(_ store: UsageStore) { self.store = store }

  func apply(_ on: Bool) {
    if on { show() } else { hide() }
  }

  func open(_ agent: String) {
    cancelClose()
    openAgent = agent
    updatePassThrough()
  }

  func scheduleClose() {
    guard openAgent != nil, pendingClose == nil else { return }
    pendingClose = Task { [weak self] in
      try? await Task.sleep(for: .milliseconds(300))
      guard let self, !Task.isCancelled else { return }
      pendingClose = nil
      openAgent = nil
      updatePassThrough()
    }
  }

  func cancelClose() {
    pendingClose?.cancel()
    pendingClose = nil
  }

  func ringsChanged(_ agents: [String]) {
    ringCount = agents.count
    if let openAgent, !agents.contains(openAgent) {
      cancelClose()
      self.openAgent = nil
    }
    place()
  }

  func cardHeightChanged(_ height: CGFloat?) {
    cardHeight = height
    updatePassThrough()
  }

  private func show() {
    guard panel == nil, let store else { return }
    let panel = NSPanel(
      contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered,
      defer: false)
    panel.isFloatingPanel = true
    panel.level = .statusBar
    panel.appearance = NSAppearance(named: .darkAqua)
    panel.backgroundColor = .clear
    panel.isOpaque = false
    panel.hasShadow = false
    panel.hidesOnDeactivate = false
    panel.isReleasedWhenClosed = false
    panel.collectionBehavior = [
      .canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .ignoresCycle,
    ]
    panel.ignoresMouseEvents = true
    panel.acceptsMouseMovedEvents = true
    let hosting = NSHostingView(rootView: NotchRoot(controller: self, store: store))
    hosting.sizingOptions = []
    // A non-key panel of an inactive app gets mouse-moved events only through a tracking area.
    hosting.addTrackingArea(
      NSTrackingArea(
        rect: .zero, options: [.mouseMoved, .activeAlways, .inVisibleRect], owner: hosting))
    panel.contentView = hosting
    self.panel = panel
    place()
    panel.orderFrontRegardless()

    // AppKit documents no thread for a global monitor; the local one runs inside sendEvent.
    let global = NSEvent.addGlobalMonitorForEvents(matching: .mouseMoved) { [weak self] _ in
      guard let self else { return }
      Task { @MainActor in self.updatePassThrough() }
    }
    let local = NSEvent.addLocalMonitorForEvents(matching: .mouseMoved) { [weak self] event in
      MainActor.assumeIsolated { self?.updatePassThrough() }
      return event
    }
    monitors = [global, local].compactMap { $0 }
    screenObserver = NotificationCenter.default.addObserver(
      forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main
    ) { [weak self] _ in
      MainActor.assumeIsolated { self?.place() }
    }
  }

  private func hide() {
    cancelClose()
    openAgent = nil
    cardHeight = nil
    monitors.forEach(NSEvent.removeMonitor)
    monitors = []
    if let screenObserver { NotificationCenter.default.removeObserver(screenObserver) }
    screenObserver = nil
    panel?.close()
    panel = nil
  }

  private func place() {
    guard let panel, let screen = NSScreen.screens.first else { return }
    panel.setFrame(
      NotchModel.panelFrame(screen: screen.frame, ringCount: ringCount), display: false)
    updatePassThrough()
  }

  private func updatePassThrough() {
    guard let panel else { return }
    let mouse = NSEvent.mouseLocation
    let rects = NotchModel.interactiveRects(
      panel: panel.frame, ringCount: ringCount, cardHeight: cardHeight)
    let inside = rects.contains { $0.contains(mouse) }
    if panel.ignoresMouseEvents == inside { panel.ignoresMouseEvents = !inside }
    guard openAgent != nil else { return }
    // SwiftUI can miss a hover change when ignoresMouseEvents flips on the same event.
    if let cardHeight, NotchModel.cardRect(panel: panel.frame, height: cardHeight).contains(mouse) {
      cancelClose()
    } else if !inside {
      scheduleClose()
    }
  }
}

struct NotchRoot: View {
  let controller: NotchController
  let store: UsageStore

  var body: some View {
    let now = store.quotaCheckDate
    let rings = NotchModel.rings(store: store, now: now)
    let card = controller.openAgent.flatMap { NotchModel.card(store: store, agent: $0, now: now) }
    ZStack(alignment: .trailing) {
      if !rings.isEmpty {
        HStack(spacing: NotchModel.cardGap) {
          if let card {
            NotchCard(data: card)
              .onHover { $0 ? controller.cancelClose() : controller.scheduleClose() }
          } else {
            Color.clear.frame(width: NotchModel.cardWidth)
          }
          notch(rings)
        }
      }
    }
    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .trailing)
    .onChange(of: rings.map(\.agent), initial: true) { controller.ringsChanged($1) }
    .onChange(
      of: card.map {
        NotchModel.cardHeight(hasSpend: $0.spend != nil, modelCount: $0.models.count)
      },
      initial: true
    ) { controller.cardHeightChanged($1) }
  }

  private func notch(_ rings: [NotchRing]) -> some View {
    VStack(spacing: NotchModel.cellSpacing) {
      ForEach(rings) { ring in
        UsageRing(ring: ring)
          .frame(width: NotchModel.notchDepth)
          .contentShape(Rectangle())
          .onHover { $0 ? controller.open(ring.agent) : controller.scheduleClose() }
      }
    }
    .padding(.top, NotchModel.curl + NotchModel.padTop)
    .padding(.bottom, NotchModel.curl + NotchModel.padBottom)
    .frame(width: NotchModel.notchDepth, height: NotchModel.notchHeight(ringCount: rings.count))
    .background(NotchShape().fill(.black))
  }
}
