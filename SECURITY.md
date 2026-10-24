# Security

Vaexil Mod Manager runs with your normal user rights. It never asks for admin rights, and it makes no network connections. The [safety model](docs/safety-model.md) lists every file it reads and writes. In short:

- It never changes Simple Mod Framework's own code or the files SMF ships, and it sends nothing to SMF's authors.
- This version writes nothing at all: not inside HITMAN's folder, not inside SMF's folder, and not even its own settings. Later versions add writes one at a time, and the safety model lists each one before it ships.

## Reporting a problem

Report anything that could harm a user's game, mods or PC privately, through GitHub: **Security > Report a vulnerability** on this repository. Please do not open a public issue for these.

Examples worth reporting:

- the app writing anywhere the safety model says it does not, especially inside HITMAN's or SMF's folder;
- the app changing, replacing or rebuilding any of SMF's own files, such as `Deploy.exe`;
- a crafted `manifest.json`, SMF `config.json`, Steam or Epic file that makes the app run a program or read files outside what the safety model lists.

Include the app version (the bottom of the window shows it), what you did, and what happened.

If the problem is in Simple Mod Framework itself, for example a deploy that fails the same way in SMF's own Mod Manager, report it to [SMF's authors](https://github.com/atampy25/simple-mod-framework) instead.

You should get a reply within a week. Fixes go into the next release, and the report is credited unless you would rather it was not.

## Supported versions

Only the latest release gets fixes.
