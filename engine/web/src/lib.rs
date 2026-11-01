//! Visual Pinball Infinite in a web browser: one table, played by the player's own code, on the
//! browser's WebGPU, Web Audio, keyboard, mouse and gamepads. The program is a WebAssembly module
//! (wasm32-wasip1) that the page (`site/`) starts and drives: it reads the table and the settings
//! from the page's files (WASI: the table's folder, read only, and the data folder the page keeps
//! in the browser's local storage), and each animation frame the page hands it what the player did
//! and asks for a frame and the sound to come.
//!
//! The page's side of SDL is `sdl` (SDL itself isn't in the program); the play is `play`.

#![cfg(target_family = "wasm")]

pub mod play;
pub mod sdl;

use std::cell::RefCell;
use std::path::PathBuf;

thread_local! {
    static PLAY: RefCell<Option<play::Play>> = const { RefCell::new(None) };
    static SCRATCH: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

unsafe extern "C" {
    /// The linker's: the C library's and libwinevbs's constructors.
    fn __wasm_call_ctors();
}

/// Sets the program up, before anything else: its C code's constructors run.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_web_init() {
    // SAFETY: once, first, as a WASI reactor's `_initialize` does
    unsafe { __wasm_call_ctors() };
}

/// Starts the table `VPI_TABLE` names; 1 when it plays, 0 when it can't (the reason printed).
#[unsafe(no_mangle)]
pub extern "C" fn vpi_web_start(refresh_hz: f32) -> i32 {
    let Some(table) = std::env::var_os("VPI_TABLE").map(PathBuf::from) else {
        eprintln!("vpi-web: no table (VPI_TABLE)");
        return 0;
    };
    match play::Play::start(&table, refresh_hz) {
        Ok(p) => {
            PLAY.with(|slot| *slot.borrow_mut() = Some(p));
            1
        }
        Err(e) => {
            eprintln!("vpi-web: {e}");
            0
        }
    }
}

/// One frame: 1 while play goes on, 0 once it has ended, -1 when drawing failed (said why).
#[unsafe(no_mangle)]
pub extern "C" fn vpi_web_frame() -> i32 {
    PLAY.with(|slot| match slot.borrow_mut().as_mut().map(play::Play::frame) {
        Some(Ok(true)) => 1,
        Some(Ok(false)) | None => 0,
        Some(Err(e)) => {
            eprintln!("vpi-web: {e}");
            -1
        }
    })
}

/// The play as it stands, as JSON (for the page's console): its length, the text in the scratch room.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_web_state() -> usize {
    let text = PLAY.with(|slot| slot.borrow().as_ref().map_or_else(|| "null".to_string(), play::Play::state));
    let p = vpi_web_scratch(text.len());
    // SAFETY: the scratch room holds at least the text's bytes
    unsafe { std::ptr::copy_nonoverlapping(text.as_ptr(), p, text.len()) };
    text.len()
}

/// The frame-rate limit the settings ask for (0: none).
#[unsafe(no_mangle)]
pub extern "C" fn vpi_web_max_fps() -> f64 {
    PLAY.with(|slot| slot.borrow().as_ref().map_or(0.0, play::Play::max_fps))
}

/// The screen's refresh, as the page measures it.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_web_refresh(hz: f32) {
    PLAY.with(|slot| {
        if let Some(p) = slot.borrow_mut().as_mut() {
            p.set_refresh(hz);
        }
    });
}

/// Room for the page to write `size` bytes into, for the next call that reads them.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_web_scratch(size: usize) -> *mut u8 {
    SCRATCH.with(|s| {
        let mut s = s.borrow_mut();
        let len = size.max(s.len());
        s.resize(len, 0);
        s.as_mut_ptr()
    })
}

/// A key pressed or let go: SDL's scancode for it (the page maps `KeyboardEvent.code`).
#[unsafe(no_mangle)]
pub extern "C" fn vpi_web_key(scancode: u32, down: i32, repeat: i32) {
    sdl::events::key(scancode, down != 0, repeat != 0);
}

/// The mouse over the canvas, in its CSS pixels.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_web_mouse_move(x: f32, y: f32) {
    sdl::events::mouse_move(x, y);
}

#[unsafe(no_mangle)]
pub extern "C" fn vpi_web_mouse_button(button: u32, down: i32, x: f32, y: f32) {
    sdl::events::mouse_button(button as u8, down != 0, x, y);
}

#[unsafe(no_mangle)]
pub extern "C" fn vpi_web_mouse_wheel(notches: f32) {
    sdl::events::mouse_wheel(notches);
}

/// The canvas took the keyboard (1) or lost it (0).
#[unsafe(no_mangle)]
pub extern "C" fn vpi_web_focus(gained: i32) {
    sdl::events::focus(gained != 0);
}

#[unsafe(no_mangle)]
pub extern "C" fn vpi_web_resized(width: u32, height: u32) {
    sdl::events::resized(width, height);
}

/// A gamepad's state now, written into the scratch room: its name's `name_len` bytes, then its
/// buttons' values and its axes as 32-bit floats (`buttons` and `axes` of them), the name padded
/// to 4 bytes. `standard`: the browser's standard mapping.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_web_pad(index: u32, standard: i32, name_len: usize, buttons: usize, axes: usize) {
    SCRATCH.with(|s| {
        let s = s.borrow();
        let name_end = name_len.next_multiple_of(4);
        let need = name_end + 4 * (buttons + axes);
        if s.len() < need {
            return;
        }
        let name = String::from_utf8_lossy(&s[..name_len]);
        let floats = |at: usize, n: usize| -> Vec<f32> { (0..n).map(|k| f32::from_le_bytes(s[at + 4 * k..at + 4 * k + 4].try_into().unwrap_or_default())).collect() };
        let values = floats(name_end, buttons);
        let moves = floats(name_end + 4 * buttons, axes);
        sdl::joystick::update(index, &name, standard != 0, &values, &moves);
    });
}

/// A gamepad the page no longer has.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_web_pad_gone(index: u32) {
    sdl::joystick::gone(index);
}

/// `frames` frames of sound for the page, stereo, 48 kHz, as 32-bit floats; they stay where they
/// are until the next call.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_web_audio(frames: u32) -> *const f32 {
    sdl::audio::pump(frames as usize)
}
