// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026, the Visual Pinball Infinite contributors

//! The Windows calls the link makes (`win.rs`), done with POSIX on Linux (the
//! Steam Frame), under the same names so the link reads the same: a named
//! mapping is shared memory (`shm_open`), a named event a named semaphore
//! (`sem_open`) that wakes one waiter, and the high-resolution clock is the
//! monotonic one. Names become POSIX ones: a `/`, then the name with its
//! slashes and backslashes as `_` (`Local\vpi-…` → `/Local_vpi-…`). What the
//! engine creates is unlinked when it closes it, as Windows drops a mapping
//! once its last handle closes. Declared from the POSIX and glibc headers.

#![allow(non_snake_case, dead_code, clippy::missing_safety_doc)]

use std::collections::HashMap;
use std::ffi::{CString, c_char, c_int, c_long, c_uint, c_void};
use std::sync::Mutex;

/// A mapping or an event: a box of ours, made by the `Create…`/`Open…` calls and freed by `CloseHandle`.
pub type Handle = *mut c_void;

pub const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
pub const PAGE_READWRITE: u32 = 0x04;
pub const FILE_MAP_ALL_ACCESS: u32 = 0x000F_001F;
pub const EVENT_MODIFY_STATE: u32 = 0x0002;
pub const SYNCHRONIZE: u32 = 0x0010_0000;
/// What `io::Error::last_os_error` reads after creating a mapping that was there already: `EEXIST`.
pub const ERROR_ALREADY_EXISTS: i32 = EEXIST;
pub const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 0x102;
const INFINITE: u32 = 0xFFFF_FFFF;

const O_RDWR: c_int = 0o2;
const O_CREAT: c_int = 0o100;
const O_EXCL: c_int = 0o200;
const PROT_READ: c_int = 1;
const PROT_WRITE: c_int = 2;
const MAP_SHARED: c_int = 1;
const EEXIST: c_int = 17;
const EINTR: c_int = 4;
const CLOCK_REALTIME: c_int = 0;
const CLOCK_MONOTONIC: c_int = 1;

#[repr(C)]
struct Timespec {
    sec: i64,
    nsec: c_long,
}

unsafe extern "C" {
    fn shm_open(name: *const c_char, flags: c_int, mode: c_uint) -> c_int;
    fn shm_unlink(name: *const c_char) -> c_int;
    fn ftruncate(fd: c_int, length: i64) -> c_int;
    fn close(fd: c_int) -> c_int;
    fn mmap(address: *mut c_void, length: usize, protection: c_int, flags: c_int, fd: c_int, offset: i64) -> *mut c_void;
    fn munmap(address: *mut c_void, length: usize) -> c_int;
    fn sem_open(name: *const c_char, flags: c_int, ...) -> *mut c_void;
    fn sem_close(semaphore: *mut c_void) -> c_int;
    fn sem_unlink(name: *const c_char) -> c_int;
    fn sem_post(semaphore: *mut c_void) -> c_int;
    fn sem_getvalue(semaphore: *mut c_void, value: *mut c_int) -> c_int;
    fn sem_wait(semaphore: *mut c_void) -> c_int;
    fn sem_timedwait(semaphore: *mut c_void, until: *const Timespec) -> c_int;
    fn clock_gettime(clock: c_int, time: *mut Timespec) -> c_int;
    fn __errno_location() -> *mut c_int;
}

enum Object {
    Mapping { fd: c_int, size: usize },
    Event { semaphore: *mut c_void },
}

struct Named {
    object: Object,
    /// The POSIX name, unlinked on close when this end created it.
    name: CString,
    created: bool,
}

/// Each view's length, for `UnmapViewOfFile`, which is given its address alone.
static VIEWS: Mutex<Option<HashMap<usize, usize>>> = Mutex::new(None);

fn posix_name(name: *const u16) -> CString {
    // SAFETY: the callers pass NUL-terminated wide strings (`wide`)
    let wide = unsafe { std::slice::from_raw_parts(name, (0..).take_while(|&i| *name.add(i) != 0).count()) };
    let name: String = String::from_utf16_lossy(wide).chars().map(|c| if c == '/' || c == '\\' { '_' } else { c }).collect();
    CString::new(format!("/{name}")).unwrap_or_default()
}

fn handle(named: Named) -> Handle {
    Box::into_raw(Box::new(named)).cast()
}

fn set_errno(value: c_int) {
    // SAFETY: the calling thread's own errno
    unsafe { *__errno_location() = value };
}

pub unsafe fn CreateFileMappingW(_file: Handle, _attributes: *const c_void, _protect: u32, size_high: u32, size_low: u32, name: *const u16) -> Handle {
    let name = posix_name(name);
    let size = ((u64::from(size_high) << 32) | u64::from(size_low)) as usize;
    unsafe {
        let fd = shm_open(name.as_ptr(), O_RDWR | O_CREAT | O_EXCL, 0o600);
        if fd >= 0 {
            if ftruncate(fd, size as i64) != 0 {
                close(fd);
                shm_unlink(name.as_ptr());
                return std::ptr::null_mut();
            }
            set_errno(0);
            return handle(Named { object: Object::Mapping { fd, size }, name, created: true });
        }
        // there already, as Windows hands it back with ERROR_ALREADY_EXISTS
        let fd = shm_open(name.as_ptr(), O_RDWR, 0);
        if fd < 0 {
            return std::ptr::null_mut();
        }
        set_errno(EEXIST);
        handle(Named { object: Object::Mapping { fd, size }, name, created: false })
    }
}

pub unsafe fn OpenFileMappingW(_access: u32, _inherit: i32, name: *const u16) -> Handle {
    let name = posix_name(name);
    let fd = unsafe { shm_open(name.as_ptr(), O_RDWR, 0) };
    if fd < 0 {
        return std::ptr::null_mut();
    }
    handle(Named { object: Object::Mapping { fd, size: 0 }, name, created: false })
}

pub unsafe fn MapViewOfFile(mapping: Handle, _access: u32, _offset_high: u32, _offset_low: u32, bytes: usize) -> *mut c_void {
    let Some(named) = (unsafe { mapping.cast::<Named>().as_ref() }) else { return std::ptr::null_mut() };
    let Object::Mapping { fd, size } = named.object else { return std::ptr::null_mut() };
    let length = if bytes == 0 { size } else { bytes };
    let view = unsafe { mmap(std::ptr::null_mut(), length, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0) };
    if view as isize == -1 {
        return std::ptr::null_mut();
    }
    VIEWS.lock().unwrap().get_or_insert_with(HashMap::new).insert(view as usize, length);
    view
}

pub unsafe fn UnmapViewOfFile(address: *const c_void) -> i32 {
    let length = VIEWS.lock().unwrap().as_mut().and_then(|v| v.remove(&(address as usize)));
    match length {
        Some(length) => i32::from(unsafe { munmap(address.cast_mut(), length) } == 0),
        None => 0,
    }
}

/// An auto-reset event (`manual_reset` 0, not set at first): a named semaphore.
pub unsafe fn CreateEventW(_attributes: *const c_void, _manual_reset: i32, _initial_state: i32, name: *const u16) -> Handle {
    let name = posix_name(name);
    unsafe {
        sem_unlink(name.as_ptr()); // a leftover of a program that ended without closing its own
        let semaphore = sem_open(name.as_ptr(), O_CREAT | O_EXCL, 0o600 as c_uint, 0 as c_uint);
        if semaphore.is_null() {
            return std::ptr::null_mut();
        }
        handle(Named { object: Object::Event { semaphore }, name, created: true })
    }
}

pub unsafe fn OpenEventW(_access: u32, _inherit: i32, name: *const u16) -> Handle {
    let name = posix_name(name);
    let semaphore = unsafe { sem_open(name.as_ptr(), 0) };
    if semaphore.is_null() {
        return std::ptr::null_mut();
    }
    handle(Named { object: Object::Event { semaphore }, name, created: false })
}

/// Sets the event: wakes one waiter, or the next one to wait. Like an event, setting it twice
/// before anyone waits wakes once.
pub unsafe fn SetEvent(event: Handle) -> i32 {
    let Some(Named { object: Object::Event { semaphore }, .. }) = (unsafe { event.cast::<Named>().as_ref() }) else { return 0 };
    unsafe {
        let mut value = 0;
        if sem_getvalue(*semaphore, &mut value) == 0 && value > 0 {
            return 1;
        }
        i32::from(sem_post(*semaphore) == 0)
    }
}

pub unsafe fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32 {
    let Some(Named { object: Object::Event { semaphore }, .. }) = (unsafe { handle.cast::<Named>().as_ref() }) else { return WAIT_TIMEOUT };
    unsafe {
        if milliseconds == INFINITE {
            while sem_wait(*semaphore) != 0 {
                if *__errno_location() != EINTR {
                    return WAIT_TIMEOUT;
                }
            }
            return WAIT_OBJECT_0;
        }
        let mut until = Timespec { sec: 0, nsec: 0 };
        clock_gettime(CLOCK_REALTIME, &mut until);
        let nanos = until.nsec as i64 + i64::from(milliseconds % 1000) * 1_000_000;
        until.sec += i64::from(milliseconds / 1000) + nanos / 1_000_000_000;
        until.nsec = (nanos % 1_000_000_000) as c_long;
        loop {
            if sem_timedwait(*semaphore, &until) == 0 {
                return WAIT_OBJECT_0;
            }
            if *__errno_location() != EINTR {
                return WAIT_TIMEOUT;
            }
        }
    }
}

pub unsafe fn CloseHandle(handle: Handle) -> i32 {
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return 0;
    }
    // SAFETY: made by `handle` and closed once
    let named = unsafe { Box::from_raw(handle.cast::<Named>()) };
    unsafe {
        match named.object {
            Object::Mapping { fd, .. } => {
                close(fd);
                if named.created {
                    shm_unlink(named.name.as_ptr());
                }
            }
            Object::Event { semaphore } => {
                sem_close(semaphore);
                if named.created {
                    sem_unlink(named.name.as_ptr());
                }
            }
        }
    }
    1
}

/// The monotonic clock, in nanoseconds.
pub unsafe fn QueryPerformanceCounter(count: *mut i64) -> i32 {
    let mut now = Timespec { sec: 0, nsec: 0 };
    unsafe {
        clock_gettime(CLOCK_MONOTONIC, &mut now);
        *count = now.sec * 1_000_000_000 + now.nsec as i64;
    }
    1
}

pub unsafe fn QueryPerformanceFrequency(frequency: *mut i64) -> i32 {
    unsafe { *frequency = 1_000_000_000 };
    1
}

/// `s` as a null-terminated UTF-16 string.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
