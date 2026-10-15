import AppKit
import Observation
import SwiftUI

@Observable @MainActor final class NotchController {
  static let shared = NotchController()

  private(set) var openAgent: String? {
    didSet { if (oldValue == NotchModel.memoryCard) != memoryCardOpen { watchMemory() } }
  }
  private var memoryCardOpen: Bool { openAgent == NotchModel.memoryCard }
  private var cardOffset: CGFloat {
    guard let openAgent, let cardHeight else { return 0 }
    return cardOffset(for: openAgent, height: cardHeight)
  }
  /// Read while the memory card is open, so closed cards never walk the process table.
  private(set) var memorySnapshot: MemorySnapshot?
  /// Apps asked to quit; their rows dim until a reading no longer lists them.
  private(set) var quitting: Set<String> = []
  // The panel is not part of a SwiftUI scene, so it has no openWindow action of its own.
  @ObservationIgnored var showDashboard: (() -> Void)?
  private(set) var disk: DiskSpace?
  private(set) var memory: MemoryStats?
  private let appearance: AppAppearance
  private let readDisk: () -> DiskSpace?
  private let readMemory: () -> MemoryStats?
  private let openURL: (URL) -> Void
  private let readSnapshot: @Sendable () async -> MemorySnapshot?
  private let apps: RunningApps
  private var memoryWatch: Task<Void, Never>?
  private var store: UsageStore?
  private var panel: NSPanel?
  /// The agents' rings top to bottom; the disk and memory rings follow them.
  private var agents: [String] = []
  // Stored, not computed from `disk` and `memory`: their readings change every few seconds,
  // and the cards that place themselves with these must not rebuild their charts each time.
  private(set) var ringCount = 0
  private(set) var hasMemoryRing = false

  private func updateRings() {
    let count = agents.count + (disk == nil ? 0 : 1) + (memory == nil ? 0 : 1)
    if count != ringCount { ringCount = count }
    if (memory != nil) != hasMemoryRing { hasMemoryRing = memory != nil }
  }

  /// Where a card sits next to its ring. Plain arithmetic on the ring layout, so it only
  /// changes with the rings or the notch size, never with the pointer.
  func cardOffset(for agent: String, height: CGFloat) -> CGFloat {
    let cell = agent == NotchModel.memoryCard ? ringCount - 1 : agents.firstIndex(of: agent)
    guard let cell else { return 0 }
    return metrics.cardOffset(height: height, cell: cell, ringCount: ringCount)
  }
  private var cardHeight: CGFloat?
  private var resizeStart: (scale: Double, mouseX: CGFloat)?
  var isResizing: Bool { resizeStart != nil }
  var metrics: NotchMetrics { NotchMetrics(scale: appearance.notchScale) }
  var magnification: Double? { appearance.magnifiesNotch ? appearance.magnification : nil }
  private var monitors: [Any] = []
  private var screenObserver: (any NSObjectProtocol)?
  private var pendingClose: Task<Void, Never>?

  init(
    appearance: AppAppearance = .shared,
    readDisk: @escaping () -> DiskSpace? = DiskSpace.startupDisk,
    readMemory: @escaping () -> MemoryStats? = MemoryStats.system,
    openURL: @escaping (URL) -> Void = { NSWorkspace.shared.open($0) },
    readSnapshot: @escaping @Sendable () async -> MemorySnapshot? = {
      await Task.detached(priority: .userInitiated) { MemorySnapshot.read() }.value
    },
    apps: RunningApps = .workspace
  ) {
    self.appearance = appearance
    self.readDisk = readDisk
    self.readMemory = readMemory
    self.openURL = openURL
    self.readSnapshot = readSnapshot
    self.apps = apps
  }

  func attach(_ store: UsageStore) { self.store = store }

  func apply(_ on: Bool) {
    if on {
      refreshDisk()
      refreshMemory()
    }
    if !on { hide() } else if panel == nil { show() } else { place() }
  }

  func open(_ agent: String) {
    guard !isResizing else { return }
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

  func openActivityMonitor() {
    openURL(URL(fileURLWithPath: "/System/Applications/Utilities/Activity Monitor.app"))
  }

  func canQuit(_ consumer: MemoryConsumer) -> Bool {
    consumer.bundlePath.map(apps.canQuit) ?? false
  }

  /// Asks the app to quit, like Quit in its menu: it can still ask to save or cancel.
  func quit(_ consumer: MemoryConsumer) {
    guard let path = consumer.bundlePath, apps.canQuit(path) else { return }
    quitting.insert(consumer.id)
    apps.quit(path)
  }

  private func watchMemory() {
    memoryWatch?.cancel()
    memoryWatch = nil
    guard memoryCardOpen else { return }
    memoryWatch = Task { [weak self, readSnapshot] in
      while !Task.isCancelled {
        let snapshot = await readSnapshot()
        guard !Task.isCancelled, let self else { return }
        if let snapshot {
          memorySnapshot = snapshot
          quitting.formIntersection(snapshot.consumers.map(\.id))
        }
        try? await Task.sleep(for: .seconds(2))
      }
    }
  }

  var hiddenAgents: Set<String> { appearance.hiddenNotchAgents }

  func hideRing(_ agent: String) { appearance.hiddenNotchAgents.insert(agent) }

  func hideDisk() {
    appearance.showsDiskSpace = false
    refreshDisk()
  }

  func hideMemory() {
    appearance.showsMemory = false
    refreshMemory()
  }

  // Screen x coordinates, so the panel resizing under the pointer never shifts the drag.
  func beginResize(mouseX: CGFloat) {
    cancelClose()
    openAgent = nil
    resizeStart = (appearance.notchScale, mouseX)
  }

  func resize(mouseX: CGFloat) {
    guard let resizeStart else { return }
    appearance.notchScale = NotchMetrics.dragScale(
      start: resizeStart.scale, startX: resizeStart.mouseX, mouseX: mouseX)
  }

  func endResize() {
    resizeStart = nil
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

  func refreshDisk() {
    let next = appearance.showsDiskSpace ? readDisk() : nil
    guard next?.drawn != disk?.drawn else { return }
    let resized = (next == nil) != (disk == nil)
    disk = next
    updateRings()
    if resized { place() }
  }

  func refreshMemory() {
    let next = appearance.showsMemory ? readMemory() : nil
    guard next?.drawn != memory?.drawn else { return }
    let resized = (next == nil) != (memory == nil)
    memory = next
    updateRings()
    if next == nil, memoryCardOpen {
      cancelClose()
      openAgent = nil
    }
    if resized { place() }
  }

  func ringsChanged(_ agents: [String]) {
    if agents != self.agents { self.agents = agents }
    updateRings()
    if let openAgent, openAgent != NotchModel.memoryCard, !agents.contains(openAgent) {
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
      metrics.panelFrame(screen: screen.frame, ringCount: ringCount), display: false)
    updatePassThrough()
  }

  private func updatePassThrough() {
    guard let panel else { return }
    let mouse = NSEvent.mouseLocation
    let rects = metrics.interactiveRects(
      panel: panel.frame, ringCount: ringCount, cardHeight: cardHeight, cardOffset: cardOffset)
    // A drag can outrun the grabber; the panel must keep the mouse until it ends.
    let inside = isResizing || rects.contains { $0.contains(mouse) }
    if panel.ignoresMouseEvents == inside { panel.ignoresMouseEvents = !inside }
    guard openAgent != nil else { return }
    // SwiftUI can miss a hover change when ignoresMouseEvents flips on the same event.
    if let cardHeight,
      NotchModel.cardRect(panel: panel.frame, height: cardHeight, offset: cardOffset)
        .contains(mouse)
    {
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
    let hidden = controller.hiddenAgents
    let rings = NotchModel.rings(store: store, hiding: hidden, now: now)
    let disk = controller.disk
    let memory = controller.memory
    ZStack(alignment: .trailing) {
      if !rings.isEmpty || disk != nil || memory != nil {
        HStack(spacing: NotchModel.cardGap) {
          NotchCardStack(controller: controller, store: store, now: now)
          NotchColumn(controller: controller, rings: rings, disk: disk, memory: memory)
        }
      }
    }
    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .trailing)
    .onChange(of: rings.map(\.agent), initial: true) { controller.ringsChanged($1) }
    .onChange(of: now) {
      controller.refreshDisk()
      controller.refreshMemory()
    }
    .onChange(
      of: controller.openAgent.flatMap { NotchModel.cardHeight(store: store, agent: $0) },
      initial: true
    ) { controller.cardHeightChanged($1) }
  }
}

// Its own view, so pointer moves for the magnification never rebuild NotchRoot.
private struct NotchColumn: View {
  let controller: NotchController
  let rings: [NotchRing]
  let disk: DiskSpace?
  let memory: MemoryStats?
  @State private var diskHovered = false
  @State private var mouseY: CGFloat?

  var body: some View {
    let metrics = controller.metrics
    let cells = rings.count + (disk == nil ? 0 : 1) + (memory == nil ? 0 : 1)
    VStack(spacing: metrics.cellSpacing) {
      ForEach(Array(rings.enumerated()), id: \.element.id) { index, ring in
        UsageRing(
          ring: ring, highlighted: controller.openAgent == ring.agent, metrics: metrics,
          onRemove: { controller.hideRing(ring.agent) }
        )
        .modifier(magnified(index, metrics))
        .onHover { $0 ? controller.open(ring.agent) : controller.scheduleClose() }
        .onTapGesture { controller.openDashboard(ring.agent) }
      }
      if let disk {
        DiskRing(
          disk: disk, highlighted: diskHovered, metrics: metrics, onRemove: controller.hideDisk
        )
        .modifier(magnified(rings.count, metrics))
        .onHover { hovering in
          diskHovered = hovering
          if hovering { controller.scheduleClose() }
        }
        .onTapGesture { controller.openStorage() }
        .accessibilityAddTraits(.isButton)
        .onDisappear { diskHovered = false }
      }
      if let memory {
        MemoryRing(
          memory: memory, highlighted: controller.openAgent == NotchModel.memoryCard,
          metrics: metrics, onRemove: controller.hideMemory
        )
        .modifier(magnified(cells - 1, metrics))
        .onHover { $0 ? controller.open(NotchModel.memoryCard) : controller.scheduleClose() }
        .onTapGesture { controller.openActivityMonitor() }
        .accessibilityAddTraits(.isButton)
      }
    }
    .padding(.top, metrics.curl + metrics.padTop)
    .padding(.bottom, metrics.curl + metrics.padBottom)
    .frame(width: metrics.notchDepth, height: metrics.notchHeight(ringCount: cells))
    .background(NotchShape(metrics: metrics).fill(.black))
    .overlay(alignment: .bottom) {
      ResizeGrabber(controller: controller, metrics: metrics).padding(.bottom, metrics.curl)
    }
    .onContinuousHover { phase in
      switch phase {
      case .active(let location): mouseY = location.y
      case .ended: mouseY = nil
      }
    }
  }

  private func magnified(_ index: Int, _ metrics: NotchMetrics) -> Magnified {
    let amount = controller.isResizing ? nil : controller.magnification
    return Magnified(
      scale: amount.map { metrics.magnification(cell: index, mouseY: mouseY, amount: $0) } ?? 1,
      hovering: mouseY != nil, width: metrics.notchDepth)
  }
}

private struct Magnified: ViewModifier {
  let scale: CGFloat
  let hovering: Bool
  let width: CGFloat

  // Pointer events come slower and less evenly than display frames, so a short interactive
  // spring fills the frames in between. Each event retargets it without losing its speed.
  func body(content: Content) -> some View {
    content
      .frame(width: width)
      .contentShape(Rectangle())
      .scaleEffect(scale)
      .animation(
        hovering
          ? .interactiveSpring(response: 0.16, dampingFraction: 0.86)
          : .spring(duration: 0.25),
        value: scale)
  }
}

/// The notch's Dock separator: drag it away from the screen edge to grow the notch.
private struct ResizeGrabber: View {
  let controller: NotchController
  let metrics: NotchMetrics
  @State private var hovered = false

  var body: some View {
    let active = hovered || controller.isResizing
    Capsule()
      .fill(Color.white.opacity(active ? 0.6 : 0.3))
      .frame(width: 18 * metrics.scale, height: 3 * metrics.scale)
      .frame(width: metrics.notchDepth * 0.6, height: metrics.padBottom)
      .contentShape(Rectangle())
      .onHover { hovering in
        hovered = hovering
        if hovering {
          NSCursor.resizeLeftRight.set()
        } else if !controller.isResizing {
          NSCursor.arrow.set()
        }
      }
      .gesture(
        DragGesture(minimumDistance: 0)
          .onChanged { _ in
            let mouseX = NSEvent.mouseLocation.x
            if controller.isResizing {
              controller.resize(mouseX: mouseX)
            } else {
              controller.beginResize(mouseX: mouseX)
            }
            NSCursor.resizeLeftRight.set()
          }
          .onEnded { _ in
            controller.endResize()
            if !hovered { NSCursor.arrow.set() }
          }
      )
      .animation(.easeOut(duration: 0.15), value: active)
      .accessibilityHidden(true)
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
      ForEach(
        NotchModel.cards(store: store, hiding: controller.hiddenAgents, now: now), id: \.agent
      ) { card in
        NotchCard(data: card)
          .offset(
            y: controller.cardOffset(
              for: card.agent,
              height: NotchModel.cardHeight(
                hasSpend: card.spend != nil, modelCount: card.models.count))
          )
          .modifier(CardVisibility(agent: card.agent, controller: controller))
      }
      if controller.hasMemoryRing {
        MemoryCardHost(controller: controller)
          .offset(
            y: controller.cardOffset(
              for: NotchModel.memoryCard, height: NotchModel.memoryCardHeight)
          )
          .modifier(CardVisibility(agent: NotchModel.memoryCard, controller: controller))
      }
    }
    .frame(width: NotchModel.cardWidth)
  }
}

// Its own view, so a fresh memory reading every two seconds never rebuilds the agents' charts.
private struct MemoryCardHost: View {
  let controller: NotchController

  var body: some View {
    MemoryCard(
      snapshot: controller.memorySnapshot, quitting: controller.quitting,
      canQuit: controller.canQuit, onQuit: controller.quit)
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
