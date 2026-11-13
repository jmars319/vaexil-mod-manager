# Vaexil Mod Manager project state

Plan artifact: https://claude.ai/artifact/HBDVnjvCrkyQsY7Bak28Dm

The working state around the plan page and the docs. The plan page holds the roadmap and the ledger; `docs/safety-model.md` and `docs/adr/` hold the rules. Checked 2026-11-13 on JASON-PC.

## Core Project Architecture & Stack

- Windows-native desktop app: Rust (edition 2024, MSRV 1.92) with Slint 1.18 (winit, software renderer, accessibility), per ADR-0034. Workspace (ADR 0003, E1): `crates/core` (`vmm-core`: `Backend` trait, `VmmError` with stable codes, game discovery, `order`), `crates/backend-smf` (`vmm-backend-smf`), `crates/engine` (`vmm-engine`: `Engine`, the app's own files, deploy, `host` Windows calls), `crates/app` (the window, which calls only the engine).
- Drives the official Simple Mod Framework already in the HITMAN folder (ADR 0001). It never patches, rebuilds or replaces SMF, and it uses only `config.json`, the `Mods` folder and `Deploy.exe --doNotPause --colors`.
- `Backend` trait (`crates/core/src/backend.rs`): `inspect`, `set_enabled`, `set_options`, `set_load_order`, `current_setup`, `apply_setup`, `deploy_fingerprint`, `deploy`; errors are `VmmError`. `vmm_backend_smf::Smf` is the only backend.
- Game discovery: Steam (registry plus `libraryfolders.vdf`, app 1659040), Epic manifests, Microsoft Store `XboxGames`. SMF version comes from `Deploy.exe`'s SHA-256 in `smf/releases.rs`.
- UI: `crates/app/ui/app.slint` plus `theme.slint`, which is byte-identical to Modlist Launcher's (blob `6ba42346`); change both repos together. Geist fonts are embedded.
- CI (`.github/workflows/ci.yml`, windows-latest): fmt, clippy `-D warnings`, tests, release build, both smoke scripts, then cargo-deny. HTTP and TLS crates are banned until stage 04. Releases: a `v*` tag makes a draft, and publishing it copies the installer to `JAMARQ-Digital-LLC/jamarq-releases` (needs the `JAMARQ_RELEASES_TOKEN` secret, which is set).

## Current Structural Milestones / Active Feature Goals

- Stage 00 (direction, ADR-0034) and stage 01 (repo, read-only first window, #1) are done.
- Stage 02, in progress since 2026-11-13, in four PRs:
  1. Mods view grouped by type, with on/off switches. #12, merged 2026-11-13 as 118506b. The app's first write.
  2. Edit sheet per mod: options (`modOptions`) and a type override kept in the app's own `settings.json`. #14, merged 2026-11-13 as d43e161, after two Codex findings were fixed.
  3. Load order view: drag or Up/Down, Auto-sort by `loadBefore`/`loadAfter`, broken-rule warnings, and "replaced by" markers on both views. A draft until Save. Merged 2026-11-13 as #16 (7615d18), after two Codex findings were fixed (npm range semantics, stale file lists).
  4. Setups as profiles with compatibility passes (ADR 0003 section 7): a known-good baseline, small themed batches, each tested (deploy, log, play) and marked good, which joins the baseline, or bad, which is turned off in one step. Then Play (deploy if changed, then start HITMAN through Steam) and Deploy, both blocked while HITMAN3.exe or Deploy.exe runs. Merged 2026-11-13 as #17 (ba4145d), after two Codex findings were fixed.
- Scope widened 2026-11-13: a multi-game mod manager that any LLM can operate (Claude, Gemini, GPT, local models). ADR 0003 (`docs/adr/0003-headless-engine-and-llm-interfaces.md`, accepted 2026-11-13 with the owner's decisions) adds a `vmm-engine` crate that every front end calls, backends as their own crates (SMF now, Cyberpunk 2077 later), one versioned JSON schema, a `vmm` CLI with `--json`, a stdio MCP server (`vmm mcp`), plan-then-apply for every change, and JSON logs and diagnosis. The owner's order (2026-11-13): finish stage 02 (PR 3 and PR 4) first. Then the ADR sequence: engine, CLI, MCP, then logs and diagnosis with old stage 03 folded in; SMF features onto the engine; Steam Play via `vmm play` (old 05); SMF updates as an engine job (old 04); first release (old 06); then the deploy engine, the `mo2` backend and Cyberpunk. No intended users for now: builds stay unsigned until the owner decides, and the owner wants automatic updates on JASON-PC from the jamarq-releases feed. AI tools stay read-only until the user asks for a specific change. Setup outside VMM (PATH, AI app registration) happens from the window's Settings. Deploys for other games use hard links with a ledger, plus an `mo2` backend for existing MO2 setups, so Modlist Launcher can later be absorbed. Each backend states its `nativeMods` status explicitly.
- Old stages 03 to 06 now sit inside the ADR 0003 sequence (see above). The old repo `JAMARQ-Digital-LLC/vaexil-simple-mod-framework` is archived, and its Dependabot #24 closed, after the app's first deploy on this PC.

## Active Task State & Known Regressions

- Stage 02 is done: `main` was at #17 (ba4145d) when it was installed on JASON-PC. The owner's first deploy through VMM (2026-11-13 03:19 to 03:34) succeeded: Deploy.exe exit 0, "Done in 15m", 0 errors in VMM's log. Overnight (owner's go, 2026-11-13): ADR steps E1 to E5, each a PR merged when green, waiting on CI allowed for tonight, nothing installed until the owner is back, and stop after E5 or at the first point that needs the owner. The old fork is archived once the deploy has succeeded (it did).
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
- Load order (`crates/core/src/order.rs`): the later mod wins. Auto-sort is a stable depth-first placement, so a mod's prerequisites move up to just before it and nothing else moves; contradicting rules are named and nothing changes. A newly enabled mod goes as late as its rules allow. SMF's own Mod Manager re-sorts on every Apply, and `Deploy.exe` doesn't sort, so the order VMM saves is the one deployed.
- Rules: an id or `[id, range]`. Ranges use the `semver` crate; a version or range that can't be read never matches, as with npm semver. Rules from chosen options count; conditional options don't (filtrex isn't evaluated).
- "Replaced by" counts only hash-named raw files (`HASH.TYPE`) in content folders, including chosen options' folders. Patches and `.meta` files never count, so it can miss an override but never invents one. On JASON-PC, Misc Outfit Mesh and Texture Fixes replaces some of the Community Patch's files.
- Setups and the pass live in `%LOCALAPPDATA%\Vaexil Mod Manager\setups.json`. One pass at a time. Its baseline is ordered; a good batch's mods join it in the order they were tried; a bad batch puts the game back to the baseline. Batch mods are placed with `order::insert_position`. Ending a pass keeps the mods as they are.
- Deploy runs `Deploy.exe --doNotPause --colors` from SMF's folder on a worker thread (CREATE_NO_WINDOW), and the window polls it every 300 ms. Logs go to `logs\deploy-*.log` (newest 20). `deploy-state.json` keeps the fingerprint taken at the start of a successful deploy: config.json's bytes, plus every file under Mods by path, size and time (about 0.1 s to walk). Play and Deploy always take it fresh. The status line reuses one while `deploy::quick_key` (config.json and the top of Mods) is unchanged, for up to 30 s. A setup or pass step gives a never-configured mod its default options, as the switch does. Play uses `steam://rungameid/1659040`; other stores get a plain refusal.
- The smoke test's Deploy.exe is `crates/app/examples/fake-deploy.rs`, built in CI before the smoke tests. The smoke helper's Click now fails on a control it can't find (off screen), which once let two pass checks pass by accident.
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
