//! SDL's events: the page's keys, mouse, focus and size, and the gamepads' presses and moves,
//! made SDL's, handed to the event watch (the player's input) as they come and queued for
//! `SDL_PollEvent`, as SDL does.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::{c_char, c_void};

use sdl3_sys::everything::*;

thread_local! {
    static QUEUE: RefCell<VecDeque<SDL_Event>> = const { RefCell::new(VecDeque::new()) };
    static WATCHES: RefCell<Vec<(SDL_EventFilter, usize)>> = const { RefCell::new(Vec::new()) };
    static MOUSE: Cell<(f32, f32)> = const { Cell::new((0.0, 0.0)) };
    static BUTTONS: Cell<u32> = const { Cell::new(0) };
}

/// Where the mouse is over the canvas, in its pixels.
pub fn mouse_at() -> (f32, f32) {
    MOUSE.get()
}

fn now() -> u64 {
    // SAFETY: our own clock
    unsafe { super::SDL_GetTicksNS() }
}

/// An event of `kind`, its time now, the rest zero.
pub fn event(kind: SDL_EventType) -> SDL_Event {
    // SAFETY: SDL's events are plain data
    let mut e: SDL_Event = unsafe { std::mem::zeroed() };
    e.r#type = kind.0;
    e.common.timestamp = now();
    e
}

/// Hands `e` to the watches, then queues it.
pub fn push(mut e: SDL_Event) {
    let watches = WATCHES.with(|w| w.borrow().clone());
    for (filter, userdata) in watches {
        if let Some(f) = filter {
            // SAFETY: the watch and its userdata as the program added them
            unsafe { f(userdata as *mut c_void, &mut e) };
        }
    }
    QUEUE.with(|q| {
        let mut q = q.borrow_mut();
        // a program that stopped reading loses the oldest, as SDL's queue caps
        if q.len() >= 65_536 {
            q.pop_front();
        }
        q.push_back(e);
    });
}

/// A key pressed or let go: its SDL scancode (the page's `KeyboardEvent.code`, made SDL's).
pub fn key(scancode: u32, down: bool, repeat: bool) {
    let mut e = event(if down { SDL_EVENT_KEY_DOWN } else { SDL_EVENT_KEY_UP });
    e.key.windowID = super::video::window_id();
    e.key.scancode = SDL_Scancode(scancode as _);
    e.key.down = down;
    e.key.repeat = repeat;
    push(e);
}

/// The mouse moved to (x, y), in the canvas's pixels.
pub fn mouse_move(x: f32, y: f32) {
    let (ox, oy) = MOUSE.replace((x, y));
    let mut e = event(SDL_EVENT_MOUSE_MOTION);
    e.motion.windowID = super::video::window_id();
    e.motion.which = SDL_MouseID(1);
    e.motion.state = SDL_MouseButtonFlags(BUTTONS.get());
    (e.motion.x, e.motion.y, e.motion.xrel, e.motion.yrel) = (x, y, x - ox, y - oy);
    push(e);
}

/// A mouse button (SDL's: 1 left, 2 middle, 3 right) pressed or let go at (x, y).
pub fn mouse_button(button: u8, down: bool, x: f32, y: f32) {
    MOUSE.set((x, y));
    let bit = 1u32 << (button.max(1) - 1);
    BUTTONS.set(if down { BUTTONS.get() | bit } else { BUTTONS.get() & !bit });
    let mut e = event(if down { SDL_EVENT_MOUSE_BUTTON_DOWN } else { SDL_EVENT_MOUSE_BUTTON_UP });
    e.button.windowID = super::video::window_id();
    e.button.which = SDL_MouseID(1);
    e.button.button = button;
    e.button.down = down;
    e.button.clicks = 1;
    (e.button.x, e.button.y) = (x, y);
    push(e);
}

/// The mouse's wheel turned: up positive, in notches.
pub fn mouse_wheel(dy: f32) {
    let mut e = event(SDL_EVENT_MOUSE_WHEEL);
    e.wheel.windowID = super::video::window_id();
    e.wheel.which = SDL_MouseID(1);
    e.wheel.y = dy;
    e.wheel.integer_y = dy.round() as _;
    let (x, y) = MOUSE.get();
    (e.wheel.mouse_x, e.wheel.mouse_y) = (x, y);
    push(e);
}

/// The canvas took the keyboard, or lost it (another tab, another window).
pub fn focus(gained: bool) {
    super::video::set_focused(gained);
    let mut e = event(if gained { SDL_EVENT_WINDOW_FOCUS_GAINED } else { SDL_EVENT_WINDOW_FOCUS_LOST });
    e.window.windowID = super::video::window_id();
    push(e);
}

/// The canvas's size in pixels changed.
pub fn resized(width: u32, height: u32) {
    let mut e = event(SDL_EVENT_WINDOW_PIXEL_SIZE_CHANGED);
    e.window.windowID = super::video::window_id();
    (e.window.data1, e.window.data2) = (width as i32, height as i32);
    push(e);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_AddEventWatch(filter: SDL_EventFilter, userdata: *mut c_void) -> bool {
    WATCHES.with(|w| w.borrow_mut().push((filter, userdata as usize)));
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_RemoveEventWatch(filter: SDL_EventFilter, userdata: *mut c_void) {
    let gone = filter.map(|f| f as usize);
    WATCHES.with(|w| w.borrow_mut().retain(|(f, u)| !(f.map(|f| f as usize) == gone && *u == userdata as usize)));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_PollEvent(event: *mut SDL_Event) -> bool {
    match QUEUE.with(|q| q.borrow_mut().pop_front()) {
        Some(e) => {
            if !event.is_null() {
                // SAFETY: the caller's event to fill
                unsafe { *event = e };
            }
            true
        }
        None => false,
    }
}

/// The browser's thread never waits: an event if one's queued.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_WaitEventTimeout(event: *mut SDL_Event, _timeout_ms: Sint32) -> bool {
    if event.is_null() {
        return QUEUE.with(|q| !q.borrow().is_empty());
    }
    // SAFETY: as SDL_PollEvent
    unsafe { SDL_PollEvent(event) }
}

/// SDL's names for the keys, by scancode (SDL_keyboard.c's `SDL_scancode_names`), for the pause
/// menu's Controls page.
const NAMES: &[(u32, &str)] = &[
    (4, "A"), (5, "B"), (6, "C"), (7, "D"), (8, "E"), (9, "F"), (10, "G"), (11, "H"), (12, "I"), (13, "J"), (14, "K"),
    (15, "L"), (16, "M"), (17, "N"), (18, "O"), (19, "P"), (20, "Q"), (21, "R"), (22, "S"), (23, "T"), (24, "U"),
    (25, "V"), (26, "W"), (27, "X"), (28, "Y"), (29, "Z"), (30, "1"), (31, "2"), (32, "3"), (33, "4"), (34, "5"),
    (35, "6"), (36, "7"), (37, "8"), (38, "9"), (39, "0"), (40, "Return"), (41, "Escape"), (42, "Backspace"),
    (43, "Tab"), (44, "Space"), (45, "-"), (46, "="), (47, "["), (48, "]"), (49, "\\"), (50, "#"), (51, ";"),
    (52, "'"), (53, "`"), (54, ","), (55, "."), (56, "/"), (57, "CapsLock"), (58, "F1"), (59, "F2"), (60, "F3"),
    (61, "F4"), (62, "F5"), (63, "F6"), (64, "F7"), (65, "F8"), (66, "F9"), (67, "F10"), (68, "F11"), (69, "F12"),
    (70, "PrintScreen"), (71, "ScrollLock"), (72, "Pause"), (73, "Insert"), (74, "Home"), (75, "PageUp"),
    (76, "Delete"), (77, "End"), (78, "PageDown"), (79, "Right"), (80, "Left"), (81, "Down"), (82, "Up"),
    (83, "Numlock"), (84, "Keypad /"), (85, "Keypad *"), (86, "Keypad -"), (87, "Keypad +"), (88, "Keypad Enter"),
    (89, "Keypad 1"), (90, "Keypad 2"), (91, "Keypad 3"), (92, "Keypad 4"), (93, "Keypad 5"), (94, "Keypad 6"),
    (95, "Keypad 7"), (96, "Keypad 8"), (97, "Keypad 9"), (98, "Keypad 0"), (99, "Keypad ."), (100, "NonUSBackslash"),
    (101, "Application"), (224, "Left Ctrl"), (225, "Left Shift"), (226, "Left Alt"), (227, "Left GUI"),
    (228, "Right Ctrl"), (229, "Right Shift"), (230, "Right Alt"), (231, "Right GUI"),
];

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetScancodeName(scancode: SDL_Scancode) -> *const c_char {
    thread_local! {
        static CACHE: RefCell<Vec<(u32, std::ffi::CString)>> = const { RefCell::new(Vec::new()) };
    }
    let code = scancode.0 as u32;
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((_, s)) = c.iter().find(|(k, _)| *k == code) {
            return s.as_ptr();
        }
        // the page's name for a key it hands on as another (`VPI_KEY_NAMES`, `<scancode>=<name>` split by `;`:
        // Windows' Caps Lock and Enter, heard as the Shifts), else SDL's
        let page = std::env::var("VPI_KEY_NAMES").ok().and_then(|names| {
            names.split(';').find_map(|pair| pair.split_once('=').filter(|(k, _)| k.trim().parse::<u32>().ok() == Some(code)).map(|(_, n)| n.trim().to_string()))
        });
        let Some(name) = page.or_else(|| NAMES.iter().find(|(k, _)| *k == code).map(|(_, n)| (*n).to_string())) else { return c"".as_ptr() };
        c.push((code, std::ffi::CString::new(name).unwrap_or_default()));
        c.last().map_or(c"".as_ptr(), |(_, s)| s.as_ptr())
    })
}
