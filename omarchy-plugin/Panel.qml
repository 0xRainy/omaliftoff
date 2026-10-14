import QtQuick
import QtQuick.Layouts
import Quickshell
import qs.Commons
import qs.Commons as Commons
import qs.Ui
import "Model.js" as Model

// Popup: mission patch, status, odds, weather, webcast button, next 5.
Panel {
  id: root
  moduleName: "liftoff.tracker"
  manageIpc: false

  property var anchorItem: null
  property var hostWidget: null

  readonly property var w: hostWidget
  readonly property var l: w ? w.current : null
  readonly property double nowMs: w ? w.nowMs : Date.now()
  readonly property color fg: root.barForeground
  readonly property color dim: Commons.Color.muted
  readonly property string font: root.bar ? root.bar.fontFamily : Style.font.family

  function open() { root.controller.show() }
  function close() { root.controller.hide() }
  function switchPanel(direction) {
    if (root.bar && typeof root.bar.switchPanelFrom === "function")
      return root.bar.switchPanelFrom(root.hostWidget || root, direction)
    return false
  }

  component Chip: Rectangle {
    property alias label: chipText.text
    property color tint: root.fg
    radius: height / 2
    color: Qt.rgba(tint.r, tint.g, tint.b, 0.18)
    border.color: tint
    border.width: 1
    implicitHeight: chipText.implicitHeight + Style.space(6)
    implicitWidth: chipText.implicitWidth + Style.space(16)
    Text {
      id: chipText
      anchors.centerIn: parent
      color: parent.tint
      font.family: root.font
      font.pixelSize: Style.font.body
      font.bold: true
    }
  }

  component ActionButton: Rectangle {
    id: btn
    property alias label: btnText.text
    property bool enabledAction: true
    signal activated()
    radius: Style.space(6)
    color: hover.containsMouse && enabledAction ? Qt.rgba(root.fg.r, root.fg.g, root.fg.b, 0.14) : Qt.rgba(root.fg.r, root.fg.g, root.fg.b, 0.06)
    border.color: Qt.rgba(root.fg.r, root.fg.g, root.fg.b, 0.3)
    opacity: enabledAction ? 1 : 0.4
    implicitHeight: btnText.implicitHeight + Style.space(12)
    implicitWidth: btnText.implicitWidth + Style.space(24)
    Text {
      id: btnText
      anchors.centerIn: parent
      color: root.fg
      font.family: root.font
      font.pixelSize: Style.font.body
    }
    MouseArea {
      id: hover
      anchors.fill: parent
      hoverEnabled: true
      cursorShape: Qt.PointingHandCursor
      onClicked: if (btn.enabledAction) btn.activated()
    }
  }

  KeyboardPanel {
    id: panel
    anchorItem: root.anchorItem
    owner: root.hostWidget || root
    bar: root.bar
    open: root.opened
    focusTarget: keyCatcher
    contentWidth: panel.fittedContentWidth(Style.space(380))
    contentHeight: panel.fittedContentHeight(content.implicitHeight)

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      onCloseRequested: root.close()
      onTabRequested: function(direction) { root.switchPanel(direction) }
      Keys.onPressed: function(e) {
        if (!root.w) return
        if (e.key === Qt.Key_J || e.key === Qt.Key_Down) { root.w.cycle(1); e.accepted = true }
        else if (e.key === Qt.Key_K || e.key === Qt.Key_Up) { root.w.cycle(-1); e.accepted = true }
        else if (e.key === Qt.Key_W) { root.w.openWebcast(); e.accepted = true }
        else if (e.key === Qt.Key_R) { root.w.refresh(); e.accepted = true }
        else if (e.key === Qt.Key_T) { root.w.openTui(); root.close(); e.accepted = true }
      }

      ColumnLayout {
        id: content
        width: parent.width
        spacing: Style.space(10)

        Text {
          visible: !root.l
          text: root.w && root.w.cache.error ? root.w.cache.error : "No upcoming launches in cache.\nStart it with: systemctl --user enable --now liftoff"
          color: root.dim
          font.family: root.font
          font.pixelSize: Style.font.body
          wrapMode: Text.WordWrap
          Layout.fillWidth: true
        }

        // ---- test flight banner (TUI hidden test flight; display only, no alerts)
        Text {
          visible: Model.isTest(root.l)
          text: "TEST FLIGHT · synthetic loop from the TUI (Ctrl-x t f to stop) · not a real launch"
          color: root.w ? root.w.statusColor("flight") : root.fg
          font.family: root.font
          font.pixelSize: Style.font.body
          font.bold: true
          wrapMode: Text.WordWrap
          Layout.fillWidth: true
        }

        // ---- header: patch + names
        RowLayout {
          visible: !!root.l
          spacing: Style.space(12)
          Layout.fillWidth: true
          Image {
            source: root.l && (root.l.patch_url || root.l.image_url) ? (root.l.patch_url || root.l.image_url) : ""
            visible: status === Image.Ready
            asynchronous: true
            cache: true
            fillMode: Image.PreserveAspectFit
            Layout.preferredWidth: Style.space(64)
            Layout.preferredHeight: Style.space(64)
            sourceSize.width: 128
            sourceSize.height: 128
          }
          ColumnLayout {
            Layout.fillWidth: true
            spacing: Style.space(2)
            Text {
              text: root.l ? (root.l.mission || root.l.name) : ""
              color: root.fg
              font.family: root.font
              font.pixelSize: Style.font.subtitle
              font.bold: true
              elide: Text.ElideRight
              Layout.fillWidth: true
            }
            Text {
              text: root.l ? Model.shortRocket(root.l) + " · " + root.l.provider : ""
              color: root.fg
              font.family: root.font
              font.pixelSize: Style.font.body
              Layout.fillWidth: true
              elide: Text.ElideRight
            }
            Text {
              text: root.l ? root.l.pad + (root.l.location ? " · " + root.l.location : "") : ""
              color: root.dim
              font.family: root.font
              font.pixelSize: Style.font.body
              Layout.fillWidth: true
              elide: Text.ElideRight
            }
          }
        }

        // ---- countdown
        Text {
          visible: !!root.l
          text: root.l ? Model.displayCountdown(root.l, root.nowMs) : ""
          color: root.w && root.l ? root.w.statusColor(Model.colorKey(root.l)) : root.fg
          font.family: root.font
          font.pixelSize: Style.font.displayLarge
          font.bold: true
          Layout.alignment: Qt.AlignHCenter
        }
        Text {
          visible: !!root.l
          text: root.l ? "NET " + Model.netText(root.l, "long") : ""
          color: root.dim
          font.family: root.font
          font.pixelSize: Style.font.body
          Layout.alignment: Qt.AlignHCenter
        }
        // ---- live: current + next flight event (from cached flight plan / LL2 timeline)
        ColumnLayout {
          readonly property var st: root.w ? Model.liveStage(root.w.cache, root.l, root.nowMs) : null
          visible: !!st
          spacing: Style.space(2)
          Layout.alignment: Qt.AlignHCenter
          Text {
            text: (Model.isTest(root.l) ? "● TEST FLIGHT  " : "● LIVE  ") + (parent.st ? Model.fmtRel(parent.st.tRel) : "")
            color: root.w ? root.w.statusColor("flight") : root.fg
            font.family: root.font
            font.pixelSize: Style.font.body
            font.bold: true
            Layout.alignment: Qt.AlignHCenter
          }
          Text {
            visible: !!(parent.st && parent.st.current)
            text: parent.st && parent.st.current ? "now  " + parent.st.current.label + "  " + parent.st.current.at : ""
            color: root.fg
            font.family: root.font
            font.pixelSize: Style.font.body
            Layout.alignment: Qt.AlignHCenter
          }
          Text {
            visible: !!(parent.st && parent.st.next)
            text: parent.st && parent.st.next ? "next " + parent.st.next.label + "  " + parent.st.next.at + "  (in " + Model.shortSecs(parent.st.next.in) + ")" : ""
            color: root.dim
            font.family: root.font
            font.pixelSize: Style.font.body
            Layout.alignment: Qt.AlignHCenter
          }
        }
        Text {
          readonly property var ev: root.w ? Model.nextMilestone(root.w.cache, root.l, root.nowMs) : null
          visible: !!ev && !(root.w && Model.liveStage(root.w.cache, root.l, root.nowMs))
          text: ev ? "next: " + ev.label + " · " + ev.at : ""
          color: root.fg
          font.family: root.font
          font.pixelSize: Style.font.body
          Layout.alignment: Qt.AlignHCenter
        }

        // ---- status, odds, weather
        Flow {
          visible: !!root.l
          Layout.fillWidth: true
          spacing: Style.space(6)
          Chip {
            label: root.l ? root.l.status_name : ""
            tint: root.w && root.l ? root.w.statusColor(Model.colorKey(root.l)) : root.fg
          }
          Chip {
            visible: !!root.l && root.l.probability !== null && root.l.probability !== undefined
            label: root.l ? "☁ " + root.l.probability + "% go" : ""
          }
          Chip {
            visible: !!root.l && root.l.webcast_live
            label: "● LIVE"
            tint: Commons.Color.urgent
          }
          Chip {
            visible: !!root.l && !!root.l.orbit
            label: root.l ? root.l.orbit : ""
            tint: root.dim
          }
        }
        Text {
          visible: !!root.l && !!root.l.weather_concerns
          text: root.l && root.l.weather_concerns ? "Weather: " + root.l.weather_concerns : ""
          color: root.dim
          font.family: root.font
          font.pixelSize: Style.font.body
          wrapMode: Text.WordWrap
          Layout.fillWidth: true
        }

        // ---- actions
        RowLayout {
          visible: !!root.l
          spacing: Style.space(8)
          ActionButton {
            label: root.l && root.l.webcast_url ? "▶ Watch webcast" : "No webcast yet"
            enabledAction: !!root.l && !!(root.l.webcast_url || root.l.info_url)
            onActivated: root.w.openWebcast()
          }
          ActionButton { label: "Open TUI"; onActivated: { root.w.openTui(); root.close() } }
          ActionButton { label: "󰑐"; onActivated: root.w.refresh() }
        }

        // ---- next 5
        Rectangle { Layout.fillWidth: true; implicitHeight: 1; color: Qt.rgba(root.fg.r, root.fg.g, root.fg.b, 0.15) }
        Text {
          text: "Up next"
          color: root.dim
          font.family: root.font
          font.pixelSize: Style.font.body
          font.bold: true
        }
        Repeater {
          model: root.w ? root.w.launches.slice(0, 5) : []
          delegate: Rectangle {
            required property var modelData
            required property int index
            Layout.fillWidth: true
            implicitHeight: rowLayout.implicitHeight + Style.space(6)
            radius: Style.space(4)
            color: (root.w && root.w.offset === index) ? Qt.rgba(root.fg.r, root.fg.g, root.fg.b, 0.10)
              : rowHover.containsMouse ? Qt.rgba(root.fg.r, root.fg.g, root.fg.b, 0.05) : "transparent"
            RowLayout {
              id: rowLayout
              anchors.fill: parent
              anchors.leftMargin: Style.space(6)
              anchors.rightMargin: Style.space(6)
              spacing: Style.space(8)
              Rectangle {
                implicitWidth: Style.space(8); implicitHeight: Style.space(8); radius: width / 2
                color: root.w ? root.w.statusColor(Model.colorKey(modelData)) : root.fg
              }
              Text {
                text: Model.shortRocket(modelData) + " · " + (modelData.mission || modelData.name)
                color: root.fg
                font.family: root.font
                font.pixelSize: Style.font.body
                elide: Text.ElideRight
                Layout.fillWidth: true
              }
              Text {
                text: Model.displayCountdown(modelData, root.nowMs)
                color: root.dim
                font.family: root.font
                font.pixelSize: Style.font.body
              }
            }
            MouseArea {
              id: rowHover
              anchors.fill: parent
              hoverEnabled: true
              cursorShape: Qt.PointingHandCursor
              onClicked: if (root.w) { root.w.offset = index; root.w.cycle(0) }
            }
          }
        }
        Text {
          text: root.w ? "cache " + Model.ago(root.w.cache.fetched_at, root.nowMs)
            + (root.w.cache.throttle ? " · LL2 " + Math.max(0, root.w.cache.throttle.limit - root.w.cache.throttle.current_use) + "/" + root.w.cache.throttle.limit + " left" : "")
            + "  ·  j/k cycle · w watch · r refresh · t TUI" : ""
          color: root.dim
          font.family: root.font
          font.pixelSize: Style.font.caption ? Style.font.caption : Style.font.body
          Layout.fillWidth: true
          wrapMode: Text.WordWrap
        }
      }
    }
  }
}
