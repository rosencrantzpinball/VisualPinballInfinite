//! libwinevbs, Wine's VBScript engine as a library, loaded as its own
//! DLL (LGPL-2.1-or-later, so it stays a separate, replaceable file);
//! `libwinevbs.so` on Linux, built the same way (`tools/libwinevbs/setup.sh`).
//!
//! libwinevbs carries its own copy of COM and OLE Automation, so every string
//! and VARIANT that passes through it is made, copied and freed with its
//! functions, never Windows' own.

#![allow(non_snake_case)]

use std::ffi::{c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::com::*;

type CoCreateInstanceFn = unsafe extern "system" fn(*const GUID, *mut c_void, u32, *const GUID, *mut *mut c_void) -> HRESULT;
type SysAllocStringLenFn = unsafe extern "system" fn(*const OLECHAR, u32) -> *mut OLECHAR;
type SysFreeStringFn = unsafe extern "system" fn(*mut OLECHAR);
type SysStringLenFn = unsafe extern "system" fn(*mut OLECHAR) -> u32;
type VariantInitFn = unsafe extern "system" fn(*mut VARIANT);
type VariantClearFn = unsafe extern "system" fn(*mut VARIANT) -> HRESULT;
type VariantCopyFn = unsafe extern "system" fn(*mut VARIANT, *const VARIANT) -> HRESULT;
type VariantChangeTypeFn = unsafe extern "system" fn(*mut VARIANT, *const VARIANT, u16, u16) -> HRESULT;
type SafeArrayCreateVectorFn = unsafe extern "system" fn(u16, i32, u32) -> *mut c_void;
type SafeArrayPutElementFn = unsafe extern "system" fn(*mut c_void, *const i32, *mut c_void) -> HRESULT;
type SafeArrayGetElementFn = unsafe extern "system" fn(*mut c_void, *const i32, *mut c_void) -> HRESULT;
type SafeArrayBoundFn = unsafe extern "system" fn(*mut c_void, u32, *mut i32) -> HRESULT;
type SafeArrayGetDimFn = unsafe extern "system" fn(*mut c_void) -> u32;
type SafeArrayCreateFn = unsafe extern "system" fn(u16, u32, *const SAFEARRAYBOUND) -> *mut c_void;
type SafeArrayAccessDataFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT;
type SafeArrayUnaccessDataFn = unsafe extern "system" fn(*mut c_void) -> HRESULT;

/// What libwinevbs asks of its host (`winevbs_callbacks` in `libwinevbs.h`).
#[repr(C)]
pub struct Callbacks {
    pub log: unsafe extern "C" fn(level: i32, format: *const c_char, args: *mut c_void),
    pub create_object: unsafe extern "C" fn(progid: *const OLECHAR, factory: *mut c_void, object: *mut *mut c_void) -> HRESULT,
}

type InitFn = unsafe extern "C" fn(*const Callbacks);

/// Asked about every path the file object touches (`libwinevbs_file_fence_t`).
pub type FileFenceFn = unsafe extern "C" fn(path: *const OLECHAR, write: i32) -> i32;
/// `libwinevbs_set_file_fence`, in our build only (`tools/libwinevbs`).
pub type SetFileFenceFn = unsafe extern "C" fn(Option<FileFenceFn>, *const OLECHAR);

/// The library's functions.
pub struct WineVbs {
    pub co_create_instance: CoCreateInstanceFn,
    pub sys_alloc_string_len: SysAllocStringLenFn,
    pub sys_free_string: SysFreeStringFn,
    pub sys_string_len: SysStringLenFn,
    pub variant_init: VariantInitFn,
    pub variant_clear: VariantClearFn,
    pub variant_copy: VariantCopyFn,
    pub variant_copy_ind: VariantCopyFn,
    pub variant_change_type: VariantChangeTypeFn,
    pub safe_array_create_vector: SafeArrayCreateVectorFn,
    pub safe_array_put_element: SafeArrayPutElementFn,
    pub safe_array_get_element: SafeArrayGetElementFn,
    pub safe_array_get_lbound: SafeArrayBoundFn,
    pub safe_array_get_ubound: SafeArrayBoundFn,
    pub safe_array_get_dim: SafeArrayGetDimFn,
    pub safe_array_create: SafeArrayCreateFn,
    pub safe_array_access_data: SafeArrayAccessDataFn,
    pub safe_array_unaccess_data: SafeArrayUnaccessDataFn,
    /// The file object's fence; `None` in a libwinevbs that isn't ours.
    pub set_file_fence: Option<SetFileFenceFn>,
}

// SAFETY: plain function pointers into a DLL that stays loaded
unsafe impl Send for WineVbs {}
unsafe impl Sync for WineVbs {}

static LIBRARY: OnceLock<Result<WineVbs, String>> = OnceLock::new();

#[cfg(windows)]
mod os {
    use std::ffi::{c_char, c_void};
    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        pub fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    }
    /// Finds the DLL's own dependencies next to it.
    const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x8;

    pub fn open(path: &std::path::Path) -> *mut c_void {
        let wide: Vec<u16> = path.to_string_lossy().encode_utf16().chain(Some(0)).collect();
        // SAFETY: a NUL-terminated path
        unsafe { LoadLibraryExW(wide.as_ptr(), std::ptr::null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH) }
    }

    /// # Safety
    /// `module` is a loaded library; `name` is NUL-terminated.
    pub unsafe fn symbol(module: *mut c_void, name: &str) -> *mut c_void {
        unsafe { GetProcAddress(module, name.as_ptr()) }
    }

    unsafe extern "C" {
        /// The C runtime's formatter behind every `printf` (UCRT).
        fn __stdio_common_vsprintf(options: u64, buffer: *mut c_char, count: usize, format: *const c_char, locale: *mut c_void, args: *mut c_void) -> i32;
    }

    /// # Safety
    /// `format` and `args` are a `printf` format and its `va_list`.
    pub unsafe fn format(buffer: &mut [c_char], format: *const c_char, args: *mut c_void) -> i32 {
        unsafe { __stdio_common_vsprintf(2, buffer.as_mut_ptr(), buffer.len(), format, std::ptr::null_mut(), args) }
    }
}

#[cfg(all(not(windows), not(target_family = "wasm")))]
mod os {
    use std::ffi::{CString, c_char, c_int, c_void};
    use std::os::unix::ffi::OsStrExt;
    unsafe extern "C" {
        fn dlopen(file: *const c_char, mode: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
        fn vsnprintf(buffer: *mut c_char, count: usize, format: *const c_char, args: *mut c_void) -> c_int;
    }
    /// Every symbol bound at once, the library's own kept to itself.
    const RTLD_NOW: c_int = 2;

    pub fn open(path: &std::path::Path) -> *mut c_void {
        let Ok(path) = CString::new(path.as_os_str().as_bytes()) else { return std::ptr::null_mut() };
        // SAFETY: a NUL-terminated path
        unsafe { dlopen(path.as_ptr(), RTLD_NOW) }
    }

    /// # Safety
    /// `module` is a loaded library; `name` is NUL-terminated.
    pub unsafe fn symbol(module: *mut c_void, name: &str) -> *mut c_void {
        unsafe { dlsym(module, name.as_ptr().cast()) }
    }

    /// # Safety
    /// `format` and `args` are a `printf` format and its `va_list`, which C passes on as a
    /// pointer here (to a copy on arm64, where it's a structure larger than 16 bytes; the array
    /// itself on x86-64), so it's handed on as it came.
    pub unsafe fn format(buffer: &mut [c_char], format: *const c_char, args: *mut c_void) -> i32 {
        unsafe { vsnprintf(buffer.as_mut_ptr(), buffer.len(), format, args) }
    }
}

/// In the browser (engine/web) libwinevbs is built into the program (`tools/web/libwinevbs.sh`), not
/// loaded: its exports are found by name among the functions linked in. Declared with their own
/// types: WebAssembly checks every call's type against the function's.
#[cfg(target_family = "wasm")]
mod os {
    use std::ffi::{c_char, c_int, c_void};

    use super::{Callbacks, FileFenceFn, GUID, HRESULT, OLECHAR, SAFEARRAYBOUND, VARIANT};
    unsafe extern "C" {
        fn CoCreateInstance(clsid: *const GUID, outer: *mut c_void, context: u32, iid: *const GUID, object: *mut *mut c_void) -> HRESULT;
        fn SysAllocStringLen(text: *const OLECHAR, len: u32) -> *mut OLECHAR;
        fn SysFreeString(text: *mut OLECHAR);
        fn SysStringLen(text: *mut OLECHAR) -> u32;
        fn VariantInit(v: *mut VARIANT);
        fn VariantClear(v: *mut VARIANT) -> HRESULT;
        fn VariantCopy(to: *mut VARIANT, from: *const VARIANT) -> HRESULT;
        fn VariantCopyInd(to: *mut VARIANT, from: *const VARIANT) -> HRESULT;
        fn VariantChangeType(to: *mut VARIANT, from: *const VARIANT, flags: u16, vt: u16) -> HRESULT;
        fn SafeArrayCreateVector(vt: u16, lower: i32, count: u32) -> *mut c_void;
        fn SafeArrayPutElement(array: *mut c_void, at: *const i32, value: *mut c_void) -> HRESULT;
        fn SafeArrayGetElement(array: *mut c_void, at: *const i32, value: *mut c_void) -> HRESULT;
        fn SafeArrayGetLBound(array: *mut c_void, dim: u32, bound: *mut i32) -> HRESULT;
        fn SafeArrayGetUBound(array: *mut c_void, dim: u32, bound: *mut i32) -> HRESULT;
        fn SafeArrayGetDim(array: *mut c_void) -> u32;
        fn SafeArrayCreate(vt: u16, dims: u32, bounds: *const SAFEARRAYBOUND) -> *mut c_void;
        fn SafeArrayAccessData(array: *mut c_void, data: *mut *mut c_void) -> HRESULT;
        fn SafeArrayUnaccessData(array: *mut c_void) -> HRESULT;
        fn libwinevbs_set_file_fence(fence: Option<FileFenceFn>, root: *const OLECHAR);
        fn libwinevbs_init(callbacks: *const Callbacks);
        fn vsnprintf(buffer: *mut c_char, count: usize, format: *const c_char, args: *mut c_void) -> c_int;
    }

    /// The library, always there.
    pub fn open(_path: &std::path::Path) -> *mut c_void {
        std::ptr::NonNull::<c_void>::dangling().as_ptr()
    }

    /// # Safety
    /// `name` is NUL-terminated.
    pub unsafe fn symbol(_module: *mut c_void, name: &str) -> *mut c_void {
        let f: *const () = match name.trim_end_matches('\0') {
            "CoCreateInstance" => CoCreateInstance as *const (),
            "SysAllocStringLen" => SysAllocStringLen as *const (),
            "SysFreeString" => SysFreeString as *const (),
            "SysStringLen" => SysStringLen as *const (),
            "VariantInit" => VariantInit as *const (),
            "VariantClear" => VariantClear as *const (),
            "VariantCopy" => VariantCopy as *const (),
            "VariantCopyInd" => VariantCopyInd as *const (),
            "VariantChangeType" => VariantChangeType as *const (),
            "SafeArrayCreateVector" => SafeArrayCreateVector as *const (),
            "SafeArrayPutElement" => SafeArrayPutElement as *const (),
            "SafeArrayGetElement" => SafeArrayGetElement as *const (),
            "SafeArrayGetLBound" => SafeArrayGetLBound as *const (),
            "SafeArrayGetUBound" => SafeArrayGetUBound as *const (),
            "SafeArrayGetDim" => SafeArrayGetDim as *const (),
            "SafeArrayCreate" => SafeArrayCreate as *const (),
            "SafeArrayAccessData" => SafeArrayAccessData as *const (),
            "SafeArrayUnaccessData" => SafeArrayUnaccessData as *const (),
            "libwinevbs_set_file_fence" => libwinevbs_set_file_fence as *const (),
            "libwinevbs_init" => libwinevbs_init as *const (),
            _ => std::ptr::null(),
        };
        f.cast_mut().cast()
    }

    /// # Safety
    /// `format` and `args` are a `printf` format and its `va_list`, a pointer to the arguments in
    /// WebAssembly's C.
    pub unsafe fn format(buffer: &mut [c_char], format: *const c_char, args: *mut c_void) -> i32 {
        unsafe { vsnprintf(buffer.as_mut_ptr(), buffer.len(), format, args) }
    }
}

/// Where to look for `winevbs64.dll`: next to the program (how it ships),
/// then, in the repository (development builds), our own build with our
/// patches (`tools/libwinevbs`, built into `vpx-build/libwinevbs/build`), then
/// the copy from VPX's automated build (`vpx-build/winevbs`).
#[cfg(windows)]
fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        out.push(dir.join("winevbs64.dll"));
        // target/release/vpi-player.exe -> repository root
        let roots: Vec<&Path> = if crate::DEVELOPMENT { dir.ancestors().take(4).collect() } else { Vec::new() };
        for up in &roots {
            out.push(up.join("vpx-build").join("libwinevbs").join("build").join("winevbs64.dll"));
        }
        for up in &roots {
            out.push(up.join("vpx-build").join("winevbs").join("winevbs64.dll"));
        }
    }
    out
}

/// Where to look for `libwinevbs.so` on Linux: next to the program (how a package carries it),
/// then, in a development build, our own build (`tools/libwinevbs/setup.sh`, into
/// `vpx-build/linux-aarch64/libwinevbs/build`).
#[cfg(all(not(windows), not(target_family = "wasm")))]
fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        out.push(dir.join("libwinevbs.so.0"));
        // target/<target>/release/vpi-player -> repository root
        if crate::DEVELOPMENT {
            for up in dir.ancestors().take(4) {
                out.push(up.join("vpx-build").join("linux-aarch64").join("libwinevbs").join("build").join("libwinevbs.so.0"));
            }
        }
    }
    out
}

/// In the browser, the library built in.
#[cfg(target_family = "wasm")]
fn candidates() -> Vec<PathBuf> {
    vec![PathBuf::from("libwinevbs (built in)")]
}

unsafe extern "C" fn log(level: i32, format: *const c_char, args: *mut c_void) {
    // SAFETY: libwinevbs passes a printf format and its arguments
    let text = unsafe {
        let mut buf = [0 as c_char; 1024];
        let n = os::format(&mut buf, format, args);
        if n < 0 && format.is_null() {
            return;
        }
        std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    };
    super::log::engine_message(level, text.trim_end());
}

unsafe extern "C" fn create_object(progid: *const OLECHAR, factory: *mut c_void, object: *mut *mut c_void) -> HRESULT {
    // SAFETY: libwinevbs passes the script's ProgID, its own factory for
    // objects it has built in (or null), and where the object goes
    unsafe {
        let name = from_wide(progid);
        super::fence::create_object(&name, factory, object)
    }
}

static CALLBACKS: Callbacks = Callbacks { log, create_object };

#[allow(clippy::missing_transmute_annotations)] // each pointer becomes its field's function type
fn load_from(path: &Path) -> Result<WineVbs, String> {
    // SAFETY: loading a library and looking up its exports by name
    unsafe {
        let module = os::open(path);
        if module.is_null() {
            return Err(format!("could not load {}", path.display()));
        }
        let get = |name: &str| -> Result<*mut c_void, String> {
            let c = format!("{name}\0");
            let p = os::symbol(module, &c);
            if p.is_null() { Err(format!("{} has no {name}", path.display())) } else { Ok(p) }
        };
        macro_rules! f {
            ($name:literal) => {
                std::mem::transmute::<*mut c_void, _>(get($name)?)
            };
        }
        let lib = WineVbs {
            co_create_instance: f!("CoCreateInstance"),
            sys_alloc_string_len: f!("SysAllocStringLen"),
            sys_free_string: f!("SysFreeString"),
            sys_string_len: f!("SysStringLen"),
            variant_init: f!("VariantInit"),
            variant_clear: f!("VariantClear"),
            variant_copy: f!("VariantCopy"),
            variant_copy_ind: f!("VariantCopyInd"),
            variant_change_type: f!("VariantChangeType"),
            safe_array_create_vector: f!("SafeArrayCreateVector"),
            safe_array_put_element: f!("SafeArrayPutElement"),
            safe_array_get_element: f!("SafeArrayGetElement"),
            safe_array_get_lbound: f!("SafeArrayGetLBound"),
            safe_array_get_ubound: f!("SafeArrayGetUBound"),
            safe_array_get_dim: f!("SafeArrayGetDim"),
            safe_array_create: f!("SafeArrayCreate"),
            safe_array_access_data: f!("SafeArrayAccessData"),
            safe_array_unaccess_data: f!("SafeArrayUnaccessData"),
            set_file_fence: get("libwinevbs_set_file_fence").ok().map(|p| std::mem::transmute::<*mut c_void, SetFileFenceFn>(p)),
        };
        if let Ok(init) = get("libwinevbs_init") {
            let init: InitFn = std::mem::transmute::<*mut c_void, InitFn>(init);
            init(&CALLBACKS);
        }
        Ok(lib)
    }
}

/// The library, loaded once.
pub fn get() -> Result<&'static WineVbs, String> {
    LIBRARY
        .get_or_init(|| {
            let tried = candidates();
            for path in &tried {
                if cfg!(target_family = "wasm") || path.exists() {
                    return load_from(path);
                }
            }
            Err(format!("libwinevbs not found (looked in {})", tried.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")))
        })
        .as_ref()
        .map_err(Clone::clone)
}

impl WineVbs {
    /// A BSTR holding `s`, made by the library.
    pub fn bstr(&self, s: &str) -> *mut OLECHAR {
        let w = wide_chars(s);
        // SAFETY: the library copies `w`
        unsafe { (self.sys_alloc_string_len)(w.as_ptr(), w.len() as u32) }
    }

    /// A BSTR's text.
    ///
    /// # Safety
    /// `b` must be null or a BSTR made by this library.
    pub unsafe fn bstr_text(&self, b: *mut OLECHAR) -> String {
        if b.is_null() {
            return String::new();
        }
        unsafe {
            let n = (self.sys_string_len)(b) as usize;
            wide_text(std::slice::from_raw_parts(b, n))
        }
    }
}
