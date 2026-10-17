//! CPMX2077 HID Input: a RED4ext plugin that adds HID game controllers
//! (pedals, wheels, flight sticks, button boxes) to the XInput gamepad
//! Cyberpunk 2077 reads. Bindings are set in the game under Settings › Mods
//! › HID Input (the Mod Settings mod, which saves them to its `user.ini`);
//! the plugin reads them at start and again whenever they change, and
//! writes `status.json` with what it found. Windows only: on other targets
//! this crate builds empty.

#[cfg(windows)]
mod plugin;
