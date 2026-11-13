# Vaexil Mod Manager project state

Plan artifact: https://claude.ai/artifact/HBDVnjvCrkyQsY7Bak28Dm

The working state around the plan page and the docs. The plan page holds the roadmap and the ledger; `docs/safety-model.md` and `docs/adr/` hold the rules. Checked 2026-11-13 on JASON-PC.

## Core Project Architecture & Stack

- Windows-native desktop app: Rust (edition 2024, MSRV 1.92) with Slint 1.18 (winit, software renderer, accessibility), per ADR-0034. Workspace with `crates/core` (`vmm-core`, platform-neutral) and `crates/app` (`vaexil-mod-manager`, window and Windows calls).
- Drives the official Simple Mod Framework already in the HITMAN folder (ADR 0001). It never patches, rebuilds or replaces SMF, and it uses only `config.json`, the `Mods` folder and `Deploy.exe --doNotPause --colors`.
- `Framework` trait (`crates/core/src/framework.rs`): `inspect` (read-only) and `set_enabled`, the only write. `smf::Smf` is the only module in v1.
- Game discovery: Steam (registry plus `libraryfolders.vdf`, app 1659040), Epic manifests, Microsoft Store `XboxGames`. SMF version comes from `Deploy.exe`'s SHA-256 in `smf/releases.rs`.
- UI: `crates/app/ui/app.slint` plus `theme.slint`, which is byte-identical to Modlist Launcher's (blob `6ba42346`); change both repos together. Geist fonts are embedded.
- CI (`.github/workflows/ci.yml`, windows-latest): fmt, clippy `-D warnings`, tests, release build, both smoke scripts, then cargo-deny. HTTP and TLS crates are banned until stage 04. Releases: a `v*` tag makes a draft, and publishing it copies the installer to `JAMARQ-Digital-LLC/jamarq-releases` (needs the `JAMARQ_RELEASES_TOKEN` secret, which is set).

## Current Structural Milestones / Active Feature Goals

- Stage 00 (direction, ADR-0034) and stage 01 (repo, read-only first window, #1) are done.
- Stage 02, in progress since 2026-11-13, in four PRs:
  1. Mods view grouped by type, with on/off switches. #12, merged 2026-11-13 as 118506b. The app's first write.
  2. Edit sheet per mod: options (`modOptions`) and a type override kept in the app's own `settings.json`. #14, merged 2026-11-13 as d43e161, after two Codex findings were fixed.
  3. Load order view: drag, Auto-sort by `loadBefore`/`loadAfter`, broken-rule warnings, "overridden by" markers in the Mods view.
  4. Setups, Play (deploy if changed, then start HITMAN) and Deploy, both blocked while HITMAN runs.
- Scope widened 2026-11-13: a multi-game mod manager that any LLM can operate (Claude, Gemini, GPT, local models). The proposed ADR 0003 (`docs/adr/0003-headless-engine-and-llm-interfaces.md`) adds a `vmm-engine` crate that every front end calls, backends as their own crates (SMF now, Cyberpunk 2077 later), one versioned JSON schema, a `vmm` CLI with `--json`, a stdio MCP server (`vmm mcp`), plan-then-apply for every change, and JSON logs and diagnosis. If accepted, its steps 1 to 4 come before stage 02's PR 3 and PR 4.
- Then stage 03 (deploy view and diagnostics), 04 (keep SMF current), 05 (Steam's Play button), 06 (first release and update notice). The old repo `JAMARQ-Digital-LLC/vaexil-simple-mod-framework` is archived, and its Dependabot #24 closed, after the app's first deploy on this PC.

## Active Task State & Known Regressions

- `main` is at #14 (d43e161), the edit sheet. ADR 0003 is waiting on the owner's decisions.
- Merging needs every review thread resolved (branch protection: conversation resolution and linear history). GitHub auto-merge is off for this repo. Codex reviews each push.
- Installed on JASON-PC: built from d43e161 with `scripts/package.ps1` and installed per-user with the Inno installer (`/VERYSILENT`) on 2026-11-13. The installed exe's SHA-256 matches the build. Still versioned 0.1.0, because nothing is released. Checked against the real install without clicking: SMF 2.33.45, 15 mods, 10 on, no orphaned load-order ids.
- HITMAN folder (2026-11-12): official SMF 2.33.45 applied the way SMF's updater does, Vortex leftovers removed, the owner's 10-mod `config.json` restored. Test deploy took 30m32s with 0 errors. The pre-reinstall backup is in `Downloads\hitman-smf-backup-2026-11-12` until the owner has checked the game.
- Known gaps:
  - Type guesses put Lighting Ultimate and the glasses fix in Gameplay. Fix them by hand with Edit, then Type.
  - SMF 2.33.43 and 2.33.44 are missing from the release table, so either would show as unknown until stage 04 replaces the table.
- No GitHub release exists yet.

## Temporary Logic Rules & Constraints

- `config.json` is JSON5 as SMF writes it (single quotes). The app reads JSON5 and writes plain JSON with key order kept (serde_json `preserve_order`). It touches only `loadOrder`, and `modOptions` for the mod being turned on for the first time or saved from its edit sheet. Both writes go through `write::update`.
- First-time on must fill the author's `enabledByDefault` checkbox and select options, minus options whose `requirements` are off: `Deploy.exe` applies no options for a mod without a `modOptions` entry.
- SMF option rules (`smf/options.rs`): only checkbox (stored as `name`) and select (stored as `group:name`, one per group) options are shown. Conditional options are hidden. An option whose required mods are off can't be chosen, because SMF drops it.
- Type overrides live in `%LOCALAPPDATA%\Vaexil Mod Manager\settings.json` as `kindOverrides` (mod id to kind key, such as `models-textures`). Unknown keys are kept. An unreadable file is never overwritten, and type changes are off until it is fixed.
- Each write: copy the old file to `%LOCALAPPDATA%\Vaexil Mod Manager\config-backups` (newest 20; a failed copy, or no `%LOCALAPPDATA%`, stops the write). Then write `config.json.vmm-<pid>.tmp`, read `config.json` again and refuse if it changed since it was read, and rename over. Any new write updates `docs/safety-model.md` in the same PR.
- A load-order id that no mod folder answers to gets its own row (kind Other, with a problem line), so it can be turned off.
- Slint: assigning a Switch's `checked` ends its binding, so the row re-syncs it on `changed item`. ScrollView already scrolls a focused control into view; don't add scroll code. UI Automation sees only on-screen rows, so smoke tests reach lower rows with Tab.
- Dev and test runs set `VAEXIL_MOD_MANAGER_GAME` (fake game folder) and `VAEXIL_MOD_MANAGER_DATA` (instead of `%LOCALAPPDATA%`). Never click switches against the real install while testing.
- SMF refreshes `cleanPackageDefinition.txt` itself but never `cleanThumbs.dat`. Stage 04 must replace a stale one after a game update.
- SMF's `cache` (about 6.4 GB) is regenerable; leave it out of backups.
- Advisory exception: pin-utils RUSTSEC-2026-0441 is ignored until 2027-02-08. Fix it together with Modlist Launcher and Keeplight.
- No AI attribution in commits or PRs. Commit with explicit pathspecs.

## Context Compaction Checklist

- Before stopping: update this file, the plan page's Now and Done tabs (same URL), and the memory note `vaexil-smf-plan`.
- Re-check `git status`, open PRs (`gh pr list`) and `main`'s CI before trusting anything above.
- Verify HITMAN's SMF version and Steam build before any work that depends on them; both change without notice.
