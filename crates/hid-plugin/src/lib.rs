//! CPMX2077 HID Input: a RED4ext plugin that adds HID game controllers
//! (pedals, wheels, flight sticks, button boxes) to the XInput gamepad
//! Cyberpunk 2077 reads. CPMX2077 writes `bindings.json` next to the DLL;
//! the plugin reads it at start and again whenever it changes, and writes
//! `status.json` with what it found. Windows only: on other targets this
//! crate builds empty.

#[cfg(windows)]
mod plugin;
