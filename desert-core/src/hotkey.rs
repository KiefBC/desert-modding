//! Edge-triggered polling of virtual keys via GetAsyncKeyState.

use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

pub struct Hotkey {
    vk: u16,
    was_down: bool,
}

impl Hotkey {
    pub fn new(vk: u16) -> Self {
        Hotkey { vk, was_down: false }
    }

    /// True exactly once per press.
    pub fn pressed(&mut self) -> bool {
        // SAFETY: GetAsyncKeyState takes a plain integer and reads only the
        // system's own async key state; it touches no memory of ours and has no
        // precondition — an i32 that is not a virtual-key code just returns 0.
        // It is callable from any thread, and this is polled from the plugin's
        // own worker thread.
        let down = (unsafe { GetAsyncKeyState(self.vk as i32) } as u16 & 0x8000) != 0;
        let edge = down && !self.was_down;
        self.was_down = down;
        edge
    }
}
