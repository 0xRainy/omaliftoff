import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Commons as Commons
import qs.Ui
import "Model.js" as Model

// OmaLiftoff bar widget: "󱓞 Falcon 9 · T-02:14:07" — plain text; only the rocket icon is colored (live: accent pulse, hold/failure: red).
// Reads ONLY the cache file written by the liftoff daemon; never touches the network.
BarWidget {
  id: root
  moduleName: "liftoff.tracker"

  // ---- settings (typed in manifest.json)
  readonly property bool showVehicle: setting("showVehicle", true)
  readonly property string providerFilter: setting("providerFilter", "")
  readonly property bool starredOnly: setting("starredOnly", false)
  readonly property int liveWindowMinutes: setting("liveWindowMinutes", 60)
  readonly property int hideBeyondDays: setting("hideBeyondDays", 0)
  readonly property string colorMode: setting("colorMode", "Theme")
  readonly property string textMode: setting("textMode", "auto")

  // Bar section: the shell loads us via `Loader { id: registryLoader }` inside a
  // ModuleSlot that has `property string region` ("left" | "center" | "right" | "tray").
  // Walk up defensively; "" if not found (then text is shown).
  readonly property string region: {
    var p = root.parent
    for (var i = 0; p && i < 8; i++) {
      if (typeof p.region === "string") return p.region
      p = p.parent
    }
    return ""
  }
  readonly property bool showText: Model.showText(textMode, region, root.vertical)
  readonly property string liftoffBinary: setting("liftoffBinary", "$HOME/.local/bin/liftoff")
  readonly property string terminalCommand: setting("terminalCommand", "omarchy-launch-or-focus-tui")
  readonly property string cachePath: {
    var p = setting("cachePath", "")
    if (p) return p
    var xdg = Quickshell.env("XDG_CACHE_HOME")
    return (xdg ? xdg : Quickshell.env("HOME") + "/.cache") + "/liftoff/launches.json"
  }

  // ---- data
  property var rawCache: ({ launches: [], starred: [] })
  property var nudge: ({})
  // Cache re-anchored on the TUI's manual T-0 nudge and the daemon's measured clock skew.
  // + the TUI's hidden test flight while its heartbeat marker is fresh (display only).
  property var testMarker: null
  readonly property var cache: Model.withTest(Model.applyClock(rawCache, nudge), testMarker, nowMs)
  readonly property string testPath: cachePath.replace(/[^\/]*$/, "testflight.json")
  readonly property string nudgePath: cachePath.replace(/[^\/]*$/, "nudge.json")
  property double nowMs: Date.now()
  property int offset: 0              // scroll-wheel cycling
  readonly property var launches: Model.upcomingWithTest(cache, nowMs, { providerFilter: providerFilter, starredOnly: starredOnly })
  readonly property var current: launches.length ? launches[Math.min(offset, launches.length - 1)] : null
  readonly property bool live: Model.isLive(current, nowMs, liveWindowMinutes)
  readonly property bool inFlight: current ? current.status_id === 6 : false

  // ---- panel plumbing (same contract as the built-in clock)
  readonly property bool opened: panelLoader.item ? panelLoader.item.opened === true : false
  readonly property bool popoutSwitchClosing: panelLoader.item ? panelLoader.item.popoutSwitchClosing === true : false
  function open() { if (panelLoader.item) panelLoader.item.open() }
  function close() { if (panelLoader.item) panelLoader.item.close() }
  function toggle() { if (panelLoader.item) panelLoader.item.toggle() }
  function closeForPopoutSwitch() { if (panelLoader.item) panelLoader.item.closeForPopoutSwitch() }
  function injectPanel() {
    if (!panelLoader.item) return
    panelLoader.item.bar = root.bar
    panelLoader.item.anchorItem = button
    panelLoader.item.hostWidget = root
  }
  onBarChanged: injectPanel()

  // ---- actions
  function refresh() {
    if (bar) bar.run(liftoffBinary + " refresh")
    cacheFile.reload()
  }
  function openWebcast(l) {
    var x = l || current
    if (x && (x.webcast_url || x.info_url)) Qt.openUrlExternally(x.webcast_url || x.info_url)
  }
  // Companion (daemon + TUI) not installed yet: the cache has never been written.
  property bool needsSetup: false
  // install.sh ships in this plugin's own checkout, so setup runs the reviewed commit (no download-and-run).
  readonly property string installerPath: String(Qt.resolvedUrl("../install.sh")).replace(/^file:\/\//, "")
  function runSetup() {
    if (bar) bar.run(terminalCommand + " " + Util.shellQuote("bash " + Util.shellQuote(installerPath)))
  }

  function openTui() {
    if (bar) bar.run(terminalCommand + " " + Util.shellQuote(liftoffBinary + " tui"))
  }
  function cycle(step) {
    if (!launches.length) return
    offset = (offset + step + launches.length) % launches.length
    offsetReset.restart()
  }

  function statusColor(key) {
    if (colorMode === "Traffic light") {
      switch (key) {
        case "go": return "#8fd67a"
        case "tbc": return "#7ac7d6"
        case "hold": return "#e8b94a"
        case "flight": return "#c78af0"
        case "success": return "#6f9bf2"
        case "failure": return "#e86a6a"
        default: return Commons.Color.muted
      }
    }
    switch (key) {
      case "go":
      case "flight": return Commons.Color.accent
      case "hold":
      case "failure": return Commons.Color.urgent
      case "tbc": return bar ? bar.barForeground : Commons.Color.foreground
      default: return Commons.Color.muted
    }
  }

  implicitWidth: hiddenByDistance ? 0 : button.implicitWidth
  implicitHeight: button.implicitHeight
  readonly property bool hiddenByDistance: Model.hidden(current, nowMs, hideBeyondDays)
  visible: !hiddenByDistance

  FileView {
    id: cacheFile
    path: root.cachePath
    watchChanges: true
    preload: true
    onFileChanged: reload()
    onLoaded: { root.needsSetup = false; root.rawCache = Model.parseCache(text()) }
    onLoadFailed: {
      root.needsSetup = true
      root.rawCache = ({ launches: [], starred: [], error: "OmaLiftoff companion not installed (or liftoff.service has not fetched yet).\nClick to run the installer from this plugin's folder." })
    }
  }

  // A FileView can't watch a file that doesn't exist yet, so while setup is pending
  // poll for the cache the daemon writes on its first fetch.
  Timer {
    id: setupPoll
    interval: 3000
    repeat: true
    running: root.needsSetup
    onTriggered: cacheFile.reload()
  }

  FileView {
    id: nudgeFile
    path: root.nudgePath
    watchChanges: true
    preload: true
    onFileChanged: reload()
    onLoaded: root.nudge = Model.parseNudge(text())
    onLoadFailed: root.nudge = ({})
  }

  // Test-flight marker: rewritten (tmp + rename) every second by the TUI, so poll it
  // instead of relying on inotify across renames. Missing file = no test flight.
  FileView {
    id: testFile
    path: root.testPath
    preload: true
    onLoaded: root.testMarker = Model.parseTest(text())
    onLoadFailed: root.testMarker = null
  }

  // 1 Hz, re-armed just after each whole second so the countdown flips together
  // with the TUI (both floor whole seconds since T-0; see Model.relSecs).
  Timer {
    id: clockTick
    // Seconds flip on the skew-corrected clock (the cache's clock_offset_ms), like the TUI.
    interval: Model.msToNextSecond(Date.now(), root.rawCache)
    running: true
    repeat: false
    triggeredOnStart: true
    onTriggered: {
      root.nowMs = Date.now()
      testFile.reload()
      interval = Model.msToNextSecond(Date.now(), root.rawCache)
      restart()
    }
  }
  Timer {
    id: offsetReset
    interval: 30000
    onTriggered: root.offset = 0
  }

  Loader {
    id: panelLoader
    active: true
    source: Qt.resolvedUrl("Panel.qml")
    visible: false
    onLoaded: { root.injectPanel(); Qt.callLater(root.injectPanel) }
  }

  // Plain theme-colored text; only the leading "●" carries the status color.
  readonly property var stage: Model.liveStage(root.cache, root.current, root.nowMs)
  readonly property string label: root.needsSetup ? "set up OmaLiftoff" : Model.barText(root.current, root.nowMs, root.showVehicle,
      root.stage && root.stage.current ? root.stage.current.label : "")
    + (root.offset > 0 ? "  " + (root.offset + 1) + "/" + root.launches.length : "")
  readonly property color textColor: root.bar ? root.bar.barForeground : Commons.Color.foreground
  readonly property string iconState: Model.iconState(root.current, root.nowMs,
      root.stage && root.stage.current ? root.stage.current.label : "")
  readonly property color iconColor: iconState === "live" ? (Commons.Color.accent || "#6f9bf2")
    : iconState === "bad" ? (Commons.Color.urgent || "#e86a6a") : root.textColor
  // Nerd Font for the glyph even if the bar font isn't one.
  readonly property string iconFont: button.fontFamily + ", JetBrainsMono Nerd Font, JetBrainsMono NF, Symbols Nerd Font"
  readonly property var nextEvent: Model.nextMilestone(root.cache, root.current, root.nowMs)

  WidgetButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    // The button keeps the full string for sizing/hit-testing; we paint our own label.
    text: root.showText ? Model.ICON + " " + root.label : Model.ICON
    labelVisible: false
    dimmed: !root.current
    tooltipText: root.current
      ? (root.showText ? "" : Model.ICON + " " + root.label + "\n")
        + root.current.name + "\n" + root.current.provider + " · " + root.current.status_name
        + "\n" + root.current.pad
        + (root.stage && root.stage.current ? "\nnow: " + root.stage.current.label + " (" + root.stage.current.at + ")" : "")
        + (root.nextEvent ? "\nnext: " + root.nextEvent.text : "")
        + (root.current.webcast_live ? "\nwebcast live" : "")
        + "\nleft: details · right: webcast · middle: refresh · scroll: cycle"
      : (root.cache.error || "No upcoming launches cached")
    onPressed: function(code) {
      if (code === Qt.LeftButton) { if (root.needsSetup) root.runSetup(); else root.toggle() }
      else if (code === Qt.RightButton) root.openWebcast()
      else if (code === Qt.MiddleButton) root.refresh()
    }
    onWheelMoved: function(delta) { root.cycle(delta < 0 ? 1 : -1) }

    Row {
      anchors.centerIn: parent
      spacing: 0
      Text {
        id: icon
        text: Model.ICON
        color: root.iconColor
        font.family: root.iconFont
        font.pixelSize: button.fontSize
        renderType: Text.NativeRendering
        anchors.verticalCenter: parent.verticalCenter
        // live: ~1 Hz pulse; bad/normal: static
        SequentialAnimation on opacity {
          running: root.iconState === "live"
          loops: Animation.Infinite
          onRunningChanged: if (!running) icon.opacity = 1
          NumberAnimation { to: 0.3; duration: 500; easing.type: Easing.InOutSine }
          NumberAnimation { to: 1.0; duration: 500; easing.type: Easing.InOutSine }
        }
      }
      Text {
        visible: root.showText
        text: " " + root.label
        color: root.textColor
        textFormat: Text.PlainText
        font.family: button.fontFamily
        font.pixelSize: button.fontSize
        renderType: Text.NativeRendering
        anchors.verticalCenter: parent.verticalCenter
      }
    }
  }
}
