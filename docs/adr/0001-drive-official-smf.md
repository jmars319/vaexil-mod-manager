# ADR 0001: Drive the official SMF, never change it

Date: 2026-10-24
Status: accepted

## Context

Vaexil's earlier tool, `vaexil-simple-mod-framework`, was a full fork of Simple Mod Framework. Its own changes were small (about 370 lines in the deploy engine, plus the Mod Manager), but owning the whole engine meant every upstream release had to be merged by hand. It was frozen at 2.33.35 while upstream shipped seven releases, four of them for new HITMAN versions, the last one day after the game patch.

The owner wants to stay current with HITMAN and with SMF as invisibly as possible, and wants Vaexil's work to never affect SMF's source code.

## Decision

- Vaexil Mod Manager contains none of SMF's code. It drives the official SMF already installed in the game folder, the way Modlist Launcher drives Mod Organizer 2 and Keeplight drives OpenRGB.
- It uses only what SMF documents for its users: `config.json`, the `Mods` folder, each mod's `manifest.json`, and `Deploy.exe` with the `--doNotPause --colors` flags SMF's own Mod Manager passes. No patches, no rebuilt `Deploy.exe`, no flags of its own.
- Diagnostics are worked out from outside SMF: its log output, the processes it starts, and its staging folders.
- A later stage applies new SMF releases automatically, the same way SMF's own updater does. A release is held only when upstream marks it "CANNOT AUTOMATICALLY UPDATE", never during a deploy, and never while HITMAN runs.
- Each mod framework is a module behind one interface (`crates/core/src/framework.rs`). Version 1 has one, SMF. Only that module knows where SMF lives, so shipping a bundled copy of SMF later would change that module alone.

## Consequences

- Upstream releases cost a version check, not a merge.
- Vaexil's deploy diagnostics depend on SMF's log wording, which is not a promise. Tests replay recorded SMF output, and a line the app does not recognise falls back to the plain log, never to a failed deploy.
- The owner's decision is recorded in agentic-instructions' decision log (2026-10-24) and the plan page linked from the README.
