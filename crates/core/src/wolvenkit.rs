//! WolvenKit, the Cyberpunk 2077 modding toolkit (github.com/WolvenKit/WolvenKit),
//! downloaded from its GitHub releases into the app's data folder, and its
//! command-line build (WolvenKit.CLI) run to read archives.
//!
//! Release assets (from WolvenKit's `release.yml`): `WolvenKit.ConsoleLinux-<v>.zip`
//! (CLI, linux-x64), `WolvenKit.Console-<v>.zip` (CLI, win-x64), both needing
//! the .NET runtime named in `WolvenKit.CLI.runtimeconfig.json`, and
//! `WolvenKit-<v>.zip`, the editor: a self-contained Windows (WPF) program,
//! so it is offered on Windows only.
//!
//! The CLI is started as `dotnet WolvenKit.CLI.dll`, which works the same with
//! either build and whatever `dotnet` the system has.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::activity::{self, Kind};
use crate::archive::{self, Limits};
use crate::sources::github::{Asset, Release};
use crate::{Error, Result, desktop, paths};

pub const OWNER: &str = "WolvenKit";
pub const REPO: &str = "WolvenKit";
pub const CLI_DLL: &str = "WolvenKit.CLI.dll";
pub const CLI_CONFIG: &str = "WolvenKit.CLI.runtimeconfig.json";
pub const EDITOR_EXE: &str = "WolvenKit.exe";
/// Written next to an installed part: what was installed from where.
const MANIFEST: &str = "cpmx-install.json";
/// How long one CLI call may take.
const RUN_TIMEOUT: Duration = Duration::from_secs(180);
/// Extraction limits: the editor is about 1 GB unpacked.
const LIMITS: Limits = Limits { max_entries: 50_000, max_total_bytes: 4 << 30 };

/// Where WolvenKit is kept.
pub fn root() -> Result<PathBuf> {
    Ok(paths::data_dir()?.join("tools").join("wolvenkit"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Part {
    /// WolvenKit.CLI, the command-line build.
    Cli,
    /// The WolvenKit editor (Windows only).
    Editor,
}

impl Part {
    pub const ALL: [Part; 2] = [Part::Cli, Part::Editor];

    /// The asset name before the version.
    fn asset_prefix(self) -> &'static str {
        match self {
            Part::Cli if cfg!(windows) => "WolvenKit.Console-",
            Part::Cli => "WolvenKit.ConsoleLinux-",
            Part::Editor => "WolvenKit-",
        }
    }

    fn folder(self) -> &'static str {
        match self {
            Part::Cli => "cli",
            Part::Editor => "editor",
        }
    }

    /// The file that marks the part's main folder in its archive.
    fn marker(self) -> &'static str {
        match self {
            Part::Cli => CLI_DLL,
            Part::Editor => EDITOR_EXE,
        }
    }

    /// Runs on this system.
    pub fn available(self) -> bool {
        self == Part::Cli || cfg!(windows)
    }

    pub fn label(self) -> &'static str {
        match self {
            Part::Cli => "WolvenKit CLI",
            Part::Editor => "WolvenKit editor",
        }
    }
}

/// The release asset with `part` for this system.
pub fn pick_asset(release: &Release, part: Part) -> Option<&Asset> {
    let prefix = part.asset_prefix().to_ascii_lowercase();
    release.assets.iter().find(|a| {
        let n = a.name.to_ascii_lowercase();
        n.starts_with(&prefix) && n.ends_with(".zip")
    })
}

/// The newest stable release that has `part`.
pub fn latest_stable(releases: &[Release], part: Part) -> Option<&Release> {
    releases.iter().find(|r| !r.prerelease && pick_asset(r, part).is_some())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Installed {
    pub part: Part,
    /// Release tag.
    pub version: String,
    pub asset: String,
    pub sha256: String,
    /// The download matched the SHA-256 digest GitHub published.
    pub verified: bool,
    /// Unix time.
    pub installed: i64,
    pub dir: String,
}

pub fn part_dir(root: &Path, part: Part) -> PathBuf {
    root.join(part.folder())
}

/// What is installed of `part`, if its files are there.
pub fn installed(root: &Path, part: Part) -> Option<Installed> {
    let dir = part_dir(root, part);
    if !dir.join(part.marker()).is_file() {
        return None;
    }
    let m: Installed = serde_json::from_slice(&std::fs::read(dir.join(MANIFEST)).ok()?).ok()?;
    Some(Installed { dir: dir.to_string_lossy().into_owned(), ..m })
}

/// Unpack a downloaded release archive of `part` into `root`, replacing what
/// was there. The archive goes through the same checks as mod archives.
pub fn install(root: &Path, part: Part, zip: &Path, version: &str, asset: &str, sha256: &str, verified: bool) -> Result<Installed> {
    std::fs::create_dir_all(root)?;
    let tmp = tempfile::Builder::new().prefix(".incoming-").tempdir_in(root)?;
    let ex = archive::extract(zip, tmp.path(), LIMITS)?;
    let marker = ex
        .files
        .iter()
        .filter(|f| f.rsplit('/').next().is_some_and(|n| n.eq_ignore_ascii_case(part.marker())))
        .min_by_key(|f| f.matches('/').count())
        .ok_or_else(|| Error::Other(format!("{asset} has no {}", part.marker())))?;
    let base = match marker.rsplit_once('/') {
        Some((d, _)) => tmp.path().join(d),
        None => tmp.path().to_path_buf(),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // The Linux build's own launcher; `dotnet <dll>` doesn't need it.
        let host = base.join("WolvenKit.CLI");
        if host.is_file() {
            std::fs::set_permissions(&host, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    let m = Installed {
        part,
        version: version.to_string(),
        asset: asset.to_string(),
        sha256: sha256.to_string(),
        verified,
        installed: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0),
        dir: String::new(),
    };
    std::fs::write(base.join(MANIFEST), serde_json::to_vec_pretty(&m)?)?;
    let dest = part_dir(root, part);
    let old = root.join(format!(".old-{}", part.folder()));
    let _ = std::fs::remove_dir_all(&old);
    if dest.exists() {
        std::fs::rename(&dest, &old)?;
    }
    if let Err(e) = std::fs::rename(&base, &dest) {
        if old.exists() {
            let _ = std::fs::rename(&old, &dest);
        }
        return Err(e.into());
    }
    let _ = std::fs::remove_dir_all(&old);
    activity::record_path(Kind::Info, format!("Installed {} {version}", part.label()), &dest);
    installed(root, part).ok_or_else(|| Error::Other(format!("{} not found after install", part.marker())))
}

pub fn remove(root: &Path, part: Part) -> Result<()> {
    let dir = part_dir(root, part);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
        activity::record_path(Kind::Info, format!("Removed {}", part.label()), &dir);
    }
    Ok(())
}

// ---- .NET runtime ---------------------------------------------------------------

/// The shared framework the CLI needs, from its runtimeconfig.json.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Framework {
    pub name: String,
    pub version: String,
}

pub fn required_framework(config: &[u8]) -> Option<Framework> {
    let v: Value = serde_json::from_slice(config).ok()?;
    let opts = v.get("runtimeOptions")?;
    let fw = opts.get("framework").or_else(|| opts.get("frameworks")?.as_array()?.first())?;
    Some(Framework { name: fw.get("name")?.as_str()?.to_string(), version: fw.get("version")?.as_str()?.to_string() })
}

/// `dotnet --list-runtimes` lines: `Microsoft.NETCore.App 10.0.12 [/usr/lib/dotnet/shared/…]`.
pub fn parse_runtimes(out: &str) -> Vec<Framework> {
    out.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some(Framework { name: it.next()?.to_string(), version: it.next()?.to_string() })
        })
        .collect()
}

fn version_parts(v: &str) -> Vec<u64> {
    v.split(['-', '+']).next().unwrap_or(v).split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// `have` runs an app built for `need` under .NET's default roll-forward
/// (same framework and major version, same or newer minor/patch).
pub fn satisfies(have: &Framework, need: &Framework) -> bool {
    let (h, n) = (version_parts(&have.version), version_parts(&need.version));
    have.name.eq_ignore_ascii_case(&need.name) && h.first() == n.first() && h >= n
}

fn exe(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

/// The `dotnet` host: `DOTNET_ROOT`, `PATH`, then the usual install folders.
pub fn find_dotnet() -> Option<PathBuf> {
    let name = exe("dotnet");
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(r) = std::env::var_os("DOTNET_ROOT") {
        dirs.push(PathBuf::from(r));
    }
    if let Some(p) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&p));
    }
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".dotnet"));
    }
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramW6432"] {
            if let Some(p) = std::env::var_os(var) {
                dirs.push(PathBuf::from(p).join("dotnet"));
            }
        }
    } else {
        for d in ["/usr/lib/dotnet", "/usr/share/dotnet", "/usr/lib64/dotnet", "/opt/dotnet", "/usr/local/share/dotnet", "/snap/dotnet/current"] {
            dirs.push(PathBuf::from(d));
        }
    }
    dirs.into_iter().map(|d| d.join(&name)).find(|p| p.is_file())
}

fn no_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Run `cmd`, killing it after `timeout`.
fn run(mut cmd: Command, timeout: Duration) -> Result<Output> {
    no_window(&mut cmd);
    cmd.env("DOTNET_CLI_TELEMETRY_OPTOUT", "1").env("DOTNET_NOLOGO", "1").stdin(Stdio::null());
    let out_file = tempfile::tempfile()?;
    let err_file = tempfile::tempfile()?;
    let mut child = cmd.stdout(out_file.try_clone()?).stderr(err_file.try_clone()?).spawn()?;
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Other(format!("stopped after {} s without finishing", timeout.as_secs())));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let read = |mut f: std::fs::File| -> Result<Vec<u8>> {
        use std::io::{Read, Seek};
        f.rewind()?;
        let mut v = Vec::new();
        f.take(16 << 20).read_to_end(&mut v)?;
        Ok(v)
    };
    Ok(Output { status, stdout: read(out_file)?, stderr: read(err_file)? })
}

#[derive(Debug, Clone, Serialize)]
pub struct Runtime {
    /// What the installed CLI asks for.
    pub required: Option<Framework>,
    pub dotnet: Option<String>,
    /// Shared frameworks `dotnet --list-runtimes` lists.
    pub found: Vec<Framework>,
    pub ok: bool,
}

pub fn runtime(cli_dir: Option<&Path>) -> Runtime {
    let required = cli_dir.and_then(|d| std::fs::read(d.join(CLI_CONFIG)).ok()).and_then(|b| required_framework(&b));
    let dotnet = find_dotnet();
    let found = dotnet
        .as_ref()
        .and_then(|d| {
            let mut c = desktop::user_command(&d.to_string_lossy());
            c.arg("--list-runtimes");
            run(c, Duration::from_secs(20)).ok()
        })
        .map(|o| parse_runtimes(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();
    let ok = dotnet.is_some() && required.as_ref().is_some_and(|need| found.iter().any(|f| satisfies(f, need)));
    Runtime { required, dotnet: dotnet.map(|d| d.to_string_lossy().into_owned()), found, ok }
}

// ---- running the CLI --------------------------------------------------------------

/// The installed WolvenKit CLI with a `dotnet` that can run it.
#[derive(Debug, Clone)]
pub struct Cli {
    pub dir: PathBuf,
    pub dotnet: PathBuf,
    pub version: String,
}

impl Cli {
    /// The CLI in `root`, or why it can't run.
    pub fn find(root: &Path) -> Result<Cli> {
        let inst = installed(root, Part::Cli).ok_or_else(|| Error::Other("WolvenKit CLI is not installed (Netrunner › WolvenKit)".into()))?;
        let dir = PathBuf::from(&inst.dir);
        let rt = runtime(Some(&dir));
        if !rt.ok {
            let need = rt.required.map(|f| format!("{} {}", f.name, f.version)).unwrap_or_else(|| ".NET".into());
            return Err(Error::Other(format!("WolvenKit CLI needs the .NET runtime ({need}); not found")));
        }
        let dotnet = PathBuf::from(rt.dotnet.unwrap_or_default());
        Ok(Cli { dir, dotnet, version: inst.version })
    }

    fn command(&self, args: &[&std::ffi::OsStr]) -> Command {
        let mut c = desktop::user_command(&self.dotnet.to_string_lossy());
        c.arg(self.dir.join(CLI_DLL)).args(args);
        c
    }

    /// Run the CLI; an error carries its last output lines.
    pub fn run(&self, args: &[&std::ffi::OsStr]) -> Result<String> {
        let out = run(self.command(args), RUN_TIMEOUT)?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let tail: Vec<&str> = text.lines().chain(err.lines()).filter(|l| !l.trim().is_empty()).collect();
            let tail = tail[tail.len().saturating_sub(4)..].join("\n");
            return Err(Error::Other(format!("WolvenKit CLI failed ({}): {tail}", out.status)));
        }
        Ok(text)
    }

    /// The version the CLI reports.
    pub fn reported_version(&self) -> Result<String> {
        Ok(self.run(&["--version".as_ref()])?.trim().to_string())
    }

    /// The files in an archive, by depot path where WolvenKit knows the name
    /// (its hash list or the archive's own list), else by hash.
    pub fn list_archive(&self, archive: &Path) -> Result<Vec<String>> {
        let out = self.run(&["archive".as_ref(), archive.as_os_str(), "--list".as_ref()])?;
        Ok(out.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('[')).map(String::from).collect())
    }
}

/// One file of an archive as WolvenKit read it.
#[derive(Debug, Clone, Serialize)]
pub struct FileCheck {
    pub path: String,
    /// Extracted bytes equal the expected bytes.
    pub same_bytes: bool,
    /// Root class WolvenKit read (`CBitmapTexture`, `inkTextureAtlas`, …).
    pub class: Option<String>,
    /// Size for textures, parts for atlases.
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchiveCheck {
    pub tool: String,
    pub files: Vec<FileCheck>,
    pub ok: bool,
}

fn class_detail(json: &Value) -> (Option<String>, Option<String>) {
    let root = &json["Data"]["RootChunk"];
    let class = root["$type"].as_str().map(String::from);
    let detail = match class.as_deref() {
        Some("CBitmapTexture") => Some(format!("{}×{}", root["width"], root["height"])),
        Some("inkTextureAtlas") => {
            let mut parts: Vec<String> = root["slots"]["Elements"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|s| s["parts"].as_array().into_iter().flatten())
                .filter_map(|p| p["partName"]["$value"].as_str().map(String::from))
                .collect();
            parts.dedup();
            Some(format!("part {}", parts.join(", ")))
        }
        _ => None,
    };
    (class, detail)
}

/// Extract `archive` with WolvenKit, compare each expected file byte for
/// byte, and read it back to JSON to name its class.
pub fn check_archive(cli: &Cli, archive: &Path, expected: &[(String, Vec<u8>)]) -> Result<ArchiveCheck> {
    let tmp = tempfile::tempdir()?;
    let (out, json) = (tmp.path().join("files"), tmp.path().join("json"));
    std::fs::create_dir_all(&json)?;
    cli.run(&["extract".as_ref(), archive.as_os_str(), "-o".as_ref(), out.as_os_str()])?;
    cli.run(&["convert".as_ref(), "serialize".as_ref(), out.as_os_str(), "-o".as_ref(), json.as_os_str()])?;
    let mut files = Vec::new();
    for (path, bytes) in expected {
        let rel: PathBuf = path.split(['\\', '/']).collect();
        let got = std::fs::read(out.join(&rel)).ok();
        let name = rel.file_name().map(|n| format!("{}.json", n.to_string_lossy())).unwrap_or_default();
        let parsed: Option<Value> = std::fs::read(json.join(&name)).ok().and_then(|b| serde_json::from_slice(&b).ok());
        let (class, detail) = parsed.as_ref().map(class_detail).unwrap_or((None, None));
        files.push(FileCheck { path: path.clone(), same_bytes: got.as_deref() == Some(bytes.as_slice()), class, detail });
    }
    let ok = !files.is_empty() && files.iter().all(|f| f.same_bytes && f.class.is_some());
    activity::record(if ok { Kind::Info } else { Kind::Error }, format!("WolvenKit {} read {}: {}", cli.version, archive.display(), if ok { "ok" } else { "mismatch" }));
    Ok(ArchiveCheck { tool: format!("WolvenKit CLI {}", cli.version), files, ok })
}

/// Start the WolvenKit editor (Windows).
pub fn open_editor(root: &Path) -> Result<()> {
    let inst = installed(root, Part::Editor).ok_or_else(|| Error::Other("WolvenKit editor is not installed".into()))?;
    if !Part::Editor.available() {
        return Err(Error::Other("the WolvenKit editor is a Windows program".into()));
    }
    let exe = Path::new(&inst.dir).join(EDITOR_EXE);
    let mut cmd = desktop::user_command(&exe.to_string_lossy());
    cmd.current_dir(&inst.dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.spawn()?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub cli: Option<Installed>,
    pub editor: Option<Installed>,
    /// The editor runs on this system.
    pub editor_available: bool,
    pub runtime: Runtime,
    pub root: String,
}

pub fn status(root: &Path) -> Status {
    let cli = installed(root, Part::Cli);
    let runtime = runtime(cli.as_ref().map(|c| Path::new(&c.dir)));
    Status { editor: installed(root, Part::Editor), editor_available: Part::Editor.available(), runtime, cli, root: root.to_string_lossy().into_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> Asset {
        Asset { id: 1, name: name.into(), size: 1, digest: None, download_url: String::new(), download_count: None, installable: true }
    }

    fn release(tag: &str, pre: bool, names: &[&str]) -> Release {
        Release {
            id: 1,
            tag: tag.into(),
            name: None,
            published: None,
            prerelease: pre,
            notes: String::new(),
            url: String::new(),
            assets: names.iter().map(|n| asset(n)).collect(),
        }
    }

    #[test]
    fn picks_the_asset_for_this_system() {
        let r = release("9.1.0", false, &[
            "WolvenKit-9.1.0.zip",
            "WolvenKit.Console-9.1.0.zip",
            "WolvenKit.ConsoleLinux-9.1.0.zip",
            "WolvenKit.Unpacker-9.1.0.zip",
            "WolvenKitSetup-9.1.0.exe",
        ]);
        let cli = pick_asset(&r, Part::Cli).unwrap().name.clone();
        assert_eq!(cli, if cfg!(windows) { "WolvenKit.Console-9.1.0.zip" } else { "WolvenKit.ConsoleLinux-9.1.0.zip" });
        assert_eq!(pick_asset(&r, Part::Editor).unwrap().name, "WolvenKit-9.1.0.zip");
        let rels = [release("9.2.0-nightly", true, &["WolvenKit.ConsoleLinux-9.2.0.zip"]), r.clone()];
        assert_eq!(latest_stable(&rels, Part::Cli).unwrap().tag, "9.1.0");
        assert!(latest_stable(&[release("1", false, &["other.zip"])], Part::Cli).is_none());
    }

    #[test]
    fn runtime_checks() {
        let cfg = br#"{"runtimeOptions":{"tfm":"net10.0","framework":{"name":"Microsoft.NETCore.App","version":"10.0.0"}}}"#;
        let need = required_framework(cfg).unwrap();
        assert_eq!(need, Framework { name: "Microsoft.NETCore.App".into(), version: "10.0.0".into() });
        let found = parse_runtimes(
            "Microsoft.AspNetCore.App 10.0.12 [/usr/lib/dotnet/shared/Microsoft.AspNetCore.App]\n\
             Microsoft.NETCore.App 8.0.20 [/usr/lib/dotnet/shared/Microsoft.NETCore.App]\n\
             Microsoft.NETCore.App 10.0.12 [/usr/lib/dotnet/shared/Microsoft.NETCore.App]\n",
        );
        assert_eq!(found.len(), 3);
        assert!(!satisfies(&found[0], &need));
        assert!(!satisfies(&found[1], &need));
        assert!(satisfies(&found[2], &need));
        let fw = |v: &str| Framework { name: "Microsoft.NETCore.App".into(), version: v.into() };
        assert!(!satisfies(&fw("11.0.0"), &need));
        assert!(!satisfies(&fw("10.0.0-rc.1"), &fw("10.0.1")));
    }

    #[test]
    fn installs_and_replaces_a_release_archive() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("wk");
        let make = |v: &str| {
            let p = dir.path().join(format!("{v}.zip"));
            let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            for (n, body) in [("WolvenKit.CLI.dll", v), ("WolvenKit.CLI", "host"), ("lib/x.dll", "x")] {
                z.start_file(n, o).unwrap();
                std::io::Write::write_all(&mut z, body.as_bytes()).unwrap();
            }
            z.finish().unwrap();
            p
        };
        assert!(installed(&root, Part::Cli).is_none());
        let a = install(&root, Part::Cli, &make("9.0.0"), "9.0.0", "a.zip", "aa", true).unwrap();
        assert_eq!(a.version, "9.0.0");
        let b = install(&root, Part::Cli, &make("9.1.0"), "9.1.0", "b.zip", "bb", true).unwrap();
        assert_eq!(b.version, "9.1.0");
        assert_eq!(std::fs::read_to_string(part_dir(&root, Part::Cli).join(CLI_DLL)).unwrap(), "9.1.0");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(part_dir(&root, Part::Cli).join("WolvenKit.CLI")).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111);
        }
        // Only the part's folder is left (no temporary or old copies).
        let names: Vec<String> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["cli"]);
        // An archive without the CLI is refused and the installed copy stays.
        let p = dir.path().join("bad.zip");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
        z.start_file("readme.txt", zip::write::SimpleFileOptions::default()).unwrap();
        z.finish().unwrap();
        assert!(install(&root, Part::Cli, &p, "x", "bad.zip", "", false).is_err());
        assert_eq!(installed(&root, Part::Cli).unwrap().version, "9.1.0");
        remove(&root, Part::Cli).unwrap();
        assert!(installed(&root, Part::Cli).is_none());
    }

    #[test]
    fn reads_classes_from_wolvenkit_json() {
        let x: Value = serde_json::from_str(r#"{"Data":{"RootChunk":{"$type":"CBitmapTexture","width":256,"height":256}}}"#).unwrap();
        assert_eq!(class_detail(&x), (Some("CBitmapTexture".into()), Some("256×256".into())));
        let a: Value = serde_json::from_str(
            r#"{"Data":{"RootChunk":{"$type":"inkTextureAtlas","slots":{"Elements":[
              {"parts":[{"partName":{"$value":"icon"}}]},{"parts":[{"partName":{"$value":"icon"}}]},{"parts":[]}]}}}}"#,
        )
        .unwrap();
        assert_eq!(class_detail(&a), (Some("inkTextureAtlas".into()), Some("part icon".into())));
    }

    /// Runs the real WolvenKit CLI when `CPMX_WOLVENKIT_CLI` names a folder
    /// with WolvenKit.CLI.dll (for example the NuGet package's
    /// `tools/net10.0/any`) and dotnet is installed.
    #[test]
    fn real_cli_reads_our_icon_archive() {
        let Some(dir) = std::env::var_os("CPMX_WOLVENKIT_CLI") else {
            return;
        };
        let Some(dotnet) = find_dotnet() else {
            return;
        };
        let cli = Cli { dir: PathBuf::from(dir), dotnet, version: "test".into() };
        eprintln!("WolvenKit reports {}", cli.reported_version().unwrap());
        let img = crate::inkicon::Rgba { width: 300, height: 200, pixels: [200, 40, 40, 255].repeat(300 * 200) };
        let icon = crate::inkicon::build(&img, "cpmx2077\\radioext\\test_1234").unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("icon.archive");
        std::fs::write(&p, &icon.archive).unwrap();
        let listed = cli.list_archive(&p).unwrap();
        assert!(listed.contains(&icon.atlas_path), "{listed:?}");
        let expected = vec![(icon.xbm_path.clone(), icon.xbm.bytes.clone()), (icon.atlas_path.clone(), icon.inkatlas.bytes.clone())];
        let check = check_archive(&cli, &p, &expected).unwrap();
        eprintln!("{check:?}");
        assert!(check.ok);
        assert_eq!(check.files[0].detail.as_deref(), Some("256×256"));
        assert_eq!(check.files[1].detail.as_deref(), Some("part icon"));
    }
}
