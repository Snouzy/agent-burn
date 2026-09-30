import AppKit
import Observation
import SwiftUI

@Observable @MainActor final class NotchController {
  static let shared = NotchController()

  private(set) var openAgent: String?
  // The panel is not part of a SwiftUI scene, so it has no openWindow action of its own.
  @ObservationIgnored var showDashboard: (() -> Void)?
  private(set) var disk: DiskSpace?
  private let appearance: AppAppearance
  private let readDisk: () -> DiskSpace?
  private let openURL: (URL) -> Void
  private var store: UsageStore?
  private var panel: NSPanel?
  private var agentCount = 0
  var ringCount: Int { agentCount + (disk == nil ? 0 : 1) }
  private var cardHeight: CGFloat?
  private var monitors: [Any] = []
  private var screenObserver: (any NSObjectProtocol)?
  private var pendingClose: Task<Void, Never>?

  init(
    appearance: AppAppearance = .shared,
    readDisk: @escaping () -> DiskSpace? = DiskSpace.startupDisk,
    openURL: @escaping (URL) -> Void = { NSWorkspace.shared.open($0) }
  ) {
    self.appearance = appearance
    self.readDisk = readDisk
    self.openURL = openURL
  }

  func attach(_ store: UsageStore) { self.store = store }

  func apply(_ on: Bool) {
    if on { refreshDisk() }
    if !on { hide() } else if panel == nil { show() } else { place() }
  }

  func open(_ agent: String) {
    cancelClose()
    openAgent = agent
    updatePassThrough()
  }

  func openDashboard(_ agent: String) {
    cancelClose()
    openAgent = nil
    store?.selection = agent
    showDashboard?()
    updatePassThrough()
  }

  func openStorage() {
    openURL(URL(string: "x-apple.systempreferences:com.apple.settings.Storage")!)
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

  func refreshDisk() {
    let next = appearance.showsDiskSpace ? readDisk() : nil
    guard next?.drawn != disk?.drawn else { return }
    let resized = (next == nil) != (disk == nil)
    disk = next
    if resized { place() }
  }

  func ringsChanged(_ agents: [String]) {
    agentCount = agents.count
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
    guard let panel,
      let screen = NotchModel.screen(
        from: NSScreen.screens, preference: appearance.notchDisplay,
        id: \.displayIdentifier, frame: \.frame)
    else { return }
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
  @State private var diskHovered = false

  var body: some View {
    let now = store.quotaCheckDate
    let rings = NotchModel.rings(store: store, now: now)
    let disk = controller.disk
    ZStack(alignment: .trailing) {
      if !rings.isEmpty || disk != nil {
        HStack(spacing: NotchModel.cardGap) {
          NotchCardStack(controller: controller, store: store, now: now)
          notch(rings, disk: disk)
        }
      }
    }
    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .trailing)
    .onChange(of: rings.map(\.agent), initial: true) { controller.ringsChanged($1) }
    .onChange(of: now) { controller.refreshDisk() }
    .onChange(
      of: controller.openAgent.flatMap { NotchModel.cardHeight(store: store, agent: $0) },
      initial: true
    ) { controller.cardHeightChanged($1) }
  }

  private func notch(_ rings: [NotchRing], disk: DiskSpace?) -> some View {
    VStack(spacing: NotchModel.cellSpacing) {
      ForEach(rings) { ring in
        UsageRing(ring: ring, highlighted: controller.openAgent == ring.agent)
          .frame(width: NotchModel.notchDepth)
          .contentShape(Rectangle())
          .onHover { $0 ? controller.open(ring.agent) : controller.scheduleClose() }
          .onTapGesture { controller.openDashboard(ring.agent) }
      }
      if let disk {
        DiskRing(disk: disk, highlighted: diskHovered)
          .frame(width: NotchModel.notchDepth)
          .contentShape(Rectangle())
          .onHover { hovering in
            diskHovered = hovering
            if hovering { controller.scheduleClose() }
          }
          .onTapGesture { controller.openStorage() }
          .accessibilityAddTraits(.isButton)
          .onDisappear { diskHovered = false }
      }
    }
    .padding(.top, NotchModel.curl + NotchModel.padTop)
    .padding(.bottom, NotchModel.curl + NotchModel.padBottom)
    .frame(
      width: NotchModel.notchDepth,
      height: NotchModel.notchHeight(ringCount: rings.count + (disk == nil ? 0 : 1))
    )
    .background(NotchShape().fill(.black))
  }
}

struct NotchDashboardLink: ViewModifier {
  @Environment(\.openWindow) private var openWindow

  func body(content: Content) -> some View {
    content.onAppear {
      NotchController.shared.showDashboard = {
        openWindow(id: "overview")
        NSApp.activate(ignoringOtherApps: true)
      }
    }
  }
}

// Every card stays built, so a hover only fades one in: building the chart costs about a frame.
// This view must not read `openAgent`, or each hover would rebuild all the charts.
private struct NotchCardStack: View {
  let controller: NotchController
  let store: UsageStore
  let now: Date

  var body: some View {
    ZStack {
      ForEach(NotchModel.cards(store: store, now: now), id: \.agent) { card in
        NotchCard(data: card).modifier(CardVisibility(agent: card.agent, controller: controller))
      }
    }
    .frame(width: NotchModel.cardWidth)
  }
}

private struct CardVisibility: ViewModifier {
  let agent: String
  let controller: NotchController

  func body(content: Content) -> some View {
    let visible = controller.openAgent == agent
    // .onHover stays inside .allowsHitTesting, or a hidden card above takes the open card's hover.
    content
      .onHover { $0 ? controller.cancelClose() : controller.scheduleClose() }
      .opacity(visible ? 1 : 0)
      .allowsHitTesting(visible)
      .accessibilityHidden(!visible)
      .zIndex(visible ? 1 : 0)
      .animation(.easeOut(duration: 0.12), value: visible)
  }
}
