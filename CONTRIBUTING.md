# Contributing

Bug reports and pull requests are welcome. Vaexil Mod Manager drives the official Simple Mod Framework and never changes it, so most of the work is in reading SMF's files carefully and showing them clearly.

## Before you start

- For anything bigger than a fix, open an issue first so we can agree on the approach.
- Read the [safety model](docs/safety-model.md). Changes must keep its promises:
  - never change SMF's own code or the files it ships, and never send anything to SMF's authors ([ADR 0001](docs/adr/0001-drive-official-smf.md));
  - write only what the safety model lists, and update it in the same pull request as any new write;
  - make no network connections;
  - need no admin rights.
- Design decisions are recorded in [docs/adr](docs/adr). The look comes from the Vaexil theme in `crates/app/ui/theme.slint`, which is copied unchanged from Modlist Launcher (ADR 0002). Use its components and `Theme` tokens rather than new colours, and do not edit that file here; change it in Modlist Launcher and copy it again.
- The workspace is split by job (ADR 0003): `crates/core` (`vmm-core`) is the shared contract (the `Backend` trait in `backend.rs`, `VmmError` and its stable codes in `error.rs`, game discovery, load-order logic); `crates/backend-smf` (`vmm-backend-smf`) is the SMF backend; `crates/engine` (`vmm-engine`) is every operation front ends share; `crates/app` is the window. The window calls the engine, never a backend's own functions for its behaviour.

## Working on it

You need the Rust toolchain (stable, MSVC) and Visual Studio Build Tools. Set `VAEXIL_MOD_MANAGER_GAME` to a fake game folder so development runs never touch a real HITMAN install. `scripts/smoke-minimize-restore.ps1` shows how to make one.

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release -p vaexil-mod-manager
./scripts/smoke-minimize-restore.ps1
./scripts/smoke-mouse-clicks.ps1
```

CI runs all of these on every pull request, plus `cargo deny check`. Test clicks with the mouse, not only with UI Automation's Invoke, which skips the mouse path. `cargo deny` bans HTTP and TLS crates and rejects yanked or vulnerable ones. Both CI checks must pass before a pull request can merge.

- Keep each pull request to one change.
- Add tests for behaviour that can be tested. Logic belongs in the crate that owns it (core, a backend, or the engine), where it is unit tested; the window only turns the engine's answers into what it shows.
- For UI changes:
  - check light and dark mode;
  - check keyboard use: Tab, Space/Enter;
  - check that controls have screen-reader labels, and that a long mod list stays reachable, since only rows scrolled into view are exposed to screen readers.
- Update the README or docs when behaviour changes.

## License

By contributing you agree that your work is licensed under [GPL-3.0-only](LICENSE), like the rest of the project.
