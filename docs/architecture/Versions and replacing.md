---
tags: [cpmx2077, install, updates]
---
# Versions and replacing

Back to [[Home]] · previous: [[Deploy and tracking]] · next: [[Disable, enable, uninstall, verify]].

Code: `Installer::finish_auto`, `Installer::previous_version`,
`Installer::finish_replacing`, `base_name`.

`finish_auto` plans the archive first, then asks `previous_version` whether an
installed mod (enabled or not) is an older version of it. Callers that already
know (Downloads **Switch to this**, **Update**, ReShade, radio stations) pass `replaces` and
call `finish_replacing` directly.

## Same mod or not

```mermaid
flowchart TD
  m([each installed mod of this game]) --> repo{"same non-Nexus source<br/>and same source_ref?<br/>(e.g. GitHub repo)"}
  repo -- yes --> yes([another version])
  repo -- no --> shared{"shares at least one<br/>target file?"}
  shared -- no --> no([separate mod])
  shared -- yes --> any{"same Nexus mod id<br/>OR same base_name<br/>OR ≥ half of each side's<br/>files shared?"}
  any -- yes --> yes
  any -- no --> no
```

The mod with the most shared files wins. `base_name` drops numbers, a leading
`v` and punctuation: "ArchiveXL 1.2.3" = "ArchiveXL-v1.3". An optional file
from the same Nexus page that shares no file with the main file stays a
separate mod.

## Replacing

```mermaid
sequenceDiagram
  participant I as Installer
  I->>I: disable(old) if it was enabled (its files leave the game)
  I->>I: finish(new) — conflicts no longer see the old files
  alt success
    I->>I: uninstall(old) (deletes staging/OLD and the db rows)
    I->>I: disable(new) if the old one was disabled
  else failure
    I->>I: enable(old, overwrite = true)
  end
```

The report's `replaced` ("Mod 1.2") shows in the install toast.
