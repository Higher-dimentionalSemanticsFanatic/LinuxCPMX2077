---
tags: [cpmx2077, install, database]
---
# Deploy and tracking

Back to [[Home]] · previous: [[Layout detection]] · next: [[Versions and replacing]].

Code: `Installer::conflicts`, `Installer::finish`, `Installer::deploy`,
`resolve_ci`, `atomic_copy` in `crates/core/src/install.rs`.

```mermaid
flowchart TD
  plan([Plan: staged → target pairs]) --> c{conflicts:<br/>db.owners_of target<br/>for each file}
  c -- "owned by an enabled mod<br/>and overwrite off" --> err(["Error::Conflict 'path (from Mod)'<br/>UI: tick Allow overwriting other mods"])
  c -- "free, or overwrite on" --> row[db.insert_mod]
  row --> mv["rename incoming-*/files<br/>→ staging/MOD_ID"]
  mv --> loop
  subgraph loop[deploy, per planned file]
    rc["resolve_ci: match existing<br/>folder names ignoring case"] --> chk{"inside game dir?<br/>target not a symlink?"}
    chk -- no --> unsafe([UnsafeArchive → rollback])
    chk -- yes --> exists{target file exists<br/>and no mod owns it?}
    exists -- yes --> bk["copy original to<br/>backups/GAME_ID/path,<br/>db.add_backup"]
    exists -- no --> cp
    bk --> cp["atomic_copy: temp file<br/>in target folder + rename"]
    cp --> rec["db.add_mod_file:<br/>rel_path, staged_path,<br/>sha256, size"]
  end
  loop -- any error --> rb([uninstall the half-installed mod])
  loop -- ok --> idx[analysis::index_mod<br/>failure only logged]
```

## Ownership rules

- A path's owners are the **enabled** mods with a `mod_files` row for it
  (`owners_of` matches `lower(rel_path)`). Disabled mods don't block.
- With **Allow overwriting other mods** ticked, the new mod's copy goes on top.
  The overridden mod keeps its `mod_files` row, so when the new mod is removed
  the previous owner's staged copy is put back ([[Disable, enable, uninstall, verify]]).
- Only a file **no mod owns** is backed up: that is treated as an original
  game file (or something installed by hand). One backup per game + path.
- `mod_files` has primary key `(mod_id, rel_path)` and is written with
  `INSERT OR REPLACE`: two staged files mapped to the same target inside one
  mod silently become one (see [[Known gaps]]).

## What is kept where

| Thing | Location | Used by |
|---|---|---|
| Unpacked mod | `staging/MOD_ID/` (staged paths as in the archive) | enable, restoring an overridden file, analysis index, File map |
| Original game file | `backups/GAME_ID/<target>` | uninstall / disable |
| Archive hashes | `mods.archive_sha256`, `archive_md5` | update checks, MCP |
| Game build at install | `mods` row (via `game_versions`) | game version ribbon |

## Not tracked

Files the game or a framework writes later (`r6/cache/modded`, CET
`db.sqlite3` / settings, `user.ini` edits, RED4ext logs) never enter
`mod_files`, so uninstall never removes them. See [[Known gaps]].
