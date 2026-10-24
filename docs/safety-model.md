# Safety model

Vaexil Mod Manager's promise is that Simple Mod Framework stays exactly as its authors publish it. It never patches, rebuilds or replaces SMF's code, and it sends nothing upstream. This page lists everything the app touches. It is updated in the same pull request as any change to that list.

## Read, never written

| What | Why |
| --- | --- |
| Registry `HKCU\Software\Valve\Steam\SteamPath` | Where Steam is installed. |
| Steam `steamapps\libraryfolders.vdf`, `steamapps\appmanifest_*.acf` | Find HITMAN World of Assassination (Steam app 1659040) in any Steam library. |
| `%ProgramData%\Epic\EpicGamesLauncher\Data\Manifests\*.item` | Find an Epic Games install of HITMAN. |
| `X:\XboxGames\*\Content\MicrosoftGame.Config` (existence only) | Find a Microsoft Store install of HITMAN. |
| The game's `Retail\HITMAN3.exe` or `Retail\Runtime\chunk0.rpkg` (existence only) | Confirm the folder is HITMAN, the same check SMF's own Mod Manager makes. |
| SMF's `Deploy.exe` (read in full, never run) | Its SHA-256 identifies the installed SMF release. It is read again only when its size or modified time changes. |
| SMF's `config.json` | The load order. Other keys are ignored. |
| SMF's `Mods` folder (names only) and each mod's `manifest.json` | The mod list: name, id, version and authors. |

Enforcement:

- Every file is opened with read-only calls (`std::fs::read`, `read_to_string`, `read_dir`, metadata). The core crate has no code that writes.
- A test snapshots a fake SMF install (every file's bytes and modified time, and every entry) before and after inspecting it twice, and fails on any change (`inspecting_never_modifies_the_install`).

## Written by the app

Nothing, in this version. There is no settings file and no log yet. When a later stage adds one, it goes in `%LOCALAPPDATA%\Vaexil Mod Manager\` and this page says so first.

## Processes

- None are started. SMF's `Deploy.exe` is read for its hash and never run.

## Not done, by design

- No network access. CI's dependency policy bans HTTP and TLS crates.
- No telemetry or accounts.
- No changes to anything inside HITMAN's folder or SMF's folder: not SMF's code, config, mods or deploy output.
- Nothing is ever sent to SMF's authors or its repository.
