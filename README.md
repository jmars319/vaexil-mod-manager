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

Mod options, the load order view, setups, deploying and starting the game, and keeping SMF up to date come in later stages on the plan page.

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
