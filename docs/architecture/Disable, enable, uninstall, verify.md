---
tags: [cpmx2077, install, uninstall]
---
# Disable, enable, uninstall, verify

Back to [[Home]] · previous: [[Versions and replacing]] · next: [[After install]].

Code: `Installer::undeploy`, `disable`, `enable`, `uninstall`, `verify`.
Commands: `disable_mod`, `enable_mod`, `uninstall_mod`, `verify_mod`.

## undeploy (shared by disable and uninstall)

```mermaid
flowchart TD
  f([each mod_files row]) --> on{file on disk<br/>inside game dir?}
  on -- no --> skip([nothing to do])
  on -- yes --> same{"current SHA-256 =<br/>recorded SHA-256?"}
  same -- no --> other{"matches another<br/>owner's copy?"}
  other -- yes --> leave1([leave: another mod overrode it])
  other -- no --> kept(["leave: changed after install<br/>(settings) → kept list"])
  same -- yes --> del[delete file]
  del --> next{another enabled owner<br/>with a staged copy?}
  next -- yes --> put[copy theirs back]
  next -- no --> bkp{backup of the<br/>original game file?}
  bkp -- yes --> restore[restore it, delete backup]
  bkp -- no --> prune[remove empty parent folders]
```

## The four actions

| Action | Effect on game folder | Effect on library |
|---|---|---|
| **Disable** | status set to disabled first (so it no longer owns files), then `undeploy` | staged copy kept; changed files saved in `kept_files` with their hash |
| **Enable** | staged copies re-checked against recorded SHA-256 (refuses if missing/changed: "reinstall the mod"); files still in `kept_files` with an unchanged hash are left alone; conflicts checked as on install; then `deploy` | status enabled, `kept_files` cleared |
| **Uninstall** | `undeploy` if enabled | `mods` row and `staging/MOD_ID` deleted; rows cascade. The `downloads` row and its archive are untouched, so `install_download` reinstalls it |
| **Verify** | read-only | reports ok / missing / modified / overridden per file |

Uninstall leaves files that changed after install in the game folder and
then forgets them, because the mod row is deleted. See [[Known gaps]].

## Mod profiles

`profiles::apply` (command `apply_profile`, card in `ui/profiles.js`) runs
**Disable** on every enabled mod not in the profile, then **Enable** (no
overwrite) on every disabled mod in it. Failures are collected per mod and
the rest go ahead. Profiles live in `profiles` / `profile_mods`, keyed by
game version string and `modpacks::mod_key` plus archive name.
