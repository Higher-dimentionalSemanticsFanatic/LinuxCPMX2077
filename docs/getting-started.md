# Getting started with CPMX2077

This walks you from a fresh download to a modded Cyberpunk 2077 that starts
cleanly. It takes about fifteen minutes, most of it waiting for downloads.
For what every button does, see the [user guide](user-guide.md).

## Before you start

- Cyberpunk 2077 installed through **Steam** (Proton) or **Heroic** (GOG), and
  started at least once, so its Proton/Wine prefix exists.
- A **Nexus Mods account** (free is fine). Premium only saves clicks.
- For the Visual C++ runtime fix: `protontricks` (recommended) or
  `winetricks`, from your distribution's packages or Flatpak.

## 1. Run the app

1. Download `CPMX2077_<version>_amd64.AppImage` and `SHA256SUMS` from
   [Releases](https://github.com/EkranoplanCC/LinuxCPMX2077/releases).
2. Optionally check the download: `sha256sum -c SHA256SUMS --ignore-missing`.
3. Make it executable and start it:

   ```sh
   chmod +x CPMX2077_*_amd64.AppImage
   ./CPMX2077_*_amd64.AppImage
   ```

The app keeps its library in `~/.local/share/cp2077-modmanager/`, so you can
replace the AppImage with a newer one at any time without losing anything.
When a newer release is out, a notice at the top of the window links to it
(Settings → *CPMX2077 updates* turns the startup check off).

On the first start an **introduction tour** opens: a few steps that outline
the Game panel, the tabs, the Nexus account box, **Install missing
frameworks**, (on Linux) the setup fixes and the mod profiles under each
game version. **Next** and **Back** step
through it (or the arrow keys); **Skip tour** or Esc closes it. It opens
once; **Settings → Introduction tour → Start tour** shows it again. This
guide covers the same steps in more detail.

**On Windows (preview)**: download `CPMX2077_<version>_x64-setup.exe` and run
it. It installs for your user only, without admin rights. It isn't code-signed
yet, so SmartScreen asks once: choose *More info*, then *Run anyway*. Steam,
GOG Galaxy and Epic installs are found automatically, and step 5 below
doesn't apply: the game runs natively, so there's nothing Linux-side to fix.
If CET or RED4ext don't start, install the latest Microsoft Visual C++
2015-2022 Redistributable (x64). The Windows build is still an early preview
and hasn't been tried on a real Windows PC yet.

## 2. Check the game was found

CPMX2077 looks for Cyberpunk 2077 in your Steam libraries (native, Flatpak
and Snap Steam) and in Heroic. **Settings → Game installs** lists every
install it found, with the game version, the Steam build, the folder and
the Proton prefix.

- If nothing is found, the **Game** panel on the left says so: click **Add
  path…** there and pick the folder that contains `bin/x64/Cyberpunk2077.exe`.
- If you have more than one install, pick the one to mod in the drop-down
  in the Game panel. Everything else in the app works on the selected
  install.

Under **Frameworks** you see which of CET, RED4ext, redscript, ArchiveXL,
TweakXL, Codeware and REDmod are already in the game folder. Highlighted
names are installed. The switch next to **REDmod** enables or disables
REDmod mods (Steam, with Steam closed), so you never need CD Projekt's
launcher and its Enable mods setting.

The **First setup** checklist at the top of **Installed mods** lists the
next steps (frameworks, Linux setup, Nexus account), each with its button,
and ticks them off as you go.

**Already have mods in the game folder?** Mods you copied in by hand or
installed with another manager are listed under **Not installed by
CPMX2077** below the mod list. They are already included in the
compatibility check. Click **Adopt** (or **Adopt all**) to let CPMX2077
track them, so you can switch them off with **Enabled**, verify and
uninstall them like mods it installed. Adopting doesn't change the game
folder.

## 3. Install the core frameworks

Most mods need some of the frameworks, so install them first.

Click **Install missing frameworks** under the Frameworks list in the Game
panel (or in the First setup checklist). The same button is in **Get mods**
with the source at the top right switched to **GitHub**.

Each one is downloaded from its official GitHub release, checked against the
SHA-256 digest GitHub publishes, and installed in the right order (for
example RED4ext before the plugins that need it).

## 4. Connect your Nexus account

Open **Get mods**.

1. Click **Open Nexus API keys page** (or on nexusmods.com go to *Site
   preferences → API Keys*) and copy your personal API key.
2. Paste it into **Connect with an API key** and click **Connect**.

The key is checked with Nexus and stored in your system keyring (GNOME
Keyring, KWallet, or another Secret Service provider), never in a file. Your
name and account type (Free or Premium) then appear at the top of the tab.

With a free account, the line under your name has **Send Nexus downloads to
CPMX2077**: click it if you also want the "Mod Manager Download" buttons in
your normal browser to land in the app. You can change this later in
Settings.

> "Sign in with Nexus Mods" (browser sign-in) appears only once Nexus has
> registered the app. Until then, the API key is the way in.

## 5. Let the app fix the Linux side

After the frameworks go in, CPMX2077 checks what they need from Proton and
offers to fix it in one go. The same items stay listed in the **Game** panel,
each with a button that fixes it and, once fixed, an **Undo** button:

| Item (button) | What the fix does |
|---|---|
| **Visual C++ runtime** (*Install vcrun2022*) | Installs `vcrun2022` into the game's prefix with protontricks (or winetricks with the game's own Proton). CET and RED4ext do not start without it. This can take a few minutes. |
| **Steam launch options** (*Set launch options*) | Sets `WINEDLLOVERRIDES="winmm,version=n,b" %command% --launcher-skip` (plus `-modded` if you use REDmod mods and the REDmod switch is on). `--launcher-skip` starts the game without the REDprelauncher window. **Close Steam first**: Steam overwrites the setting while it runs. |
| **Shader compiler for ReShade** (*Install d3dcompiler_47*) | Only listed once ReShade is installed. Installs Microsoft's shader compiler into the prefix, which many ReShade effects need to compile. |
| **DLL overrides**, Heroic only (*Set overrides*) | Sets the same `winmm` and `version` overrides in the Heroic prefix. |
| **Folder names** (*Merge folders*) | Merges mod folders that exist under two spellings, such as `Mods` and `mods`. |
| **HDR** (*Set HDR launch options*) | Only listed when a connected display supports HDR10. Lists what HDR needs (Wayland session, desktop HDR enabled, Mesa 25.1+ or NVIDIA 595+, GE-Proton 10+ or gamescope) and adds `PROTON_ENABLE_WAYLAND=1 DXVK_HDR=1` (or a `gamescope --hdr-enabled --force-grab-cursor` wrapper, which keeps the mouse captured) to the launch options. When both work, **Set HDR with gamescope** picks gamescope instead, and **Use Wayland instead** / **Use gamescope instead** switch later. Marked as a problem while the desktop runs the display in HDR; with desktop HDR off it shows as fine. **Close Steam first**. |

Accept the fixes. If Steam is open, the launch options fix waits; close
Steam, then click **Set launch options** in the Game panel. **Copy** next to
it copies the launch option if you would rather paste it into Steam
yourself.

## 6. Install your first mod

1. In **Get mods**, search for a mod or click **Trending**, **Most
   endorsed** and so on. Click a mod to open its page.
2. Open **Files** and pick the file you want (usually under *Main files*).
3. Then:
   - **Premium**: click **Download & install**. Done.
   - **Free account**: click **Get from Nexus**. The file's page opens in a
     Nexus window inside the app. Sign in there once, then click **Slow
     download**. The app catches the download link, closes the Nexus window,
     fetches the file, checks it with Nexus, and installs it.

If the mod needs other mods you don't have (most need some of the core
frameworks), a *Missing requirements* dialog lists them first. Click
**Queue requirements first** and the app installs them, then the mod.

You stay on **Get mods** while it downloads; the download's bar is on the
mod's card and in the downloads panel at the bottom of the window, one bar per
file when several download at once. For a quick install from the list, click the **⬇** button on a mod's
picture and pick one of its main files. Click an author's name to see all
their mods.

Add-on files from the same mod page (for example LUT packs for LUT Switcher 3)
show in **Downloads** and **Installed mods** under their own file title, not as
versions of the main mod.

If the mod ships a FOMOD installer, a wizard opens and asks which options you
want, the same way it would on Windows.

You can also install an archive you already have: **Installed mods →
Install from archive…** and pick a `.zip`, `.7z` or `.rar`.

## 7. Get several mods at once

1. In **Get mods**, click **Select several**, then click each mod you want.
2. Click **Download all**.

GitHub mods and Premium downloads run straight through. With a free account,
one Nexus window steps through the file pages ("Mod 3 of 12"): click the
download button on each page and the app installs that mod in the background
while it moves on to the next. The panel at the bottom of the window shows
progress and has **Skip this mod** and **Cancel**.

Prefer a ready-made setup? **Modpacks** (under **Get mods** in the menu) lists Nexus collections. Open
one to see which of its mods you already have, then **Install required
mods** to queue the rest. With **Separate profile** ticked (the default),
your current mods are saved as a profile and disabled first, and the
collection gets a profile of its own; switch between them under
**Installed mods** › **Game version and profiles**.

## 8. Start the game and check

Launch Cyberpunk 2077 from Steam or Heroic as usual. Then open
**Installed mods** in CPMX2077: the **Netrunner** panel at the bottom of
the window holds the checks (click the strip with **Show ▴** if it is
folded). Click **Check again**. Its *Summary* tab shows anything that needs
attention: setup problems, the first step of the
last start that failed (**Start here**), whether the game crashed, mods
named in errors from that session, known problem mods, and compatibility
problems between your mods. Click a mod in the list to see only what
concerns that mod. The *Files* tab shows every file your mods installed and
where it went, and *Launch options* explains each part of the line the Linux setup fixes wrote
to Steam.

If the game crashes or a framework does not load, see
[Troubleshooting](user-guide.md#troubleshooting).
To watch what the app does step by step (API requests, downloads,
extractions, files moved), tick *Debug mode* in **Settings**: the
[terminal](user-guide.md#terminal) opens at the foot of the
sidebar. **Pop out** moves it to its own window.

## Next steps

- Turn mods off and on with the switch in the **Enabled** column (next to
  **Uninstall**) instead of uninstalling them.
- Click **Check for updates** in **Installed mods** now and then (it also
  runs at start-up).
- After a game update, open **Patch-note check** under **Installed
  mods** in the menu: it lists each mod's last update against the game
  patches, oldest first, and frameworks that need a newer release (see
  [Patch-note check](user-guide.md#patch-note-check)).
- Using Ultra+ for path tracing? Once it's installed, the **Ultra+
  recommendations** (under **Installed mods** in the menu) shows what its authors
  recommend with it and what conflicts with it.
- Want ReShade? Open **ReShade** under **Installed mods** in the menu and click
  **Install ReShade**. On Linux, accept the launch options and
  d3dcompiler_47 fixes the app offers afterwards (close Steam first). See
  [ReShade](user-guide.md#reshade) in the user guide.
- Want your own radio station? Open **RadioEXT** under **Installed
  mods**, click **Install radioExt** if it's missing, then **New station**
  and **Add songs…**. For your own logo, pick **PNG image** as the icon
  source. See [RadioEXT](user-guide.md#radioext).
- Modding tools: the Netrunner panel's **WolvenKit** tab installs WolvenKit from GitHub.
  Its command-line build needs the .NET 10 runtime (on Linux, your
  distribution's `dotnet-runtime-10.0` package). See
  [WolvenKit](user-guide.md#wolvenkit).
- Read the [user guide](user-guide.md) for everything else.
