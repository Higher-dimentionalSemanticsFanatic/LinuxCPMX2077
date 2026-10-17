# CPMX2077 HID Input

A RED4ext plugin for Cyberpunk 2077 that adds HID game controllers
(pedals, wheels, flight sticks, throttles, button boxes) to the XInput
gamepad the game reads. Triggers take the larger value of the gamepad and
the bound inputs, stick axes add up, buttons combine. Works next to a
gamepad or with mouse and keyboard alone.

## Install

Unpack the archive into the game folder (or install it with a mod
manager). It holds:

- `red4ext/plugins/cpmx_hid_input/cpmx_hid_input.dll`: the plugin
- `r6/scripts/cpmx_hid_input/HIDInput.reds`: the in-game settings

Needs RED4ext. The in-game settings need
[Mod Settings](https://www.nexusmods.com/cyberpunk2077/mods/4885), which
needs redscript and ArchiveXL.

## Settings › Mods › HID Input

- **General**: **Bindings on**, **Gamepad slot**, **Report a gamepad when
  none is connected** (with no gamepad on the slot, the plugin reports one
  while a bound controller is connected).
- One section per analog gamepad input (**Right trigger**, **Left
  trigger**, both sticks' X and Y): **Controller**, **Input**, **Invert**,
  **Deadzone (%)**, **Response**, and **Stick range** on stick axes.
- **Buttons**: controller and input for each gamepad button and d-pad
  direction. An axis counts as pressed past half its travel.
- **D-pad (hat switch)**: a hat switch drives all four d-pad directions.

The plugin reads the values again within a second of Mod Settings saving
them, also while the game runs.

**Controller numbers**: game controllers that are not XInput gamepads,
numbered by USB vendor and product ID (the same numbers every session
while the same controllers are connected). Axis, button and hat numbers
are the order the plugin reads the controller in.
`red4ext/plugins/cpmx_hid_input/status.json`, written while the game
runs, lists each numbered controller with its axes, so you can look up
which number is the brake pedal. Many pedals rest at the top of their
range: turn **Invert** on for those.

## With CPMX2077

CPMX2077 (Installed mods › HID input) installs the same files, binds by
moving the input on a gamepad drawing, writes `bindings.json` next to the
plugin and shows what `status.json` reports. Bindings from both places
apply; once saved in the game menu, its **General** settings are the ones
used.
