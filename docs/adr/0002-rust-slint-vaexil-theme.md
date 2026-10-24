# ADR 0002: Rust and Slint, laid out like Modlist Launcher, in the Vaexil theme

Date: 2026-10-24
Status: accepted

## Context

JAMARQ's Windows-native desktop apps are Rust and Slint (agentic-instructions ADR-0034, 2026-10-24); Mac-first and multi-platform apps are Tauri. Modlist Launcher is the first of the Windows apps and the template for the rest. Its ADR 0001 explains the choice of Slint's software renderer (about 34 MB working set against about 111 MB with OpenGL), and its ADR 0002 records the Vaexil theme.

## Decision

- Same layout as Modlist Launcher: a `core` crate with no UI and no writes, an `app` crate with the Slint window, a written safety model, CI on GitHub-hosted runners, cargo-deny with HTTP and TLS crates banned, and an Inno Setup per-user installer plus a portable zip.
- Slint 1.18.1 with the `cupertino` style for the standard widgets still in use, and the software renderer.
- `crates/app/ui/theme.slint` is copied unchanged from Modlist Launcher, and the Geist fonts are copied from its `assets/fonts`. First copied from jmars319/modlist-launcher#13 at de1ec1c. Recopied on 2026-10-24 from #20 at c04e8d0, which fixes mouse clicks: the TouchArea in Button, Switch, CheckBox, ComboBox and ActionItem sat below the FocusScope that Slint 1.18 lets take the first press for focus. Import Theme, Button, ComboBox, Switch and CheckBox from it rather than from std-widgets. A shared theme crate waits until a third Slint app needs it.
- The icon keeps the Vaexil canvas and the mark's blue-to-magenta, with this app's own symbol (a mod list with load-order markers), so it stays distinct from Modlist Launcher in the taskbar.
- The repaint workaround after restoring from minimized is carried over (`repaint_after_restore`, and `window-background-alt` one step off `window-background`), with the same smoke test.

## Consequences

- One shared look and one way of working across the Windows apps.
- Slint's accessibility tree only includes rows that are scrolled into view. A screen reader cannot reach a mod further down the list until it is scrolled there, which Modlist Launcher also found. The interactive mod list must move keyboard focus row by row and scroll the focused row into view.
- When the theme changes in Modlist Launcher, copy the file again; do not edit it here.
