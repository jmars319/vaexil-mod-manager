# Vaexil Mod Manager

Plan artifact: https://claude.ai/artifact/HBDVnjvCrkyQsY7Bak28Dm

A Windows mod manager in the Vaexil look. Version 1 manages [Simple Mod Framework](https://github.com/atampy25/simple-mod-framework) (SMF) mods for HITMAN World of Assassination.

Vaexil Mod Manager drives the official SMF that is already installed in your game folder. It never patches, rebuilds or replaces SMF's own code, and it sends nothing upstream. See [ADR 0001](docs/adr/0001-drive-official-smf.md).

Vaexil Mod Manager is an independent project. It is not affiliated with Simple Mod Framework, its authors, or IO Interactive.

## Status

Early development, nothing released yet. It can now:

- **Find HITMAN** in your Steam libraries, an Epic Games install, or the Microsoft Store's `XboxGames` folders.
- **Find SMF** in a folder inside the game folder and identify which official release it is.
- **List your SMF mods** grouped by what they change (Gameplay, Models & Textures, Sound, Dialogue, UI, Text, Other), with each mod's load position, version, authors and any problem reading it. An id in the load order that no mod answers to gets its own row, so it can be turned off. The grouping is worked out from each mod's files and is information only.
- **Turn mods on and off** with a switch. This changes the load order in SMF's `config.json` exactly as SMF's own Mod Manager does, after keeping a copy of the old file. The change reaches the game at the next deploy, which is still done in SMF. Everything it writes is listed in the [safety model](docs/safety-model.md).
- **Set the load order** in the Load order view: drag a row or use Up and Down, or Auto-sort by the mods' own `loadBefore`/`loadAfter` rules. Broken rules, and mods whose files replace another's, are shown on both views. Nothing is saved until Save.
- **Change a mod's options and type** in its edit sheet. Options are saved to SMF's `config.json` as SMF's own Mod Manager saves them, after keeping a copy. The type only regroups the Mods view and is kept in the app's own `settings.json`.
- **Keep setups:** save the mods that are on, in their order and with their options, and switch between saved setups.
- **Run a compatibility pass** after a game update: start with every mod off or from the mods on now, try small batches, and mark each good (it joins the baseline) or bad (the game goes back to the baseline). Saves are never touched.
- **Deploy and Play:** run SMF's own `Deploy.exe` unchanged, with its output in a log, and start HITMAN through Steam. Deploy and Play refuse while the game is running. **Report** reads that log into stages, warnings grouped by mod (each counted once) and errors.
- **Logs and diagnostics:** every change and failure goes to an event log with who made it (the window, `vmm`, or an AI app), and Settings' **Copy diagnostics** copies everything needed to look into a problem.

Keeping SMF current and Steam's own Play button come in later stages on the plan page.

## Command line

`vmm.exe` ships beside the app and runs the same engine as the window, so a terminal or an AI tool can use Vaexil Mod Manager without screen control. Reading never changes anything: `vmm status`, `vmm mods`, `vmm mod <id>`, `vmm order`, `vmm logs`, `vmm deploy-report`, `vmm diagnose`. A change is planned first and then applied by its plan id, which refuses if anything changed in between:

```
vmm plan enable Some.Mod.Id
vmm apply <plan id> --yes
```

Setups (`use-setup`, `save-setup`, `delete-setup`), auto-sort and the compatibility pass (`start-pass`, `try-batch`, `mark-batch`, `end-pass`) are planned the same way, and `vmm setups` shows them. `vmm deploy --yes` runs SMF's `Deploy.exe` and waits for its report.

Add `--json` to any command for one JSON object in the shapes under [`schema/`](schema/) (`vmm schema` writes them; `vmm errors` lists the error codes). Exit codes: 0 ok, 2 bad usage, 3 refused by a rule, 4 changed meanwhile, 5 the game or a deploy is running, 10 unexpected. Settings in the window can put `vmm` on your PATH; nothing else does.

## AI apps

`vmm mcp` is an MCP server, so Claude Code, Gemini CLI, Codex CLI or another MCP client can read your mods and, when you ask, change them through structured tools rather than screen control. It works over stdio only and makes no network connection. Changes from AI tools are off until you turn on "Let AI tools make changes" in Settings, and each one is recorded with your request. Setup and the tool list are in [docs/ai.md](docs/ai.md).

## Build from source

You need the Rust toolchain (stable, MSVC) and Visual Studio Build Tools.

```powershell
cargo test --workspace
cargo run -p vaexil-mod-manager
```

Set `VAEXIL_MOD_MANAGER_GAME` to a game folder to use it instead of looking for installs, and `VAEXIL_MOD_MANAGER_DATA` to a folder to use instead of `%LOCALAPPDATA%\Vaexil Mod Manager`. The smoke tests use both with throwaway folders.

After a release build, these check that the window repaints after minimize/restore and maximize/restore, and that real mouse clicks and keyboard focus work on the mod switches:

```powershell
./scripts/smoke-minimize-restore.ps1
./scripts/smoke-mouse-clicks.ps1
```

`scripts/package.ps1` builds the portable zip and, when Inno Setup 6 is installed, the per-user installer into `dist/`. Releases: a `v*` tag builds those into a draft GitHub release. Publishing the draft also copies the installer to [jamarq-releases](https://github.com/JAMARQ-Digital-LLC/jamarq-releases) as `vaexil-mod-manager-<version>-x64-setup.exe`, where `vaexil-mod-manager-latest/release.json` names the newest one and its SHA-256 (`.github/workflows/publish-feed.yml`).

## Safety

[docs/safety-model.md](docs/safety-model.md) lists every file the app reads. It writes none.

## License

GPL-3.0-only. The embedded Geist fonts are under the SIL Open Font License 1.1 (`assets/fonts/OFL.txt`).
