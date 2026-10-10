---
tags: [cpmx2077, install, security]
---
# Archive extraction

Back to [[Home]] · previous: [[Entry points]] · next: [[FOMOD]].

Code: `crates/core/src/archive.rs`, called from `Installer::prepare`.

```mermaid
flowchart TD
  a[archive path] --> fmt{detect_format<br/>magic bytes,<br/>never the extension}
  fmt -- "PK.." --> zip[extract_zip<br/>zip crate]
  fmt -- "7z.." --> sz[extract_7z<br/>sevenz_rust2]
  fmt -- "Rar!" --> rar[extract_external<br/>bsdtar or unrar,<br/>Windows: tar.exe]
  fmt -- other --> bad([UnsupportedArchive])
  zip & sz --> each[per entry:<br/>sanitize_entry_name,<br/>refuse symlink / encrypted,<br/>count entries,<br/>create_new + byte cap]
  rar --> list[list names first,<br/>sanitize each] --> run[run extractor] --> walk[validate_tree:<br/>no symlinks or special files,<br/>byte cap]
  each & walk --> ok([Extracted: files list,<br/>'/'-separated paths])
  each & walk -- any error --> wipe([delete everything<br/>extracted so far])
```

## Rules

- `sanitize_entry_name`: backslashes become `/`; absolute paths, `..`,
  `:` (drive letters, NTFS streams) and NUL bytes are refused.
- `Limits::default()`: 100 000 entries, 32 GiB of real bytes written (header
  sizes are not trusted).
- `create_new` refuses duplicate entry names and pre-existing symlinks;
  `ensure_inside` re-checks the parent after `create_dir_all`.
- Extraction goes to `staging/incoming-<random>/files`. The folder is deleted
  on error, on **Cancel** in the wizard (`discard`), and at startup
  (`cleanup_incoming`).
- Install tokens are only accepted if they start with `incoming-` and contain
  no `/` or `..` (`prepared_dir`).

The resulting file list is what [[FOMOD]] and [[Layout detection]] work on.

Gap: see "RAR on Windows" in [[Known gaps]].
