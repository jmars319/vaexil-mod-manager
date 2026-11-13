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

The deploy report ("Report" in the window, `vmm deploy-report`, and the `get_deploy_report` and `diagnose` tools) only reads the app's own last deploy log; `vmm diagnose` and "Copy diagnostics" in Settings read the files on this page and write nothing. "Copy diagnostics" puts its JSON on the clipboard, with paths in your user folder written as `~`.

## Written by the app

| What | When | How |
| --- | --- | --- |
| SMF's `config.json` | A mod's switch is used, Save in a mod's edit sheet with its options changed, Save in the Load order view, Use on a saved setup, or a step of a compatibility pass (start with every mod off, try a batch, mark it good or bad). From the command line: `vmm apply` of a plan that turns mods on or off, chooses options or reorders. | A switch changes only `loadOrder`: on inserts the mod's id as late as its load rules allow (usually last, as SMF's own Mod Manager does), off removes it. Save in the Load order view replaces `loadOrder` with the same ids in the new order, and refuses if a mod was turned on or off meanwhile. Using a setup, or a pass step, replaces `loadOrder` with that setup's mods (any no longer installed are left out and named), and a setup also sets its own mods' `modOptions` entries; other mods' options are kept. Moves and Auto-sort in that view change nothing until Save. A mod with options that has never had any chosen gets its author's defaults in `modOptions` (leaving out options whose required mods are off), as SMF's own Mod Manager does. Save replaces that mod's `modOptions` entry with the choice, checked against its manifest the way SMF checks it: only checkbox and select options, one per select group, none whose required mods are off. Every other key, and their order, is kept. The file is written as plain JSON, which SMF reads like the JSON5 it writes itself. |
| `config.json.vmm-<process id>.tmp` in SMF's folder | During that write, for a moment. | The new file is written here first, then renamed over `config.json` in one step, so SMF never reads half a file. Just before the rename `config.json` is read again; if another program (SMF's own Mod Manager, a second copy of this app) changed it meanwhile, nothing is written and the window says to try again. The temporary file is removed whenever the write does not complete. |
| `%LOCALAPPDATA%\Vaexil Mod Manager\settings.json` | Save in a mod's edit sheet with its type changed, `vmm apply` of a plan that sets a type, or the "Let AI tools make changes" switch in Settings. | The app's own settings: the types you chose, by mod id, and `aiChanges` (`true` only while that switch is on). Never read by SMF. Written to `settings.json.<process id>.tmp` and renamed over. Keys this version does not know are kept. If the file can't be read, it is never overwritten and type changes are off, with a note in the window. |
| `%LOCALAPPDATA%\Vaexil Mod Manager\setups.json` | Saving or deleting a setup, and each step of a compatibility pass. | Saved setups (mods on, their order and options) and the pass: its baseline, and each batch with its mods, result and notes. Never read by SMF. Written like `settings.json`, and never overwritten when it can't be read. |
| `%LOCALAPPDATA%\Vaexil Mod Manager\deploy-state.json` | When a deploy finishes. | Whether it succeeded, its exit code, its log's path, when it started and finished, and a fingerprint of `config.json` and the Mods folder at the start, so the window can tell whether anything changed since. |
| `%LOCALAPPDATA%\Vaexil Mod Manager\logs\deploy-<time>.log` | While a deploy runs. | Every line `Deploy.exe` prints, colour codes removed. The newest 20 are kept. SMF still writes its own `Deploy.log` too. |
| `%LOCALAPPDATA%\Vaexil Mod Manager\plans\<plan id>.json` | `vmm plan`. | A checked list of changes, with the SHA-256 of each file it depends on (`config.json`, and `settings.json` for a type). `vmm apply <plan id> --yes` reads it, refuses if it is over 10 minutes old, is for another game folder, or if any of those files has changed since, deletes it (a plan is used once), then makes each change through the same calls as the window, each with its backup. Plan ids are 16 hex digits; nothing else is read as a plan. Plans past their time are deleted when the next plan is made. |
| `%LOCALAPPDATA%\Vaexil Mod Manager\logs\vmm-<date>.jsonl` | Every change (from the window, `vmm` or an AI tool) and every failed operation. | The event log: one JSON line per event, with the time, the event (such as `mod.enabled` or `plan.applied`), who made it (`gui`, `cli` or `mcp:<AI app>`), the game folder, the mod or plan, a message, and the error's code when it failed. A file per day (UTC), only ever appended to; files older than 14 days are deleted when the app or `vmm` starts. Never holds file contents. If a line can't be written, the operation goes ahead. |
| `%LOCALAPPDATA%\Vaexil Mod Manager\crash-<time>.json` | Only if the app or `vmm` panics. | The panic message, where in the code it happened, the version and who was running it, written just before the process ends, so `vmm diagnose` and a bug report can show it. Never holds file contents. |
| `%LOCALAPPDATA%\Vaexil Mod Manager\journal.jsonl` | Each time a plan is applied, from `vmm apply` or an AI tool. | One JSON line per applied plan: the time, who applied it (`cli`, or `mcp:<the AI app's name>`), the game folder, the plan id, for an AI tool the user's request it acted on, and each step with whether it changed something. Only ever appended to. If it can't be written, the change stands and the result says the journal failed. |
| `%LOCALAPPDATA%\Vaexil Mod Manager\config-backups\config-<time>.json` | Before every change to `config.json`. | A copy of the file as it was. The newest 20 are kept. If the copy cannot be made, or Windows does not say where `%LOCALAPPDATA%` is, `config.json` is not changed. |

| Registry `HKCU\Environment\Path` (your own PATH) | Only when you press "Add the command line to my PATH" in Settings and then confirm "Add to PATH", after the sheet has shown the folder it will add; or "Take it out of my PATH". | Adds the folder that holds `vmm.exe` to the end of the value, or removes exactly that folder; every other entry, and the value's type, stay as they were. Then Windows is told the environment changed (`WM_SETTINGCHANGE`), so new terminals see it. The system PATH is never touched. Only the window does this (`crates/app/src/cli_path.rs`); the command line never changes PATH. |

| Steam's `userdata\<account>\config\localconfig.vdf`, HITMAN's `LaunchOptions` only (app 1659040) | Only when you press "Use Vaexil Mod Manager for Steam's Play button" in Settings and then confirm "Change Steam's launch options", after the sheet has shown the old and new value; or "Put Steam's launch options back". Refused while Steam runs (it rewrites the file on exit). | The one value is changed in place; every other byte of the file is kept. The file is replaced through `localconfig.vdf.vaexil-mod-manager-tmp` and a rename, then read back; if it doesn't read back right, the original is written back. The value before is kept in `settings.json` (`steamPlay`) and put back on undo, unless the value was changed outside the app since, when it is left alone. Ported from Modlist Launcher. Tests set `VAEXIL_MOD_MANAGER_STEAM` to a stand-in Steam folder, so they never reach the real one. |
| `%LOCALAPPDATA%\Vaexil Mod Manager\play-as-is` | When you choose to play a Steam copy without deploying (in the window, or `vmm play --no-deploy`). | The time of that choice, so Steam's Play button, which the game then starts through, doesn't ask again. Read and deleted by that next start; ignored after two minutes. |
| `%LOCALAPPDATA%\Vaexil Mod Manager\steam-backups\localconfig-<account>-<time>.vdf` | Before each change to Steam's settings above. | A copy of the file as it was. The newest 20 per account are kept. If the copy can't be made, nothing is changed. |

Enforcement:

- All writing to SMF's `config.json` is in one module, `crates/backend-smf/src/write.rs`, behind the `Backend` trait's change methods. The app's own files are written only by `crates/engine` (`settings.rs`, `setups.rs`, `store.rs`, `deploy.rs`). Tests cover key order, other keys, JSON5 input, defaults, the backup, a failed backup stopping the write, and a file it cannot read being left alone.
- The mouse-click smoke test runs the real window against a throwaway game folder and checks that a click changes only the load order and leaves a backup, and that Save in the edit sheet stores the clicked option. It opens Settings and checks that "Add the command line to my PATH" first shows what it will change and that Cancel changes nothing; it never confirms, so it never changes PATH.
- The command line's golden tests (`crates/cli/tests/golden.rs`) run `vmm --json` against a throwaway game folder, including a plan refused because `config.json` changed after it was made.
- `VAEXIL_MOD_MANAGER_DATA` moves the app's folder for tests, so they never write to the user's profile.

## AI tools

`vmm mcp` is an MCP server for AI apps (`docs/ai.md`). It speaks only over the stdin and stdout of the AI app that starts it: it opens no port and makes no network call, and the `rmcp` crate is built with its stdio transport only, so `cargo deny`'s ban on HTTP and TLS crates still holds. Its tools are the engine's own operations, with no file, command or network access. Reading is always allowed. `apply_plan`, the only tool that changes anything, is refused unless "Let AI tools make changes" is on in Settings (off by default) and the call carries the user's request, which the journal keeps. Every check and backup above applies as in the window. The MCP integration test (`crates/cli/tests/mcp.rs`) checks the refusal, the tool annotations, and that a refused plan leaves `config.json` untouched.

## Processes

- **SMF's own `Deploy.exe`,** when you press Deploy, or Play with changes since the last deploy, or run `vmm deploy --yes`, or an AI tool calls `start_deploy` (only with "Let AI tools make changes" on, and the user's request). It runs exactly as SMF's Mod Manager runs it: `Deploy.exe --doNotPause --colors` from SMF's folder, with no console window, unchanged. Its output goes to the deploy log above. Deploy and Play refuse while `HITMAN3.exe` or another `Deploy.exe` is running (read from the process list).
- **Steam,** when you press Play on a Steam copy. The app opens `steam://rungameid/1659040`, as Steam's own Play button does. With changes since the last deploy, Play first asks: deploy, then play; play the last deployed build; or cancel. Copies from other stores are not started yet; the window says so.
- **HITMAN's own command, from Steam,** once Steam's Play button is connected (below): Steam runs `vaexil-mod-manager.exe --steam-play -- <the game's command>`. The app starts that command itself, from the game's folder, and waits for it to end, so Steam sees the game running. It never goes back through Steam, so it can't loop. With changes since the last deploy it shows the same choice in a small window, and a deploy chosen there runs SMF's `Deploy.exe` as above.
- **The log,** when you press Open log: Windows opens it with whatever it uses for text files.

Nothing else is started.

## Not done, by design

- No network access, including from `vmm mcp`. CI's dependency policy bans HTTP and TLS crates.
- No telemetry or accounts.
- No changes to anything else inside HITMAN's folder or SMF's folder by the app itself: not SMF's code or mods, and no `config.json` key besides `loadOrder` and `modOptions`. What `Deploy.exe` writes when it deploys is SMF's own work, as when SMF's Mod Manager runs it. Never a save game.
- Nothing is ever sent to SMF's authors or its repository.
