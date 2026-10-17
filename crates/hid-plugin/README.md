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

Needs RED4ext and
[Mod Settings](https://www.nexusmods.com/cyberpunk2077/mods/4885) (which
needs redscript and ArchiveXL): bindings are set in Mod Settings' menu.

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

## Finding the numbers

The menu binds by number: **Controller 1** to **4**, and **Axis**,
**Button** or **Hat** 1 to n on that controller.

- **Controller numbers**: game controllers that are not XInput gamepads,
  numbered by USB vendor and product ID (the same numbers every session
  while the same controllers are connected).
- **Input numbers**: the order the plugin reads the controller in.

While the game runs, the plugin writes
`red4ext/plugins/cpmx_hid_input/status.json`. Each entry under `devices`
is one controller: `controller` is its number, `axis_names` and
`axis_values` its axes and where they are now (percent), and
`recent_inputs` the last inputs moved, newest first. Press the brake pedal
once and it shows as, for example, `"Axis 3"` at the top of
`recent_inputs`. Many pedals rest at the top of their range (`axis_values`
near 100): turn **Invert** on for those.

`missing` lists bound controller numbers with no controller connected,
`active_bindings` the bindings in use.

## Install with a mod manager

The archive is laid out from the game folder, so mod managers that install
RED4ext plugins and redscript files take it as is (CPMX2077: **Install
from archive…**). To uninstall, remove the folders
`red4ext/plugins/cpmx_hid_input` and `r6/scripts/cpmx_hid_input`.
