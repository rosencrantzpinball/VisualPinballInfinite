//! SDL's property groups: values by name, kept here.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_float, c_void};

use sdl3_sys::everything::*;

enum Value {
    Number(i64),
    Float(f32),
    Boolean(bool),
    Text(CString),
    Pointer(*mut c_void),
}

thread_local! {
    static GROUPS: RefCell<(u32, HashMap<u32, HashMap<String, Value>>)> = RefCell::new((0, HashMap::new()));
}

fn name(n: *const c_char) -> String {
    if n.is_null() {
        return String::new();
    }
    // SAFETY: SDL's property names are NUL-terminated
    unsafe { CStr::from_ptr(n) }.to_string_lossy().into_owned()
}

fn set(props: SDL_PropertiesID, n: *const c_char, value: Value) -> bool {
    GROUPS.with(|g| match g.borrow_mut().1.get_mut(&props.0) {
        Some(group) => {
            group.insert(name(n), value);
            true
        }
        None => false,
    })
}

fn get<R>(props: SDL_PropertiesID, n: *const c_char, f: impl FnOnce(Option<&Value>) -> R) -> R {
    GROUPS.with(|g| f(g.borrow().1.get(&props.0).and_then(|group| group.get(&name(n)))))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_CreateProperties() -> SDL_PropertiesID {
    GROUPS.with(|g| {
        let mut g = g.borrow_mut();
        g.0 += 1;
        let id = g.0;
        g.1.insert(id, HashMap::new());
        SDL_PropertiesID(id)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_DestroyProperties(props: SDL_PropertiesID) {
    GROUPS.with(|g| g.borrow_mut().1.remove(&props.0));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_SetNumberProperty(props: SDL_PropertiesID, name: *const c_char, value: Sint64) -> bool {
    set(props, name, Value::Number(value))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_SetFloatProperty(props: SDL_PropertiesID, name: *const c_char, value: c_float) -> bool {
    set(props, name, Value::Float(value))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_SetBooleanProperty(props: SDL_PropertiesID, name: *const c_char, value: bool) -> bool {
    set(props, name, Value::Boolean(value))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_SetStringProperty(props: SDL_PropertiesID, name: *const c_char, value: *const c_char) -> bool {
    if value.is_null() {
        return GROUPS.with(|g| g.borrow_mut().1.get_mut(&props.0).map(|group| group.remove(&self::name(name))).is_some());
    }
    // SAFETY: a NUL-terminated string, copied
    let text = unsafe { CStr::from_ptr(value) }.to_owned();
    set(props, name, Value::Text(text))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_SetPointerProperty(props: SDL_PropertiesID, name: *const c_char, value: *mut c_void) -> bool {
    set(props, name, Value::Pointer(value))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetNumberProperty(props: SDL_PropertiesID, name: *const c_char, default_value: Sint64) -> Sint64 {
    get(props, name, |v| match v {
        Some(Value::Number(n)) => *n,
        Some(Value::Float(f)) => *f as i64,
        Some(Value::Boolean(b)) => i64::from(*b),
        _ => default_value,
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetFloatProperty(props: SDL_PropertiesID, name: *const c_char, default_value: c_float) -> c_float {
    get(props, name, |v| match v {
        Some(Value::Float(f)) => *f,
        Some(Value::Number(n)) => *n as f32,
        _ => default_value,
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetBooleanProperty(props: SDL_PropertiesID, name: *const c_char, default_value: bool) -> bool {
    get(props, name, |v| match v {
        Some(Value::Boolean(b)) => *b,
        Some(Value::Number(n)) => *n != 0,
        _ => default_value,
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetStringProperty(props: SDL_PropertiesID, name: *const c_char, default_value: *const c_char) -> *const c_char {
    // the text stays where it is until the property is set again
    get(props, name, |v| match v {
        Some(Value::Text(t)) => t.as_ptr(),
        _ => default_value,
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetPointerProperty(props: SDL_PropertiesID, name: *const c_char, default_value: *mut c_void) -> *mut c_void {
    get(props, name, |v| match v {
        Some(Value::Pointer(p)) => *p,
        _ => default_value,
    })
}
