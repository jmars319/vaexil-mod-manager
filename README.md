# Vaexil Mod Manager

Plan artifact: https://claude.ai/artifact/HBDVnjvCrkyQsY7Bak28Dm

A Windows mod manager in the Vaexil look. Version 1 manages [Simple Mod Framework](https://github.com/atampy25/simple-mod-framework) (SMF) mods for HITMAN World of Assassination.

Vaexil Mod Manager drives the official SMF that is already installed in your game folder. It never patches, rebuilds or replaces SMF's own code, and it sends nothing upstream. See [ADR 0001](docs/adr/0001-drive-official-smf.md).

Vaexil Mod Manager is an independent project. It is not affiliated with Simple Mod Framework, its authors, or IO Interactive.

## Status

Early development, nothing released yet. This first version is read-only:

- **Finds HITMAN** in your Steam libraries, an Epic Games install, or the Microsoft Store's `XboxGames` folders.
- **Finds SMF** in a folder inside the game folder and identifies which official release it is.
- **Lists your SMF mods**, in load order first, with each mod's version, authors and any problem reading it.
- **Changes nothing.** It writes no file anywhere, not even its own settings.

Enabling mods, setting the load order, deploying and starting the game, and keeping SMF up to date come in later stages on the plan page.

## Build from source

You need the Rust toolchain (stable, MSVC) and Visual Studio Build Tools.

```powershell
cargo test --workspace
cargo run -p vaexil-mod-manager
```

Set `VAEXIL_MOD_MANAGER_GAME` to a game folder to use it instead of looking for installs. The smoke test uses it with a throwaway fake folder.

To check that the window repaints after minimize/restore and maximize/restore, run this after a release build:

```powershell
./scripts/smoke-minimize-restore.ps1
```

## Safety

[docs/safety-model.md](docs/safety-model.md) lists every file the app reads. It writes none.

## License

GPL-3.0-only. The embedded Geist fonts are under the SIL Open Font License 1.1 (`assets/fonts/OFL.txt`).
