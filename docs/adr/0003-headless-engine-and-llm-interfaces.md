# ADR 0003: A headless engine, game backends and interfaces any LLM can use

Date: 2026-11-13
Status: proposed (the owner's open decisions are at the end)

## Context

Vaexil Mod Manager is growing from a HITMAN tool into a mod manager for several games. It starts with Simple Mod Framework (SMF) and should eventually replace Mod Organizer 2 for games such as Cyberpunk 2077. The owner also wants any language model, whether Claude, Gemini, GPT or a local model, to use the app directly through structured calls, without screen control.

Today (after #14) the app is two crates:

- `vmm-core` holds game discovery, the `Framework` trait (`inspect`, `set_enabled`, `set_options`) and its only module, `smf`. All writes go through `smf::write::update`: a backup, a re-check that the file hasn't changed, then one rename.
- `vaexil-mod-manager` is the Slint window. It calls `vmm-core` directly and keeps the user's type overrides in its own `settings.json`.

That shape is close, but the window holds logic (type overrides, error wording, the one-write-at-a-time rule) that a second front end would have to duplicate. The trait is also shaped around SMF.

What the game backends look like, checked 2026-11-13:

| | SMF (HITMAN) | Cyberpunk 2077 |
| --- | --- | --- |
| Mod formats | Framework mods (a `manifest.json` with content folders, options and load rules) and RPKG-only mods | `.archive` files in `archive/pc/mod`, REDmod packages in `mods/` (an `info.json` plus `archives/`), redscript in `r6/scripts`, Cyber Engine Tweaks Lua mods under `bin/x64/plugins/cyber_engine_tweaks/mods` |
| Settings | `config.json` (JSON5): `loadOrder`, `modOptions` | Load order files; MO2 writes `r6/cache/modded/MO_REDmod_load_order.txt` and `archive/pc/mod/modlist.txt` |
| Deploy | `Deploy.exe` builds RPKG patch archives (`chunkNpatchM.rpkg`) into `Runtime` | Files placed into the game folder, then `redMod deploy` compiles REDmod scripts and tweaks into `r6/cache` |
| Load orders | One | Two: REDmod, and archives. Legacy archives load before REDmods. |

Sources: SMF `docs/Manifest.md`, the REDmodding wiki ("Using Mods", "Core Mods explained"), and MO2's basic_games wiki page for Cyberpunk 2077.

On the AI side, the Model Context Protocol (spec 2026-07-28) is supported by Claude Code, Gemini CLI and Codex CLI, which all start a local server over stdio from their config files. A tool can declare an `outputSchema` and return `structuredContent`, carry `readOnlyHint` or `destructiveHint` annotations, and report a tool execution error (`isError`) the model can act on. The protocol keeps no session, so state that spans calls has to be an explicit handle. The Tasks extension covers long-running work. The official Rust SDK is `rmcp` (3.10, Apache-2.0), with a stdio transport.

## Decision

### 1. One engine, every front end a thin client

```
crates/
  core/           vmm-core: the contract. Schema types (serde + schemars), error codes,
                  the Backend trait, game discovery. No front end, no writes of its own.
  backend-smf/    vmm-backend-smf: SMF for HITMAN (today's core::smf, moved).
  backend-cp2077/ later: archives, REDmod, redscript and CET as one backend.
  engine/         vmm-engine: the only API front ends call. Picks the backend, runs
                  queries, plans and applies changes, runs jobs (deploys), holds locks,
                  keeps the journal and the JSON logs, and owns the app's settings.
  cli/            vmm.exe: every engine operation as a command, plus `vmm mcp`.
  app/            the Slint window, a client of vmm-engine like the CLI.
```

- Front ends depend on `vmm-engine` and `vmm-core` only, never on a backend crate. A CI check (`cargo tree -p vaexil-mod-manager -p vmm-cli`) enforces it.
- **No daemon.** The GUI, the CLI and the MCP server each run the engine in their own process. They coordinate through files in `%LOCALAPPDATA%\Vaexil Mod Manager`:
  - `locks\<game>.lock`: one writer per game at a time, holding its process id. A deploy holds it for the whole run.
  - `jobs\<id>.json`: what a running or finished job is doing.
  - The journal and the logs (section 4).

  The window already rereads state every 5 seconds, so it shows changes made from the CLI or by an AI. A named-pipe service is added only if this proves too slow, and nothing here prevents adding one.
- The engine API is synchronous Rust. Only `vmm mcp` brings in an async runtime (tokio, which `rmcp` needs), at its edge.

### 2. One schema for every game

All front ends speak the same versioned types, defined once in `vmm-core` with `serde` and `schemars`. `vmm schema` prints them as JSON Schema files, which are committed under `schema/`. CI fails when the committed schema and the code disagree, and a breaking change bumps `schemaVersion`.

- `Game`: id, name, store, folder, version or build.
- `Backend`: id (`smf`), name, version (SMF 2.33.45), and capabilities:
  - `orders`: the load orders it has, such as `["smf"]` or `["redmod", "archive"]`;
  - `options`: whether mods have options;
  - `deploy`: how it deploys, as `external_tool` or `place_files` steps;
  - `needsDeploy`: whether changes reach the game only after a deploy.
- `Mod`: `{ id, backend, name, version, authors, kind, kindSource: "files" | "user", enabled, positions: { <order>: n }, options: [...], requires: [...], loadBefore: [...], loadAfter: [...], problems: [Problem], files: { count, bytes } }`.
- `Problem` and `Conflict`: `{ code, severity, mods, detail }`. Examples: `file_overwrite` (with the winner), `rule_broken` (`loadBefore`/`loadAfter`), `missing_requirement`, `manifest_unreadable`, `orphaned_load_order_id`.
- `Change`: `set_enabled`, `set_options`, `reorder`, `auto_sort`, `set_kind`, `switch_setup`.
- `Plan`: `{ planId, game, changes, steps: [{ backend, description, files }], preconditions: [{ path, sha256 }], expiresAt }`.
- `Job`: `{ jobId, kind: "deploy", state, startedAt, stage, progress, report }`.
- `ErrorReport` (section 4).

Each backend maps its own world onto these types. Its raw data (an SMF manifest, a REDmod `info.json`) is available under `mod.native` for diagnosis, and nothing else depends on it. The `Backend` trait in `vmm-core` replaces `Framework`:

```rust
pub trait Backend {
    fn info(&self, game: &Game) -> BackendInfo;          // capabilities, version
    fn snapshot(&self, game: &Game) -> Result<State, VmmError>;   // read only
    fn plan(&self, game: &Game, change: &Change) -> Result<Vec<Step>, VmmError>; // no writes
    fn apply(&self, game: &Game, steps: &[Step], ctx: &mut Apply) -> Result<(), VmmError>;
    fn deploy(&self, game: &Game, job: &mut JobCtx) -> Result<DeployReport, VmmError>;
}
```

`set_kind` and setups are engine-level, because they live in the app's settings and not in the game.

### 3. Three ways in, one set of operations

| Operation | GUI | CLI (`vmm …`) | MCP tool |
| --- | --- | --- | --- |
| What games and backends are there | header | `games` | `list_games` (read-only) |
| Current state | Mods view | `status`, `mods list`, `mod show <id>` | `get_status`, `list_mods`, `get_mod` (read-only) |
| Load order and conflicts | Load order view | `order show`, `conflicts` | `get_load_order`, `check_conflicts` (read-only) |
| Plan a change | (implicit) | `plan <change.json>` | `plan_change` (read-only: returns `planId` and what would change) |
| Apply a plan | switch, Save | `apply <planId> --yes` | `apply_plan` (destructive) |
| Deploy | Deploy, Play | `deploy --yes [--wait]` | `start_deploy` (destructive, returns `jobId`), `get_job` |
| Logs and diagnosis | Copy diagnostics | `logs --json`, `diagnose --json` | `get_logs`, `diagnose` (read-only) |

- **CLI.** Every command takes `--json` and then prints exactly one JSON object on stdout: the result, or an `ErrorReport`. Without `--json` it prints plain text. Exit codes are stable: 0 ok, 2 bad usage, 3 refused by a rule, 4 changed meanwhile, 5 game or deploy running, 10 unexpected. Any agent that can run a shell command can use it, with or without MCP.
- **MCP.** `vmm mcp` is a stdio server (`rmcp`, `transport-io` only). It opens no port and makes no network call. Each tool's input and output schemas come from the same `schemars` types. Resources expose `vmm://state`, `vmm://mods/{id}` and `vmm://logs/latest`. A deploy runs as a job, through the Tasks extension where the client supports it and through `get_job` everywhere else. Setup lines for Claude Code, Gemini CLI and Codex CLI go in `docs/ai.md`.
- **Action plans.** Every change, from any front end, is planned before it is applied. The plan lists its steps and the file hashes it depends on. Applying it rechecks those hashes, extending today's "changed meanwhile" check, and refuses on any difference. A plan expires after 10 minutes. The `planId` is the explicit handle the MCP spec recommends, since MCP keeps no session.

### 4. Errors and logs a model can read

- `VmmError` becomes a typed error with a stable `code` such as `config.changed_meanwhile`, `backup.unavailable`, `game.running`, `deploy.tool_failed`, `backend.version_unsupported` or `option.requirement_off`. Each carries:
  - a plain `message` for people;
  - a `hint` saying what to do;
  - `retryable`;
  - `details` (paths, mod ids, versions);
  - a `cause` chain.

  The window shows the message and hint. `--json` and MCP return the whole object, and MCP marks it `isError`. `docs/errors.md` is generated from the code list.
- **Logs:** JSON Lines in `logs\vmm-YYYY-MM-DD.jsonl`, kept for 14 days. Each line is `{ ts, level, event, caller, game, backend, mod, job, message, error }`. `caller` is `gui`, `cli` or `mcp:<client name>`, so every change records who made it.
- **Deploy reports:** the backend reads its tool's output (SMF's log lines) into `{ outcome, durationMs, stages, warnings: [{ code, mod, detail }], errors, rawLog }`. A line it doesn't recognise stays in the raw log and never fails the deploy.
- **`vmm diagnose --json`:** one object with
  - app, backend and game versions;
  - what was detected and why;
  - current problems and conflicts;
  - the last deploy report and the last 200 log events;
  - settings.

  Paths are shown relative to the user's profile.
- **Crashes:** a panic hook writes `crash-<time>.json` with the same fields before the process exits.

### 5. Safety, unchanged and extended

- Every rule in `docs/safety-model.md` holds for every front end, enforced in the engine and not in the window: SMF stays untouched, a backup comes before each write, and nothing deploys while the game runs.
- AI clients get the same operations as the window and nothing more: no raw file access and no arbitrary commands.
- Changes need an explicit step. The CLI needs `--yes`, and MCP write tools are marked destructive so clients ask the user first. A Settings switch, "Let AI tools make changes", can turn every MCP write tool off.

### 6. How it is checked

- Golden JSON fixtures for every CLI command, run against fake game folders (`VAEXIL_MOD_MANAGER_GAME` and `VAEXIL_MOD_MANAGER_DATA`).
- An integration test starts `vmm mcp`, lists its tools, and calls each one over stdio.
- The schema files are diffed in CI.
- Before adding `rmcp`: confirm that `cargo deny` still passes. The ban on HTTP and TLS crates (`hyper`, `reqwest`, `rustls`, `native-tls`) must hold with only the stdio transport enabled.

## Sequence

This work comes before the rest of stage 02, so that the load order, setups and deploy are built engine-first and reach the window, the CLI and MCP in the same PR.

1. **Engine extraction.**
   - Create `vmm-engine` and move `smf` into `vmm-backend-smf`.
   - `Framework` becomes `Backend`, and the window calls only the engine.
   - Add typed errors with codes.
   - No visible change.
2. **Schema and CLI.**
   - Schema types, JSON Schema files and the CI diff.
   - `vmm.exe` with the read commands, and plan and apply for on/off, options and type.
   - The installer puts `vmm.exe` beside the app and on the user's PATH.
3. **MCP server.**
   - `vmm mcp` with the read tools and `plan_change`/`apply_plan`.
   - `docs/ai.md` with setup for Claude Code, Gemini CLI and Codex CLI.
   - The Settings switch.
4. **Logs and diagnosis.** JSON logs, `diagnose`, the crash hook.
5. **Stage 02 continues engine-first:** the load order view with Auto-sort and conflicts, then setups, Play and Deploy, with deploy as a job.
6. **A second backend (Cyberpunk 2077),** after the first release, once the decisions below are made.

## Consequences

- Each feature is written once in the engine and is then usable from the window, the shell and any MCP client.
- More crates, plus one async dependency, kept to the MCP edge.
- The JSON schema becomes a public contract. Changing it means a version bump and updated fixtures.
- Deploys can be started and watched by an AI, and the journal records every change and who made it.

## Open decisions for the owner

1. **Order of work.** Default: the headless steps 1 to 4 come before the load order view, as above.
2. **What AI tools may change.** Default:
   - reading is always allowed;
   - changes go through plan and apply and are marked destructive, so the client asks you;
   - deploys are allowed;
   - a Settings switch turns all AI changes off, and it starts on.
3. **The CLI on PATH.** Default: the installer adds `vmm.exe` to your user PATH, so any agent can find it.
4. **How Cyberpunk mods reach the game,** decided before that backend starts. Default: hardlinks from a staging folder, with a ledger and backups so everything can be undone, as Vortex does. Not a virtual file system like MO2's usvfs.
5. **Records.** Once accepted: a decision-log entry and updated registry wording in agentic-instructions (product scope: a multi-game, AI-operable mod manager).
