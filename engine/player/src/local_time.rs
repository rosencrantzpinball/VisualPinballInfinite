//! The local and universal date and time on Linux, from the C library (`localtime_r`,
//! `gmtime_r`), where Windows has `GetLocalTime` and `GetSystemTime`. Ours.

use std::ffi::c_char;

/// The C library's `struct tm` (glibc's, with its two extra fields).
#[repr(C)]
pub struct Tm {
    pub sec: i32,
    pub min: i32,
    pub hour: i32,
    /// 1 to 31.
    pub mday: i32,
    /// 0 to 11.
    pub mon: i32,
    /// Years since 1900.
    pub year: i32,
    pub wday: i32,
    pub yday: i32,
    pub isdst: i32,
    pub gmtoff: i64,
    pub zone: *const c_char,
}

unsafe extern "C" {
    fn time(t: *mut i64) -> i64;
    fn localtime_r(t: *const i64, tm: *mut Tm) -> *mut Tm;
    fn gmtime_r(t: *const i64, tm: *mut Tm) -> *mut Tm;
}

fn now(convert: unsafe extern "C" fn(*const i64, *mut Tm) -> *mut Tm) -> Tm {
    // SAFETY: plain C library calls filling the structure they're given
    unsafe {
        let t = time(std::ptr::null_mut());
        let mut tm: Tm = std::mem::zeroed();
        convert(&t, &mut tm);
        tm
    }
}

/// Now, in the local time zone.
pub fn local() -> Tm {
    now(localtime_r)
}

/// Now, in universal time.
pub fn utc() -> Tm {
    now(gmtime_r)
}
