//! The browser's gamepads as SDL's joysticks and gamepads. The page reads each pad every frame
//! (the Gamepad API) and hands its state here; what changed becomes SDL's joystick events, which
//! the player's input maps as it maps any controller's (VPX's gamepad layout unless the settings
//! map it).
//!
//! A pad with the browser's standard mapping is a gamepad of 17 buttons, as the W3C numbers them
//! (A, B, X, Y, the shoulders, the triggers, back, start, the sticks' presses, the pad's four ways,
//! home), and 6 axes: the left stick's x and y, the right's, then the left and right triggers, from
//! -1 let go to 1 pulled through, as SDL's joysticks give a trigger. Other pads are joysticks of
//! their own buttons and axes.

use std::cell::RefCell;
use std::ffi::{CString, c_char, c_int};

use sdl3_sys::everything::*;

#[link(wasm_import_module = "vpi_page")]
unsafe extern "C" {
    /// Rumbles the page's pad `index`: its strong and weak motors, 0 to 1, for `ms`.
    fn page_rumble(index: u32, strong: f32, weak: f32, ms: u32);
}

/// Buttons and axes of a standard pad.
const BUTTONS: usize = 17;
const AXES: usize = 6;

/// A pad, connected or gone: SDL's handles to it stay valid while the player runs.
pub struct Pad {
    id: SDL_JoystickID,
    /// The page's index for it (`navigator.getGamepads()`).
    index: u32,
    name: CString,
    vendor: u16,
    product: u16,
    standard: bool,
    connected: bool,
    buttons: Vec<bool>,
    axes: Vec<i16>,
}

thread_local! {
    static PADS: RefCell<Vec<Box<Pad>>> = const { RefCell::new(Vec::new()) };
    static NEXT_ID: std::cell::Cell<u32> = const { std::cell::Cell::new(1) };
}

/// The vendor and product the browser puts in a pad's name ("... Vendor: 054c Product: 09cc", or
/// Firefox's "054c-09cc-...").
fn ids(name: &str) -> (u16, u16) {
    let hex = |s: &str| u16::from_str_radix(s.get(..4)?, 16).ok();
    if let (Some(v), Some(p)) = (name.find("Vendor: "), name.find("Product: ")) {
        return (hex(&name[v + 8..]).unwrap_or(0), hex(&name[p + 9..]).unwrap_or(0));
    }
    let mut parts = name.split('-');
    match (parts.next().and_then(hex), parts.next().and_then(hex)) {
        (Some(v), Some(p)) if name.as_bytes().get(4) == Some(&b'-') => (v, p),
        _ => (0, 0),
    }
}

/// A pad's state now, from the page: its name, its mapping, its buttons' values (0 to 1) and its
/// axes (-1 to 1). Connected for the first time, or again, it's announced; what changed since
/// becomes SDL's events.
pub fn update(index: u32, name: &str, standard: bool, buttons: &[f32], axes: &[f32]) {
    let (id, added, changes) = PADS.with(|pads| {
        let mut pads = pads.borrow_mut();
        let found = pads.iter().position(|p| p.index == index && p.connected && p.name.to_bytes() == name.as_bytes());
        let (k, added) = match found {
            Some(k) => (k, false),
            None => {
                let id = SDL_JoystickID(NEXT_ID.replace(NEXT_ID.get() + 1));
                let (vendor, product) = ids(name);
                let (b, a) = if standard { (BUTTONS, AXES) } else { (buttons.len(), axes.len()) };
                pads.push(Box::new(Pad {
                    id,
                    index,
                    name: CString::new(name.replace('\0', " ")).unwrap_or_default(),
                    vendor,
                    product,
                    standard,
                    connected: true,
                    buttons: vec![false; b],
                    // a trigger at rest is let go
                    axes: (0..a).map(|k| if standard && k >= 4 { i16::MIN } else { 0 }).collect(),
                }));
                (pads.len() - 1, true)
            }
        };
        let pad = &mut pads[k];
        // the axes now: a standard pad's triggers are its buttons 6 and 7
        let value = |v: f32| (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        let axes_now: Vec<i16> = if pad.standard {
            let at = |k: usize| axes.get(k).copied().unwrap_or(0.0);
            let trigger = |k: usize| value(buttons.get(k).copied().unwrap_or(0.0) * 2.0 - 1.0);
            vec![value(at(0)), value(at(1)), value(at(2)), value(at(3)), trigger(6), trigger(7)]
        } else {
            axes.iter().map(|&v| value(v)).collect()
        };
        let mut changes = Vec::new();
        for (k, now) in buttons.iter().map(|&v| v > 0.5).enumerate().take(pad.buttons.len()) {
            if pad.buttons[k] != now {
                pad.buttons[k] = now;
                changes.push(Change::Button(k as u8, now));
            }
        }
        for (k, now) in axes_now.into_iter().enumerate().take(pad.axes.len()) {
            if pad.axes[k] != now {
                pad.axes[k] = now;
                changes.push(Change::Axis(k as u8, now));
            }
        }
        (pad.id, added, changes)
    });
    if added {
        let mut e = super::events::event(SDL_EVENT_JOYSTICK_ADDED);
        e.jdevice.which = id;
        super::events::push(e);
    }
    for change in changes {
        match change {
            Change::Button(button, down) => {
                let mut e = super::events::event(if down { SDL_EVENT_JOYSTICK_BUTTON_DOWN } else { SDL_EVENT_JOYSTICK_BUTTON_UP });
                e.jbutton.which = id;
                e.jbutton.button = button;
                e.jbutton.down = down;
                super::events::push(e);
            }
            Change::Axis(axis, value) => {
                let mut e = super::events::event(SDL_EVENT_JOYSTICK_AXIS_MOTION);
                e.jaxis.which = id;
                e.jaxis.axis = axis;
                e.jaxis.value = value;
                super::events::push(e);
            }
        }
    }
}

/// A pad the page no longer has.
pub fn gone(index: u32) {
    let ids: Vec<SDL_JoystickID> = PADS.with(|pads| {
        let mut pads = pads.borrow_mut();
        pads.iter_mut()
            .filter(|p| p.index == index && p.connected)
            .map(|p| {
                p.connected = false;
                p.id
            })
            .collect()
    });
    for id in ids {
        let mut e = super::events::event(SDL_EVENT_JOYSTICK_REMOVED);
        e.jdevice.which = id;
        super::events::push(e);
    }
}

enum Change {
    Button(u8, bool),
    Axis(u8, i16),
}

fn with_pad<R>(id: SDL_JoystickID, f: impl FnOnce(&Pad) -> R) -> Option<R> {
    PADS.with(|pads| pads.borrow().iter().find(|p| p.id == id).map(|p| f(p)))
}

fn pad_of<'a>(handle: *mut SDL_Joystick) -> Option<&'a Pad> {
    // SAFETY: our pads' handles: boxed, never freed while the player runs
    unsafe { handle.cast::<Pad>().as_ref() }
}

fn handle(id: SDL_JoystickID) -> *mut SDL_Joystick {
    PADS.with(|pads| pads.borrow().iter().find(|p| p.id == id).map_or(std::ptr::null_mut(), |p| std::ptr::from_ref::<Pad>(p).cast_mut().cast()))
}

fn connected(gamepads_only: bool) -> Vec<SDL_JoystickID> {
    PADS.with(|pads| pads.borrow().iter().filter(|p| p.connected && (!gamepads_only || p.standard)).map(|p| p.id).collect())
}

fn id_list(ids: Vec<SDL_JoystickID>, count: *mut c_int) -> *mut SDL_JoystickID {
    if !count.is_null() {
        // SAFETY: the caller's out-parameter
        unsafe { *count = ids.len() as c_int };
    }
    let mut list = ids;
    list.push(SDL_JoystickID(0));
    super::c_alloc(&list)
}

/// SDL's GUID for a pad: USB, its vendor and product, as SDL makes one.
fn guid(vendor: u16, product: u16) -> SDL_GUID {
    let mut data = [0u8; 16];
    data[0] = 0x03; // USB
    data[4..6].copy_from_slice(&vendor.to_le_bytes());
    data[8..10].copy_from_slice(&product.to_le_bytes());
    SDL_GUID { data }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetJoysticks(count: *mut c_int) -> *mut SDL_JoystickID {
    id_list(connected(false), count)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetGamepads(count: *mut c_int) -> *mut SDL_JoystickID {
    id_list(connected(true), count)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_IsGamepad(instance_id: SDL_JoystickID) -> bool {
    with_pad(instance_id, |p| p.standard).unwrap_or(false)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_OpenJoystick(instance_id: SDL_JoystickID) -> *mut SDL_Joystick {
    handle(instance_id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetJoystickFromID(instance_id: SDL_JoystickID) -> *mut SDL_Joystick {
    handle(instance_id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_CloseJoystick(_joystick: *mut SDL_Joystick) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_OpenGamepad(instance_id: SDL_JoystickID) -> *mut SDL_Gamepad {
    if with_pad(instance_id, |p| p.standard).unwrap_or(false) { handle(instance_id).cast() } else { std::ptr::null_mut() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetGamepadFromID(instance_id: SDL_JoystickID) -> *mut SDL_Gamepad {
    // SAFETY: as SDL_OpenGamepad
    unsafe { SDL_OpenGamepad(instance_id) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetJoystickName(joystick: *mut SDL_Joystick) -> *const c_char {
    pad_of(joystick).map_or(std::ptr::null(), |p| p.name.as_ptr())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetJoystickNameForID(instance_id: SDL_JoystickID) -> *const c_char {
    // SAFETY: as SDL_GetJoystickName
    unsafe { SDL_GetJoystickName(handle(instance_id)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetJoystickVendorForID(instance_id: SDL_JoystickID) -> Uint16 {
    with_pad(instance_id, |p| p.vendor).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetJoystickProductForID(instance_id: SDL_JoystickID) -> Uint16 {
    with_pad(instance_id, |p| p.product).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetJoystickGUID(joystick: *mut SDL_Joystick) -> SDL_GUID {
    pad_of(joystick).map_or(SDL_GUID { data: [0; 16] }, |p| guid(p.vendor, p.product))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetJoystickGUIDForID(instance_id: SDL_JoystickID) -> SDL_GUID {
    with_pad(instance_id, |p| guid(p.vendor, p.product)).unwrap_or(SDL_GUID { data: [0; 16] })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetNumJoystickAxes(joystick: *mut SDL_Joystick) -> c_int {
    pad_of(joystick).map_or(0, |p| p.axes.len() as c_int)
}

/// Where a standard pad's gamepad buttons and axes are among its joystick's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetGamepadBindings(gamepad: *mut SDL_Gamepad, count: *mut c_int) -> *mut *mut SDL_GamepadBinding {
    let Some(_pad) = pad_of(gamepad.cast()).filter(|p| p.standard) else {
        if !count.is_null() {
            // SAFETY: the caller's out-parameter
            unsafe { *count = 0 };
        }
        return std::ptr::null_mut();
    };
    let buttons = [
        (0, SDL_GAMEPAD_BUTTON_SOUTH),
        (1, SDL_GAMEPAD_BUTTON_EAST),
        (2, SDL_GAMEPAD_BUTTON_WEST),
        (3, SDL_GAMEPAD_BUTTON_NORTH),
        (4, SDL_GAMEPAD_BUTTON_LEFT_SHOULDER),
        (5, SDL_GAMEPAD_BUTTON_RIGHT_SHOULDER),
        (8, SDL_GAMEPAD_BUTTON_BACK),
        (9, SDL_GAMEPAD_BUTTON_START),
        (10, SDL_GAMEPAD_BUTTON_LEFT_STICK),
        (11, SDL_GAMEPAD_BUTTON_RIGHT_STICK),
        (12, SDL_GAMEPAD_BUTTON_DPAD_UP),
        (13, SDL_GAMEPAD_BUTTON_DPAD_DOWN),
        (14, SDL_GAMEPAD_BUTTON_DPAD_LEFT),
        (15, SDL_GAMEPAD_BUTTON_DPAD_RIGHT),
        (16, SDL_GAMEPAD_BUTTON_GUIDE),
    ];
    let axes = [
        (0, SDL_GAMEPAD_AXIS_LEFTX, -32768),
        (1, SDL_GAMEPAD_AXIS_LEFTY, -32768),
        (2, SDL_GAMEPAD_AXIS_RIGHTX, -32768),
        (3, SDL_GAMEPAD_AXIS_RIGHTY, -32768),
        (4, SDL_GAMEPAD_AXIS_LEFT_TRIGGER, 0),
        (5, SDL_GAMEPAD_AXIS_RIGHT_TRIGGER, 0),
    ];
    // SAFETY: plain data, every field set
    let mut list: Vec<SDL_GamepadBinding> = Vec::new();
    for (input, output) in buttons {
        let mut b: SDL_GamepadBinding = unsafe { std::mem::zeroed() };
        b.input_type = SDL_GAMEPAD_BINDTYPE_BUTTON;
        b.input.button = input;
        b.output_type = SDL_GAMEPAD_BINDTYPE_BUTTON;
        b.output.button = output;
        list.push(b);
    }
    for (input, output, min) in axes {
        let mut b: SDL_GamepadBinding = unsafe { std::mem::zeroed() };
        b.input_type = SDL_GAMEPAD_BINDTYPE_AXIS;
        b.input.axis.axis = input;
        b.input.axis.axis_min = -32768;
        b.input.axis.axis_max = 32767;
        b.output_type = SDL_GAMEPAD_BINDTYPE_AXIS;
        b.output.axis.axis = output;
        b.output.axis.axis_min = min;
        b.output.axis.axis_max = 32767;
        list.push(b);
    }
    // one block, as SDL makes it: the pointers, then the bindings they point at
    let n = list.len();
    let pointers = n * size_of::<*mut SDL_GamepadBinding>();
    // SAFETY: room for both, written before it's handed out (freed with SDL_free)
    unsafe {
        let block = super::SDL_malloc(pointers + n * size_of::<SDL_GamepadBinding>()).cast::<u8>();
        if block.is_null() {
            return std::ptr::null_mut();
        }
        let table = block.cast::<*mut SDL_GamepadBinding>();
        let items = block.add(pointers).cast::<SDL_GamepadBinding>();
        for (k, b) in list.into_iter().enumerate() {
            items.add(k).write(b);
            table.add(k).write(items.add(k));
        }
        if !count.is_null() {
            *count = n as c_int;
        }
        table
    }
}

/// What the face buttons are called on the pad: PlayStation's shapes for Sony's, Nintendo's
/// letters (A on the right) for Nintendo's, else Xbox's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetGamepadButtonLabel(gamepad: *mut SDL_Gamepad, button: SDL_GamepadButton) -> SDL_GamepadButtonLabel {
    let vendor = pad_of(gamepad.cast()).map_or(0, |p| p.vendor);
    let face = [SDL_GAMEPAD_BUTTON_SOUTH, SDL_GAMEPAD_BUTTON_EAST, SDL_GAMEPAD_BUTTON_WEST, SDL_GAMEPAD_BUTTON_NORTH];
    let Some(k) = face.iter().position(|&b| b == button) else { return SDL_GAMEPAD_BUTTON_LABEL_UNKNOWN };
    let labels = match vendor {
        0x054c => [SDL_GAMEPAD_BUTTON_LABEL_CROSS, SDL_GAMEPAD_BUTTON_LABEL_CIRCLE, SDL_GAMEPAD_BUTTON_LABEL_SQUARE, SDL_GAMEPAD_BUTTON_LABEL_TRIANGLE],
        0x057e => [SDL_GAMEPAD_BUTTON_LABEL_B, SDL_GAMEPAD_BUTTON_LABEL_A, SDL_GAMEPAD_BUTTON_LABEL_Y, SDL_GAMEPAD_BUTTON_LABEL_X],
        _ => [SDL_GAMEPAD_BUTTON_LABEL_A, SDL_GAMEPAD_BUTTON_LABEL_B, SDL_GAMEPAD_BUTTON_LABEL_X, SDL_GAMEPAD_BUTTON_LABEL_Y],
    };
    labels[k]
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_RumbleGamepad(gamepad: *mut SDL_Gamepad, low_frequency_rumble: Uint16, high_frequency_rumble: Uint16, duration_ms: Uint32) -> bool {
    let Some(p) = pad_of(gamepad.cast()).filter(|p| p.connected) else { return false };
    // SAFETY: a plain call into the page
    unsafe { page_rumble(p.index, f32::from(low_frequency_rumble) / 65535.0, f32::from(high_frequency_rumble) / 65535.0, duration_ms.min(5000)) };
    true
}
