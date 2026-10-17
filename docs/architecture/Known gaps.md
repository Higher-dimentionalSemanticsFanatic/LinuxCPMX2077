---
tags: [cpmx2077, install, gaps]
---
# Known gaps

Back to [[Home]]. Rules referred to here are in [[Layout detection]].

Archive shapes the install logic handles wrongly today. Each "verified" entry
was run through `install::plan` on 2026-10-10 (commit `2813e2a`); the input
file list and the result are shown so the case can become a test. Fix a gap →
add the case to `install::tests::plans_common_layouts`, move the entry to
**Fixed** with the commit.

## How to trace a mod that doesn't work

1. Read the install toast: the layout name in brackets and the **Skipped**
   list. Wrong layout or a mod file under Skipped → a gap below.
2. Terminal (Settings → Open terminal, or the sidebar): every
   `Copied … to <path>` line shows where each file went.
3. Netrunner → File map → By mod: the installed tree, with missing files
   marked.
4. Installed fine but no effect in game → [[After install]] (launch options,
   frameworks, logs, startup timeline).
5. Add the mod and the step where it went wrong to "Reported mods" below.

## Open: install places or skips files wrongly

| # | Archive shape | Example input | What happens | Verified |
|---|---|---|---|---|
| 1 | **Several wrapper folders** with game-root content (main + optional, or variants as folders) | `Main/archive/pc/mod/a.archive`, `Optional/r6/scripts/b.reds` | Only the shallowest wrapper installs; the other is **skipped** with no question | yes |
| 2 | Same, different depths | `A/archive/pc/mod/a.archive`, `B/C/archive/pc/mod/c.archive` | `B/C/…` skipped | yes |
| 3 | Game-root folders **plus loose files** | `archive/pc/mod/a.archive`, `extra.reds` | `extra.reds` skipped | yes |
| 4 | Helper folder named `tools/` next to a loose mod | `a.archive`, `tools/helper.exe` | Treated as game-root: `helper.exe` goes to the game's `tools/`, **the mod's `.archive` is skipped** | yes |
| 5 | CET mod **plus** loose archive/script | `MyCet/init.lua`, `x.archive` | `x.archive` skipped | yes |
| 6 | REDmod folder plus a file outside it | `X/info.json`, `X/archives/a.archive`, `patch.archive` | `patch.archive` skipped | yes |
| 7 | Loose files with the **same name in different folders** | `Mod/a/main.reds`, `Mod/b/main.reds` | Both map to `r6/scripts/<mod>/main.reds`; the second overwrites the first and only one row is recorded | yes |
| 8 | RED4ext plugin folder **without** `red4ext/plugins/` around it | `MyPlugin/MyPlugin.dll`, `MyPlugin/config.toml` | Refused: "unrecognised archive layout" | yes |
| 9 | CET files without `init.lua` at their level (a CET addon or module only) | `MyCet/main.lua` | Refused: "unrecognised archive layout" | yes |
| 10 | Loose ASI plugin | `plugin.asi` | Refused: "unrecognised archive layout" | yes |
| 11 | Skipped list in the toast | any archive with > 5 skipped files | Toast shows the first 5 only; the rest are visible only in the terminal | from `app.js` |

Most of rows 1–6 come from one design choice: `plan` picks **one** rule for
the whole archive and skips whatever that rule doesn't place. A fix would plan
per top-level folder (or per file) and ask in a *Versions*-style step when
wrappers look like alternatives rather than parts.

## Open: platform

| # | Gap | Where | Verified |
|---|---|---|---|
| 12 | **RAR on Windows**: `validate_tree` builds the file list with `to_string_lossy()` and does not turn `\` into `/` (the zip/7z writer does). Every path from a RAR would then be one name with backslashes, so `plan` sees no folders and refuses or flattens the mod. | `archive::validate_tree` | from code, not run on Windows |

## Open: tracking and uninstall

| # | Gap | Effect |
|---|---|---|
| 13 | Files a mod changed after install (its settings) are left in the game folder on **uninstall**, and the mod row is deleted, so nothing tracks them any more. | Leftover configs; a later install of the same mod sees them as "original game files" and backs them up |
| 14 | Runtime files (`r6/cache/modded`, CET `db.sqlite3`/settings, `user.ini` written by tools) are never recorded. | Not removed on uninstall; not shown in File map |
| 15 | FOMOD file conditions only report *Active* or *Missing*; *Inactive* is never returned. | An installer that checks for a disabled plugin file takes the *Missing* branch |

## Reported mods

Mods players reported as not installing or not working, traced through this
map. Add: mod name + Nexus link, the toast's layout, and which gap (or new
row) explains it.

- LUT Switcher 3 ([Nexus 16310](https://www.nexusmods.com/cyberpunk2077/mods/16310)), 2026-10-17: installs fine (each LUT pack has its own `archive/pc/mod/LUTSwitcher-Addon-*.archive` and `r6/tweaks/LUTSwitcher/*.yaml`), but every pack showed in Downloads as a version of the mod and installed under the mod's name. Fixed, see below.

## Fixed

- Separate files of one Nexus page (add-on packs, optional files) were grouped as versions of one mod in Downloads, named by the mod, and offered **Switch to this** over each other. Now `downloads::file_line` (update chain from `files_with_updates`) splits them, they are named by file title, and `game_versions::compare` pairs them by name.
