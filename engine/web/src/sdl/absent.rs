//! What the program reaches for that the browser hasn't: the shared memory the cabinet's helper
//! programs link through (PinMAME, colour, DOF, DMD), which never start here. Each answers as a
//! failure would.

use std::ffi::{c_char, c_int, c_void};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn shm_unlink(_name: *const c_char) -> c_int {
    -1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn munmap(_address: *mut c_void, _length: usize) -> c_int {
    -1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sem_close(_semaphore: *mut c_void) -> c_int {
    -1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sem_unlink(_name: *const c_char) -> c_int {
    -1
}
