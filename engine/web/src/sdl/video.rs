//! SDL's windows and displays: one window, the page's canvas, on one display, the browser's screen.

use std::cell::Cell;
use std::ffi::{c_char, c_float, c_int};

use sdl3_sys::everything::*;

#[link(wasm_import_module = "vpi_page")]
unsafe extern "C" {
    /// The page's device pixels per CSS pixel.
    fn page_pixel_density() -> c_float;
    /// The screen's size in device pixels.
    fn page_screen_size(width: *mut u32, height: *mut u32);
    /// How often the screen refreshes, as the page measured it (0: not yet known).
    fn page_refresh_rate() -> c_float;
    /// Asks for the canvas full screen (1) or not (0): the browser allows it after a press.
    fn page_fullscreen(on: u32);
}

/// The window, our one: its number, and whether it has the keyboard.
pub struct Window {
    pub id: SDL_WindowID,
    props: SDL_PropertiesID,
}

thread_local! {
    static WINDOW: Cell<*mut Window> = const { Cell::new(std::ptr::null_mut()) };
    static FOCUSED: Cell<bool> = const { Cell::new(true) };
    static MODE: Cell<SDL_DisplayMode> = Cell::new(unsafe { std::mem::zeroed() });
    static DISPLAY_PROPS: Cell<SDL_PropertiesID> = const { Cell::new(SDL_PropertiesID(0)) };
}

pub const DISPLAY: SDL_DisplayID = SDL_DisplayID(1);

/// The page's canvas has the keyboard (or lost it).
pub fn set_focused(focused: bool) {
    FOCUSED.set(focused);
}

/// The window's number, once made.
pub fn window_id() -> SDL_WindowID {
    WINDOW.with(|w| if w.get().is_null() { SDL_WindowID(0) } else { SDL_WindowID(1) })
}

fn canvas_pixels() -> (c_int, c_int) {
    let (w, h) = super::gpu::canvas_size();
    (w as c_int, h as c_int)
}

fn density() -> f32 {
    // SAFETY: a plain call into the page
    unsafe { page_pixel_density() }.max(0.25)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_CreateWindow(_title: *const c_char, _w: c_int, _h: c_int, _flags: SDL_WindowFlags) -> *mut SDL_Window {
    WINDOW.with(|w| {
        if w.get().is_null() {
            // SAFETY: our own properties
            let props = unsafe { SDL_CreateProperties() };
            w.set(Box::into_raw(Box::new(Window { id: SDL_WindowID(1), props })));
        }
        w.get().cast()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_CreateWindowWithProperties(_props: SDL_PropertiesID) -> *mut SDL_Window {
    // SAFETY: as SDL_CreateWindow
    unsafe { SDL_CreateWindow(std::ptr::null(), 0, 0, SDL_WindowFlags(0)) }
}

/// The canvas stays: the page's own.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_DestroyWindow(_window: *mut SDL_Window) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_ShowWindow(_window: *mut SDL_Window) -> bool {
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_HideWindow(_window: *mut SDL_Window) -> bool {
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_RaiseWindow(_window: *mut SDL_Window) -> bool {
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_SetWindowFullscreen(_window: *mut SDL_Window, fullscreen: bool) -> bool {
    // SAFETY: a plain call into the page
    unsafe { page_fullscreen(u32::from(fullscreen)) };
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_SetWindowPosition(_window: *mut SDL_Window, _x: c_int, _y: c_int) -> bool {
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetWindowPosition(_window: *mut SDL_Window, x: *mut c_int, y: *mut c_int) -> bool {
    // SAFETY: the caller's out-parameters, each may be null
    unsafe {
        if !x.is_null() {
            *x = 0;
        }
        if !y.is_null() {
            *y = 0;
        }
    }
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetWindowSizeInPixels(_window: *mut SDL_Window, w: *mut c_int, h: *mut c_int) -> bool {
    let (pw, ph) = canvas_pixels();
    // SAFETY: the caller's out-parameters, each may be null
    unsafe {
        if !w.is_null() {
            *w = pw;
        }
        if !h.is_null() {
            *h = ph;
        }
    }
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetWindowSize(_window: *mut SDL_Window, w: *mut c_int, h: *mut c_int) -> bool {
    let (pw, ph) = canvas_pixels();
    let d = density();
    // SAFETY: the caller's out-parameters, each may be null
    unsafe {
        if !w.is_null() {
            *w = (pw as f32 / d).round() as c_int;
        }
        if !h.is_null() {
            *h = (ph as f32 / d).round() as c_int;
        }
    }
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetWindowPixelDensity(_window: *mut SDL_Window) -> c_float {
    density()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetWindowFlags(_window: *mut SDL_Window) -> SDL_WindowFlags {
    let focus = if FOCUSED.get() { SDL_WINDOW_INPUT_FOCUS | SDL_WINDOW_MOUSE_FOCUS } else { SDL_WindowFlags(0) };
    SDL_WINDOW_HIGH_PIXEL_DENSITY | focus
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetWindowID(_window: *mut SDL_Window) -> SDL_WindowID {
    SDL_WindowID(1)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetWindowProperties(window: *mut SDL_Window) -> SDL_PropertiesID {
    // SAFETY: our window, if it's ours
    unsafe { window.cast::<Window>().as_ref() }.map_or(SDL_PropertiesID(0), |w| w.props)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetDisplayForWindow(_window: *mut SDL_Window) -> SDL_DisplayID {
    DISPLAY
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetDisplays(count: *mut c_int) -> *mut SDL_DisplayID {
    if !count.is_null() {
        // SAFETY: the caller's out-parameter
        unsafe { *count = 1 };
    }
    super::c_alloc(&[DISPLAY, SDL_DisplayID(0)])
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetPrimaryDisplay() -> SDL_DisplayID {
    DISPLAY
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetDisplayName(_display: SDL_DisplayID) -> *const c_char {
    c"the browser's screen".as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetDisplayProperties(_display: SDL_DisplayID) -> SDL_PropertiesID {
    DISPLAY_PROPS.with(|p| {
        if p.get().0 == 0 {
            // SAFETY: our own properties
            p.set(unsafe { SDL_CreateProperties() });
        }
        p.get()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetDisplayContentScale(_display: SDL_DisplayID) -> c_float {
    density()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetDisplayBounds(_display: SDL_DisplayID, rect: *mut SDL_Rect) -> bool {
    let (mut w, mut h) = (0, 0);
    // SAFETY: a plain call into the page; the caller's out-parameter
    unsafe {
        page_screen_size(&mut w, &mut h);
        if !rect.is_null() {
            let d = density();
            *rect = SDL_Rect { x: 0, y: 0, w: (w as f32 / d) as c_int, h: (h as f32 / d) as c_int };
        }
    }
    true
}

/// The screen's mode: its size in pixels and its refresh rate as the page measured it.
fn mode() -> *const SDL_DisplayMode {
    let (mut w, mut h) = (0, 0);
    // SAFETY: plain calls into the page
    let refresh = unsafe {
        page_screen_size(&mut w, &mut h);
        page_refresh_rate()
    };
    MODE.with(|m| {
        // SAFETY: a plain structure, every field set
        let mut mode: SDL_DisplayMode = unsafe { std::mem::zeroed() };
        mode.displayID = DISPLAY;
        mode.format = SDL_PIXELFORMAT_XRGB8888;
        let d = density();
        mode.w = (w as f32 / d) as c_int;
        mode.h = (h as f32 / d) as c_int;
        mode.pixel_density = d;
        mode.refresh_rate = if refresh > 0.0 { refresh } else { 60.0 };
        mode.refresh_rate_numerator = (mode.refresh_rate * 1000.0).round() as c_int;
        mode.refresh_rate_denominator = 1000;
        m.set(mode);
        m.as_ptr().cast_const()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetCurrentDisplayMode(_display: SDL_DisplayID) -> *const SDL_DisplayMode {
    mode()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetDesktopDisplayMode(_display: SDL_DisplayID) -> *const SDL_DisplayMode {
    mode()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetGlobalMouseState(x: *mut c_float, y: *mut c_float) -> SDL_MouseButtonFlags {
    let (mx, my) = super::events::mouse_at();
    // SAFETY: the caller's out-parameters, each may be null
    unsafe {
        if !x.is_null() {
            *x = mx;
        }
        if !y.is_null() {
            *y = my;
        }
    }
    SDL_MouseButtonFlags(0)
}
