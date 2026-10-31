//! The shaders as SPIR-V, for SDL_GPU's Vulkan backend (Linux): DXC compiled every one the
//! renderer asks for when the player was built (`build.rs`), where on Windows FXC compiles them as
//! the player starts (`fxc.rs`). A shader is found by its source, put together as the renderer
//! puts it together, and its entry point.

use std::ffi::CStr;

include!(concat!(env!("OUT_DIR"), "/spirv.rs"));

/// FNV-1a over the source's bytes, as `build.rs` reckons it.
fn source_hash(source: &str) -> u64 {
    source.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// The SPIR-V of `entry_point` in `source`.
pub fn compiled(source: &str, entry_point: &CStr) -> Result<Vec<u8>, String> {
    let key = source_hash(source);
    let entry = entry_point.to_str().unwrap_or_default();
    SPIRV
        .iter()
        .find(|(k, e, _)| *k == key && *e == entry)
        // a copy: SDL hands the words to Vulkan as they are, so they must sit on a 4-byte boundary,
        // which the program's own bytes needn't
        .map(|(_, _, code)| code.to_vec())
        .ok_or_else(|| format!("no SPIR-V of {entry} for this source (hash {key:016x}): add it to build.rs's SOURCES"))
}
