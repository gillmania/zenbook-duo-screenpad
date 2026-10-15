import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import qs.Commons

// The ScreenPad's touchpad surface. While the ScreenPad is a touchpad (see
// service/), it shows a plain grey pad with the Omarchy logo, in the colours of
// the theme. It takes no input: the helper holds the touchscreen, and the
// pointer cannot reach this screen. It is drawn once and never animates, so
// Hyprland has nothing to redraw.
Item {
  id: root

  // Injected by omarchy-shell.
  property var shell: null
  property var manifest: null

  readonly property string modePath:
    (Quickshell.env("XDG_STATE_HOME") || (Quickshell.env("HOME") + "/.local/state")) + "/zenbook-duo-screenpad/mode"
  // "off", "desktop" or "touchpad". Read from the service's mode file when the
  // shell starts, and told by the service on every change after that.
  property string mode: ""
  readonly property bool touchpad: mode === "touchpad"

  FileView {
    path: root.modePath
    onLoaded: root.mode = text().trim()
  }

  // ---- Colours ----
  // The pad shifts from pink through purple to blue, in the theme's own
  // shades of those (Tokyo Night: red, bright magenta, blue). A theme without
  // them falls back to its accent.
  FileView {
    id: themeFile
    path: (Quickshell.env("HOME") + "/.local/state/omarchy/current/theme/colors.toml")
    watchChanges: true
    onFileChanged: reload()
  }
  function themeColor(key) {
    var match = String(themeFile.text() || "").match(new RegExp("^" + key + "\\s*=\\s*\"(#[0-9a-fA-F]{6})\"", "m"))
    return match ? match[1] : String(Color.accent).slice(0, 7)
  }
  readonly property var shades: [themeColor("red"), themeColor("bright_magenta"), themeColor("blue")]

  // Omarchy's logo in the same shift. The file draws it in black; the fill is
  // swapped for a gradient in the SVG itself, which keeps it sharp.
  FileView {
    id: logoFile
    path: "/usr/share/omarchy/logo.svg"
  }
  readonly property string logoSource: {
    var svg = logoFile.text()
    if (!svg) return ""
    var stops = ""
    for (var i = 0; i < shades.length; i++)
      stops += '<stop offset="' + (i / (shades.length - 1)) + '" stop-color="' + shades[i] + '"/>'
    var defs = '<defs><linearGradient id="shift" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="1215" y2="0">'
      + stops + '</linearGradient></defs>'
    svg = svg.replace(/(<svg[^>]*>)/, "$1" + defs).replace('fill="#000"', 'fill="url(#shift)"')
    return "data:image/svg+xml;utf8," + encodeURIComponent(svg)
  }

  IpcHandler {
    target: "zenbook-duo-screenpad"
    function show(mode: string): void { root.mode = mode }
  }

  Variants {
    // The ScreenPad by its EDID model, as the rest of the project does.
    model: Quickshell.screens.filter(function(s) { return String(s.model).indexOf("0x0921") !== -1 })

    PanelWindow {
      required property var modelData

      screen: modelData
      visible: root.touchpad
      color: surface
      exclusionMode: ExclusionMode.Ignore
      WlrLayershell.layer: WlrLayer.Overlay
      WlrLayershell.keyboardFocus: WlrKeyboardFocus.None
      WlrLayershell.namespace: "zenbook-duo-screenpad-touchpad"
      anchors { top: true; bottom: true; left: true; right: true }

      // Grey from the theme: its background lifted toward its foreground, so a
      // dark theme gives a dark grey pad and a light theme a light one.
      readonly property color surface: Qt.tint(Color.background, Util.alpha(Color.foreground, 0.12))

      // A faint wash of the three shades across the pad.
      Rectangle {
        anchors.fill: parent
        gradient: Gradient {
          orientation: Gradient.Horizontal
          GradientStop { position: 0.0; color: Util.alpha(root.shades[0], 0.10) }
          GradientStop { position: 0.5; color: Util.alpha(root.shades[1], 0.08) }
          GradientStop { position: 1.0; color: Util.alpha(root.shades[2], 0.10) }
        }
      }

      // A honeycomb texture, drawn once: it is only painted again when the
      // theme or the size changes.
      Canvas {
        id: texture
        anchors.fill: parent
        anchors.margins: Style.space(10)
        readonly property var shades: root.shades
        onShadesChanged: requestPaint()
        onWidthChanged: requestPaint()
        onHeightChanged: requestPaint()
        onPaint: {
          var ctx = getContext("2d")
          ctx.reset()
          // The lines shift with the wash, and the frame is drawn in the same
          // shift, a little stronger.
          function shift(alpha) {
            var g = ctx.createLinearGradient(0, 0, width, 0)
            for (var i = 0; i < shades.length; i++)
              g.addColorStop(i / (shades.length - 1), Util.alpha(shades[i], alpha))
            return g
          }
          ctx.strokeStyle = shift(0.20)
          ctx.lineWidth = 1
          var r = 14
          var w = Math.sqrt(3) * r
          ctx.beginPath()
          for (var row = 0, cy = 0; cy < height + r; row++, cy += 1.5 * r) {
            for (var cx = (row % 2) * w / 2; cx < width + w; cx += w) {
              for (var k = 0; k <= 6; k++) {
                var a = Math.PI / 3 * k + Math.PI / 6
                var px = cx + r * Math.cos(a), py = cy + r * Math.sin(a)
                if (k === 0) ctx.moveTo(px, py); else ctx.lineTo(px, py)
              }
            }
          }
          ctx.stroke()
          ctx.strokeStyle = shift(0.55)
          ctx.lineWidth = 2
          ctx.strokeRect(1, 1, width - 2, height - 2)
        }
      }

      Image {
        anchors.centerIn: parent
        width: parent.width * 0.22
        height: width * 285 / 1215
        source: root.logoSource
        sourceSize.width: width
        sourceSize.height: height
        opacity: 0.75
      }
    }
  }
}
