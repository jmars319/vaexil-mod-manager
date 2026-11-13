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
| SMF's `Mods` folder and each mod's `manifest.json` | The mod list: name, id, version, authors, options and load rules (`loadBefore`/`loadAfter`, including those of chosen options). File names inside each mod (never their contents) show what kind of mod it is, for grouping, and which hash-named game files it replaces, for the "replaced by" warnings. |

Enforcement:

- Every file above is opened with read-only calls (`std::fs::read`, `read_to_string`, `read_dir`, metadata).
- A test snapshots a fake SMF install (every file's bytes and modified time, and every entry) before and after inspecting it twice, and fails on any change (`inspecting_never_modifies_the_install`).

## Written by the app

| What | When | How |
| --- | --- | --- |
| SMF's `config.json` | A mod's switch is used, Save in a mod's edit sheet with its options changed, or Save in the Load order view. | A switch changes only `loadOrder`: on inserts the mod's id as late as its load rules allow (usually last, as SMF's own Mod Manager does), off removes it. Save in the Load order view replaces `loadOrder` with the same ids in the new order, and refuses if a mod was turned on or off meanwhile. Moves and Auto-sort in that view change nothing until Save. A mod with options that has never had any chosen gets its author's defaults in `modOptions` (leaving out options whose required mods are off), as SMF's own Mod Manager does. Save replaces that mod's `modOptions` entry with the choice, checked against its manifest the way SMF checks it: only checkbox and select options, one per select group, none whose required mods are off. Every other key, and their order, is kept. The file is written as plain JSON, which SMF reads like the JSON5 it writes itself. |
| `config.json.vmm-<process id>.tmp` in SMF's folder | During that write, for a moment. | The new file is written here first, then renamed over `config.json` in one step, so SMF never reads half a file. Just before the rename `config.json` is read again; if another program (SMF's own Mod Manager, a second copy of this app) changed it meanwhile, nothing is written and the window says to try again. The temporary file is removed whenever the write does not complete. |
| `%LOCALAPPDATA%\Vaexil Mod Manager\settings.json` | Save in a mod's edit sheet with its type changed. | The app's own settings: the types you chose, by mod id. Never read by SMF. Written to `settings.json.<process id>.tmp` and renamed over. Keys this version does not know are kept. If the file can't be read, it is never overwritten and type changes are off, with a note in the window. |
| `%LOCALAPPDATA%\Vaexil Mod Manager\config-backups\config-<time>.json` | Before every change to `config.json`. | A copy of the file as it was. The newest 20 are kept. If the copy cannot be made, or Windows does not say where `%LOCALAPPDATA%` is, `config.json` is not changed. |

Enforcement:

- All writing is in one module, `crates/core/src/smf/write.rs`, behind `Framework::set_enabled`. Tests cover key order, other keys, JSON5 input, defaults, the backup, a failed backup stopping the write, and a file it cannot read being left alone.
- The mouse-click smoke test runs the real window against a throwaway game folder and checks that a click changes only the load order and leaves a backup, and that Save in the edit sheet stores the clicked option.
- `VAEXIL_MOD_MANAGER_DATA` moves the app's folder for tests, so they never write to the user's profile.

## Processes

- None are started. SMF's `Deploy.exe` is read for its hash and never run. A switch takes effect in the game at the next deploy, which is still done in SMF.

## Not done, by design

- No network access. CI's dependency policy bans HTTP and TLS crates.
- No telemetry or accounts.
- No changes to anything else inside HITMAN's folder or SMF's folder: not SMF's code, mods or deploy output, and no `config.json` key besides `loadOrder` and a first-time `modOptions` entry.
- Nothing is ever sent to SMF's authors or its repository.
