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
- Each mod framework is a module behind the `Framework` interface in `crates/core/src/framework.rs`. SMF is the only one so far, and the window must not depend on SMF specifics.

## Working on it

You need the Rust toolchain (stable, MSVC) and Visual Studio Build Tools. Set `VAEXIL_MOD_MANAGER_GAME` to a fake game folder so development runs never touch a real HITMAN install. `scripts/smoke-minimize-restore.ps1` shows how to make one.

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release -p vaexil-mod-manager
./scripts/smoke-minimize-restore.ps1
```

CI runs all of these on every pull request except the smoke test, plus `cargo deny check`. `cargo deny` bans HTTP and TLS crates and rejects yanked or vulnerable ones. Both CI checks must pass before a pull request can merge.

- Keep each pull request to one change.
- Add tests for behaviour that can be tested. Platform-neutral logic belongs in `crates/core`, where it is unit tested.
- For UI changes:
  - check light and dark mode;
  - check keyboard use: Tab, Space/Enter;
  - check that controls have screen-reader labels, and that a long mod list stays reachable, since only rows scrolled into view are exposed to screen readers.
- Update the README or docs when behaviour changes.

## License

By contributing you agree that your work is licensed under [GPL-3.0-only](LICENSE), like the rest of the project.
