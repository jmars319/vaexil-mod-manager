# Using Vaexil Mod Manager from an AI app

Vaexil Mod Manager has an MCP server, `vmm mcp`. Through it, an AI app such as Claude Code, Gemini CLI or Codex CLI can read your mods and change them when you ask, as structured tool calls. It doesn't need screen control. The server is `vmm.exe`, which ships beside the app. It runs only while the AI app runs it, speaks over that app's stdin and stdout, and makes no network connection.

## Connect an app

Settings in Vaexil Mod Manager shows these lines with the real path filled in. Run the line for your app once, in a terminal:

```
claude mcp add --scope user vaexil-mod-manager -- "C:\path\to\vmm.exe" mcp
gemini mcp add --scope user vaexil-mod-manager "C:\path\to\vmm.exe" mcp
codex mcp add vaexil-mod-manager -- "C:\path\to\vmm.exe" mcp
```

Each line is that app's own command for adding a server, and it changes only that app's settings. Vaexil Mod Manager doesn't edit other apps' settings. `--scope user` makes the server available in every project rather than only the current one. For another MCP client, the server is the command `vmm.exe` with the single argument `mcp`, over stdio.

## What an AI app can do

| Tool | What it does | Changes anything |
| --- | --- | --- |
| `list_games` | Every install of the game found. | No |
| `get_status` | The game, SMF and its version, mods on, whether the game has the current mods, setups, any compatibility pass. | No |
| `list_mods`, `get_mod` | Mods with their place in the order, type, options (and the value to choose each), load rules and file overrides. | No |
| `get_load_order` | The order of the mods that are on, the rules it breaks, and an order that keeps them. | No |
| `list_error_codes` | Every error code, what it means and what to do. | No |
| `get_logs` | The newest events: every change, who made it (`gui`, `cli` or `mcp:<app>`), and every failure with its code. | No |
| `get_deploy_report` | The last deploy, read from SMF's output: result, duration, stages, warnings grouped by kind and mod (each counted once), errors. | No |
| `diagnose` | All of it at once for working out a problem, with paths in the user folder written as `~`. | No |
| `plan_change` | Checks the changes you asked for and returns a plan: its steps in plain words and a `planId`. | No (it saves the plan for 10 minutes) |
| `apply_plan` | Applies a plan once, if nothing it depends on has changed since. | Yes |

Results are the same JSON as `vmm --json`, in the shapes under [`schema/`](../schema/). Each tool declares its output schema, and the read tools are marked read-only. On failure a tool returns an error result (`isError`) holding an `ErrorReport`: a stable `code`, a `message`, a `hint` and whether retrying can work.

A change is one or more of these, in order: turn mods on or off, choose a mod's options, set the type shown for a mod, or a new load order. Every check that applies in the window applies here too, and each change keeps a backup of SMF's `config.json`, as in the window. Deploying and starting the game stay in the window.

## Your control

- **Off until you turn it on.** "Let AI tools make changes" in Settings is off by default. While it's off, AI tools can read and plan, but `apply_plan` is refused with `ai.changes_off`.
- **Only what you ask for.** The server tells the model to change nothing on its own initiative, and to show you a plan's steps before applying it. `apply_plan` is marked destructive, so the app asks you before it runs, and it needs your request in your own words.
- **A record.** Every applied plan is appended to `journal.jsonl` in `%LOCALAPPDATA%\Vaexil Mod Manager`. It records who applied it (`mcp:<app name>`, or `cli`), your request, and each step.
- **Nothing else.** The server has no tool for files, commands or the network. It can do only what the window does, and less: it can't deploy or start the game.

## For example

After a game update, to find which mods still work:

> Start with all mods disabled.

The app calls `get_status` and `list_mods`, then `plan_change` with one `setEnabled` per mod that is on. It shows you the plan, then calls `apply_plan` with that `planId` and your request. You then deploy in Vaexil Mod Manager and test the game.

## The command line

Everything above is also available without MCP: `vmm status --json`, `vmm plan …`, `vmm apply <plan id> --yes`. See the README.
