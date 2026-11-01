//! The part of SDL the player calls, made for the browser: SDL itself isn't in the program, its
//! functions are these, on the page's WebGPU, Web Audio, keyboard, mouse and gamepads
//! (`site/vpi.js`), with SDL_ttf's part (`ttf`). What the browser has no use for (PinMAME's
//! windows, HID devices, VR) is left out: the browser's play never reaches it, and the program
//! links only what it does (the cabinet's shared memory answers as a failure would, `absent`).

pub mod absent;
pub mod audio;
pub mod events;
pub mod gpu;
pub mod joystick;
pub mod props;
pub mod ttf;
pub mod video;

use std::cell::RefCell;
use std::collections::HashSet;
use std::ffi::{CString, c_char, c_void};
use std::sync::OnceLock;
use std::time::Instant;

use sdl3_sys::everything::*;

thread_local! {
    static ERROR: RefCell<CString> = RefCell::new(CString::default());
    static SAID: RefCell<HashSet<&'static str>> = RefCell::new(HashSet::new());
}

/// What `SDL_GetError` says next.
pub fn set_error(text: &str) {
    ERROR.with(|e| *e.borrow_mut() = CString::new(text.replace('\0', " ")).unwrap_or_default());
}

/// Says `text` in the console the first time only.
pub fn once(text: &'static str) {
    if SAID.with(|s| s.borrow_mut().insert(text)) {
        eprintln!("{text}");
    }
}

/// A copy of `text` made with the C library's allocator, as SDL hands strings and lists back
/// (freed with `SDL_free`).
pub fn c_alloc<T: Copy>(items: &[T]) -> *mut T {
    // SAFETY: room for the items, written before it's handed out
    unsafe {
        let p = malloc(std::mem::size_of_val(items).max(1)).cast::<T>();
        if !p.is_null() {
            std::ptr::copy_nonoverlapping(items.as_ptr(), p, items.len());
        }
        p
    }
}

unsafe extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_malloc(size: usize) -> *mut c_void {
    // SAFETY: the C library's allocator
    unsafe { malloc(size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_free(mem: *mut c_void) {
    // SAFETY: what SDL_malloc (or c_alloc) gave
    unsafe { free(mem) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetError() -> *const c_char {
    ERROR.with(|e| e.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_Init(_flags: SDL_InitFlags) -> bool {
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_Quit() {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_SetHint(_name: *const c_char, _value: *const c_char) -> bool {
    true
}

fn start() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

/// SDL's clock: nanoseconds since the program started, by the page's (`performance.now`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetTicksNS() -> Uint64 {
    start().elapsed().as_nanos() as u64 + 1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetTicks() -> Uint64 {
    start().elapsed().as_millis() as u64 + 1
}

/// The browser's thread is never put to sleep: a wait comes back at once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_DelayPrecise(_ns: Uint64) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_Delay(_ms: Uint32) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GUIDToString(guid: SDL_GUID, psz_guid: *mut c_char, cb_guid: core::ffi::c_int) {
    if psz_guid.is_null() || cb_guid <= 0 {
        return;
    }
    let text: String = guid.data.iter().map(|b| format!("{b:02x}")).collect();
    let n = text.len().min(cb_guid as usize - 1);
    // SAFETY: room for `cb_guid` characters, the last for the NUL
    unsafe {
        std::ptr::copy_nonoverlapping(text.as_ptr().cast(), psz_guid, n);
        *psz_guid.add(n) = 0;
    }
}
