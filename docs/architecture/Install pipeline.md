---
tags: [cpmx2077, install]
---
# Install pipeline

Back to [[Home]]. Same flow as a board: [[Install flow.canvas]].

Every install, whatever started it ([[Entry points]]), runs through
`main.rs::begin_install` and then `core::install::Installer`:

```mermaid
flowchart TD
  start([Archive on disk<br/>zip / 7z / rar]) --> prep

  subgraph prep["Installer::prepare"]
    hash[Hash archive<br/>SHA-256 + MD5] --> ext[archive::extract<br/>to staging/incoming-*/files]
    ext --> fm{fomod/ModuleConfig.xml<br/>anywhere? shallowest wins}
    fm -- yes --> parse[fomod::parse<br/>+ default_selections]
    fm -- no --> var{variant_choice:<br/>loose files named like<br/>versions of one thing?}
    var -- yes --> vstep[App-made 'Versions' step<br/>all ticked by default]
    var -- no --> noq[No questions]
  end

  parse --> wiz
  vstep --> wiz
  wiz[[NeedsChoices:<br/>UI shows wizard,<br/>install waits in AppState.pending]] --> fin
  noq --> fin

  subgraph fin["finish_auto / finish_replacing"]
    planp[plan_prepared] --> which{FOMOD?}
    which -- real FOMOD --> res[fomod resolve:<br/>source → destination pairs]
    which -- Versions step --> pl2[install::plan, then drop<br/>unticked variants]
    which -- none --> pl[install::plan<br/>layout detection]
    res & pl2 & pl --> prev{previous_version?<br/>same repo / page / name /<br/>mostly same files}
    prev -- yes --> rep[disable old version]
    prev -- no --> conf
    rep --> conf{files owned by<br/>another enabled mod?}
    conf -- "yes, overwrite off" --> cerr([Error::Conflict<br/>prepared files kept for retry])
    conf -- "no / overwrite on" --> ins[insert mods row<br/>move files to staging/ID]
    ins --> dep[deploy: per file<br/>path + symlink checks,<br/>backup original game file,<br/>atomic copy, record SHA-256]
    dep -- error --> rb([rollback: uninstall new mod,<br/>re-enable old version])
    dep -- ok --> idx[analysis::index_mod]
    idx --> old[uninstall old version<br/>if replacing]
  end

  old --> report([InstallReport → toast:<br/>layout, files, overrides,<br/>backups, Skipped list])
```

## Where each box is explained

| Step | Note | Code |
|---|---|---|
| Hash + extract | [[Archive extraction]] | `Installer::prepare`, `archive::extract` |
| FOMOD / Versions | [[FOMOD]] | `fomod::find_config`, `install::variant_choice` |
| Where files go | [[Layout detection]] | `install::plan` |
| Replace older version | [[Versions and replacing]] | `previous_version`, `finish_replacing` |
| Conflicts, deploy, record | [[Deploy and tracking]] | `conflicts`, `finish`, `deploy` |
| After the copy | [[After install]] | `analysis::index_mod` |

## What the user sees

- The toast built in `app.js` (after `install_archive` / `finish_install`):
  `Installed <name>: N files (<layout>)`, replaced version, overridden mods,
  backed-up game files, and the **first five** skipped files.
- Every step also lands in the debug terminal through `core::activity`
  (Hashed, Extracting, Copied, Saved the original, Installed … layout).
- The layout name (`game-root`, `redmod`, `cet-mod`, `radioext-station`, `reshade`, `loose`,
  `engine-config`, `fomod`) is the quickest clue when a mod lands in the wrong
  place. See [[Known gaps]].
