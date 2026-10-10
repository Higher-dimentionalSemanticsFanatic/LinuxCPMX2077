---
tags: [cpmx2077, map]
---
# CPMX2077 code map

An [Obsidian](https://obsidian.md) vault describing how the app is built, with
the install logic in most detail. Open **this folder** (`docs/architecture/`)
as a vault: *Open folder as vault* in Obsidian. Mermaid diagrams render in
Obsidian and on GitHub; `[[links]]` and the `.canvas` need Obsidian.

Written for developers. The player-facing description is in
`docs/user-guide.md`.

## Start here

- [[Install flow.canvas|Install flow canvas]]: the whole install pipeline on one board, each box linking to its note.
- [[App structure]]: crates, tabs, commands and which core module does what.
- [[Install pipeline]]: the install path end to end, as one Mermaid chart.
- [[Known gaps]]: archive shapes that install wrongly or are refused today, checked against the code. **Start here when a mod "doesn't work".**

## Install pipeline, step by step

1. [[Entry points]]: every button and flow that ends in an install.
2. [[Archive extraction]]: format by magic bytes, path checks, size caps.
3. [[FOMOD]]: installers with options, plus the app's own *Versions* step.
4. [[Layout detection]]: `install::plan`, deciding where each file goes.
5. [[Deploy and tracking]]: conflicts, backups, atomic copies, the database.
6. [[Versions and replacing]]: when a new archive replaces an installed mod.
7. [[Disable, enable, uninstall, verify]]: taking files back out.
8. [[After install]]: analysis index, dependencies, Linux launch fixes.

## Keeping this current

`CLAUDE.md` requires any change to install logic (`crates/core/src/install.rs`,
`archive.rs`, `fomod.rs`, the install commands in `src-tauri/src/main.rs`,
`reshade.rs` installs) to update the matching note here and the canvas in the
same commit. Notes name functions rather than line numbers so they don't
drift. When a gap in [[Known gaps]] is fixed, move it to its "Fixed" list with
the commit.
