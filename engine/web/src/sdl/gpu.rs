//! SDL's GPU calls, as the renderer makes them, on the browser's WebGPU (`site/gpu.js`). Textures,
//! buffers, samplers, shaders and pipelines are made in the page at once and named by number; a
//! command buffer is recorded here as words, and the page plays them into WebGPU when it's
//! submitted, a frame's commands in one call. Transfer buffers stay in the program's memory, which
//! the page reads as it plays the uploads.
//!
//! What WebGPU has no room for: no downloads (the player's captures, never asked for in the
//! browser), multisampling with 4 samples or none, no shading rate image (VR's).

use std::cell::{Cell, RefCell, UnsafeCell};
use std::ffi::{CStr, c_void};
use std::fmt::Write as _;
use std::rc::Rc;

use sdl3_sys::everything::*;

#[link(wasm_import_module = "vpi_gpu")]
unsafe extern "C" {
    /// A texture from its description (JSON); its number.
    fn gpu_create_texture(json: *const u8, len: usize) -> u32;
    fn gpu_create_buffer(size: u32, usage: u32) -> u32;
    fn gpu_create_sampler(json: *const u8, len: usize) -> u32;
    /// A shader from its WGSL (`build.rs` puts a header before it), and its entry point.
    fn gpu_create_shader(code: *const u8, len: usize, entry: *const u8, entry_len: usize) -> u32;
    fn gpu_create_pipeline(json: *const u8, len: usize) -> u32;
    /// Lets go of a texture (0), buffer (1), sampler (2), shader (3) or pipeline (4).
    fn gpu_release(kind: u32, id: u32);
    /// Plays a command buffer's words, with the uniforms it pushed; 0 when it couldn't.
    fn gpu_submit(words: *const u32, count: usize, uniforms: *const u8, len: usize) -> u32;
    /// The page's canvas: its size in pixels, and its texture format (0 BGRA, 1 RGBA).
    fn gpu_canvas_size(width: *mut u32, height: *mut u32);
    fn gpu_canvas_format() -> u32;
    /// The canvas shows sRGB-encoded pixels written as linear (1) or as they are (0).
    fn gpu_set_canvas_srgb(srgb: u32);
    /// Whether WebGPU has a feature: 0 clip distances, 1 filtering of 32-bit floats, 2 unclipped depth.
    fn gpu_has_feature(feature: u32) -> u32;
    /// The page decodes a picture's file (`len` bytes at `file`, copied during the call) into texture
    /// `texture`, with the browser's own decoder, and makes VPX's mipmaps of it; 0 when it can't.
    fn gpu_picture(texture: u32, file: *const u8, len: usize) -> u32;
}

/// A picture the page decodes (`player::render::set_host_pictures`): a texture of its size, sRGB, all
/// its levels, which the page fills from the picture's file before the first frame (its own decoder,
/// then VPX's mipmaps on the graphics card); null if it can't.
pub fn host_picture(_device: *mut SDL_GPUDevice, file: &[u8], width: u32, height: u32) -> *mut SDL_GPUTexture {
    let levels = 32 - width.max(height).max(1).leading_zeros();
    let json = format!(
        r#"{{"format":"rgba8unorm-srgb","kind":"2d","width":{width},"height":{height},"layers":1,"levels":{levels},"samples":1,"sampled":true,"target":false,"picture":true}}"#
    );
    let id = json_call(&json, gpu_create_texture);
    if id == 0 {
        return std::ptr::null_mut();
    }
    // SAFETY: the page copies the file's bytes while the call lasts
    if unsafe { gpu_picture(id, file.as_ptr(), file.len()) } == 0 {
        // SAFETY: a plain call into the page, for a texture of ours
        unsafe { gpu_release(0, id) };
        return std::ptr::null_mut();
    }
    boxed(Texture { id, format: SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB })
}

/// What the page knows a texture by: the swapchain's, the canvas's own for this frame.
const SWAPCHAIN: u32 = u32::MAX;

/// The commands' words (`site/gpu.js` reads them in the same order).
mod op {
    pub const BEGIN_RENDER: u32 = 1;
    pub const END_RENDER: u32 = 2;
    pub const PIPELINE: u32 = 3;
    pub const VERTEX_BUFFERS: u32 = 4;
    pub const INDEX_BUFFER: u32 = 5;
    pub const FRAGMENT_TEXTURES: u32 = 6;
    pub const VERTEX_TEXTURES: u32 = 7;
    pub const UNIFORM: u32 = 8;
    pub const DRAW: u32 = 9;
    pub const DRAW_INDEXED: u32 = 10;
    pub const VIEWPORT: u32 = 11;
    pub const SCISSOR: u32 = 12;
    pub const UPLOAD_BUFFER: u32 = 13;
    pub const UPLOAD_TEXTURE: u32 = 14;
    pub const COPY_TEXTURE: u32 = 15;
    pub const MIPMAPS: u32 = 16;
}

/// The device: there's one, the page's.
struct Device {
    /// The swapchain's colours: SDR, or SDR linear (written to an sRGB view of the canvas).
    srgb: Cell<bool>,
}

struct Texture {
    id: u32,
    format: SDL_GPUTextureFormat,
}

struct Buffer {
    id: u32,
}

/// Upload data in the program's memory, which the page reads when the command buffer that
/// uploads it is played. Mapped with `cycle` while a command buffer still holds it, it's given a
/// new block, so the upload keeps what it was given.
struct TransferBuffer {
    data: RefCell<Rc<UnsafeCell<Vec<u8>>>>,
    size: usize,
}

struct Sampler {
    id: u32,
}

struct Shader {
    id: u32,
}

struct Pipeline {
    id: u32,
}

/// A command buffer as it's recorded: its words, the uniform data it pushed (each push at a
/// 256-byte boundary, as WebGPU's dynamic offsets want them), and the upload data it reads.
#[derive(Default)]
struct Commands {
    words: Vec<u32>,
    uniforms: Vec<u8>,
    holds: Vec<Rc<UnsafeCell<Vec<u8>>>>,
    /// It draws into the canvas.
    swapchain: bool,
}

impl Commands {
    fn push(&mut self, words: &[u32]) {
        self.words.extend_from_slice(words);
    }
}

fn commands<'a>(command_buffer: *mut SDL_GPUCommandBuffer) -> Option<&'a mut Commands> {
    // SAFETY: the renderer passes the command buffer (or one of its passes, the same pointer) we
    // made, until it's submitted or cancelled
    unsafe { command_buffer.cast::<Commands>().as_mut() }
}

fn of<'a, T, U>(p: *mut U) -> Option<&'a T> {
    // SAFETY: the renderer passes back the handles we made, until it releases them
    unsafe { p.cast::<T>().as_ref() }
}

fn texture_id(t: *mut SDL_GPUTexture) -> u32 {
    if t.is_null() {
        0
    } else if t as usize == SWAPCHAIN_HANDLE {
        SWAPCHAIN
    } else {
        of::<Texture, _>(t).map_or(0, |t| t.id)
    }
}

/// The swapchain's texture handed to the renderer: never a texture of ours.
const SWAPCHAIN_HANDLE: usize = usize::MAX & !7;

fn boxed<T, U>(value: T) -> *mut U {
    Box::into_raw(Box::new(value)).cast()
}

fn json_call(json: &str, f: unsafe extern "C" fn(*const u8, usize) -> u32) -> u32 {
    // SAFETY: the page reads the text while the call lasts
    unsafe { f(json.as_ptr(), json.len()) }
}

// ---- formats ----

/// A texture format's WebGPU name and bytes per texel; None where WebGPU has no such format.
pub fn format_info(f: SDL_GPUTextureFormat) -> Option<(&'static str, u32)> {
    Some(match f {
        SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM => ("rgba8unorm", 4),
        SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB => ("rgba8unorm-srgb", 4),
        SDL_GPU_TEXTUREFORMAT_B8G8R8A8_UNORM => ("bgra8unorm", 4),
        SDL_GPU_TEXTUREFORMAT_B8G8R8A8_UNORM_SRGB => ("bgra8unorm-srgb", 4),
        SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT => ("rgba16float", 8),
        SDL_GPU_TEXTUREFORMAT_R32G32B32A32_FLOAT => ("rgba32float", 16),
        SDL_GPU_TEXTUREFORMAT_R10G10B10A2_UNORM => ("rgb10a2unorm", 4),
        SDL_GPU_TEXTUREFORMAT_R8G8_UNORM => ("rg8unorm", 2),
        SDL_GPU_TEXTUREFORMAT_R8_UNORM => ("r8unorm", 1),
        SDL_GPU_TEXTUREFORMAT_R8_UINT => ("r8uint", 1),
        SDL_GPU_TEXTUREFORMAT_R16_FLOAT => ("r16float", 2),
        SDL_GPU_TEXTUREFORMAT_R16G16_FLOAT => ("rg16float", 4),
        SDL_GPU_TEXTUREFORMAT_R32_FLOAT => ("r32float", 4),
        SDL_GPU_TEXTUREFORMAT_R11G11B10_UFLOAT => ("rg11b10ufloat", 4),
        SDL_GPU_TEXTUREFORMAT_D16_UNORM => ("depth16unorm", 2),
        SDL_GPU_TEXTUREFORMAT_D24_UNORM => ("depth24plus", 4),
        SDL_GPU_TEXTUREFORMAT_D32_FLOAT => ("depth32float", 4),
        SDL_GPU_TEXTUREFORMAT_D24_UNORM_S8_UINT => ("depth24plus-stencil8", 4),
        _ => return None,
    })
}

fn is_depth(f: SDL_GPUTextureFormat) -> bool {
    format_info(f).is_some_and(|(name, _)| name.starts_with("depth"))
}

fn vertex_format(f: SDL_GPUVertexElementFormat) -> &'static str {
    match f {
        SDL_GPU_VERTEXELEMENTFORMAT_FLOAT => "float32",
        SDL_GPU_VERTEXELEMENTFORMAT_FLOAT2 => "float32x2",
        SDL_GPU_VERTEXELEMENTFORMAT_FLOAT3 => "float32x3",
        SDL_GPU_VERTEXELEMENTFORMAT_FLOAT4 => "float32x4",
        SDL_GPU_VERTEXELEMENTFORMAT_UBYTE4_NORM => "unorm8x4",
        SDL_GPU_VERTEXELEMENTFORMAT_UBYTE4 => "uint8x4",
        SDL_GPU_VERTEXELEMENTFORMAT_UINT => "uint32",
        SDL_GPU_VERTEXELEMENTFORMAT_UINT2 => "uint32x2",
        SDL_GPU_VERTEXELEMENTFORMAT_UINT3 => "uint32x3",
        SDL_GPU_VERTEXELEMENTFORMAT_UINT4 => "uint32x4",
        SDL_GPU_VERTEXELEMENTFORMAT_INT => "sint32",
        SDL_GPU_VERTEXELEMENTFORMAT_INT2 => "sint32x2",
        SDL_GPU_VERTEXELEMENTFORMAT_INT3 => "sint32x3",
        SDL_GPU_VERTEXELEMENTFORMAT_INT4 => "sint32x4",
        SDL_GPU_VERTEXELEMENTFORMAT_HALF2 => "float16x2",
        SDL_GPU_VERTEXELEMENTFORMAT_HALF4 => "float16x4",
        SDL_GPU_VERTEXELEMENTFORMAT_SHORT2_NORM => "snorm16x2",
        SDL_GPU_VERTEXELEMENTFORMAT_SHORT4_NORM => "snorm16x4",
        SDL_GPU_VERTEXELEMENTFORMAT_USHORT2_NORM => "unorm16x2",
        SDL_GPU_VERTEXELEMENTFORMAT_USHORT4_NORM => "unorm16x4",
        _ => "float32x4",
    }
}

fn blend_factor(f: SDL_GPUBlendFactor) -> &'static str {
    match f {
        SDL_GPU_BLENDFACTOR_ZERO => "zero",
        SDL_GPU_BLENDFACTOR_ONE => "one",
        SDL_GPU_BLENDFACTOR_SRC_COLOR => "src",
        SDL_GPU_BLENDFACTOR_ONE_MINUS_SRC_COLOR => "one-minus-src",
        SDL_GPU_BLENDFACTOR_DST_COLOR => "dst",
        SDL_GPU_BLENDFACTOR_ONE_MINUS_DST_COLOR => "one-minus-dst",
        SDL_GPU_BLENDFACTOR_SRC_ALPHA => "src-alpha",
        SDL_GPU_BLENDFACTOR_ONE_MINUS_SRC_ALPHA => "one-minus-src-alpha",
        SDL_GPU_BLENDFACTOR_DST_ALPHA => "dst-alpha",
        SDL_GPU_BLENDFACTOR_ONE_MINUS_DST_ALPHA => "one-minus-dst-alpha",
        SDL_GPU_BLENDFACTOR_CONSTANT_COLOR => "constant",
        SDL_GPU_BLENDFACTOR_ONE_MINUS_CONSTANT_COLOR => "one-minus-constant",
        SDL_GPU_BLENDFACTOR_SRC_ALPHA_SATURATE => "src-alpha-saturated",
        _ => "one",
    }
}

fn blend_op(o: SDL_GPUBlendOp) -> &'static str {
    match o {
        SDL_GPU_BLENDOP_SUBTRACT => "subtract",
        SDL_GPU_BLENDOP_REVERSE_SUBTRACT => "reverse-subtract",
        SDL_GPU_BLENDOP_MIN => "min",
        SDL_GPU_BLENDOP_MAX => "max",
        _ => "add",
    }
}

fn compare(o: SDL_GPUCompareOp) -> &'static str {
    match o {
        SDL_GPU_COMPAREOP_NEVER => "never",
        SDL_GPU_COMPAREOP_LESS => "less",
        SDL_GPU_COMPAREOP_EQUAL => "equal",
        SDL_GPU_COMPAREOP_LESS_OR_EQUAL => "less-equal",
        SDL_GPU_COMPAREOP_GREATER => "greater",
        SDL_GPU_COMPAREOP_NOT_EQUAL => "not-equal",
        SDL_GPU_COMPAREOP_GREATER_OR_EQUAL => "greater-equal",
        _ => "always",
    }
}

fn stencil_op(o: SDL_GPUStencilOp) -> &'static str {
    match o {
        SDL_GPU_STENCILOP_ZERO => "zero",
        SDL_GPU_STENCILOP_REPLACE => "replace",
        SDL_GPU_STENCILOP_INCREMENT_AND_CLAMP => "increment-clamp",
        SDL_GPU_STENCILOP_DECREMENT_AND_CLAMP => "decrement-clamp",
        SDL_GPU_STENCILOP_INVERT => "invert",
        SDL_GPU_STENCILOP_INCREMENT_AND_WRAP => "increment-wrap",
        SDL_GPU_STENCILOP_DECREMENT_AND_WRAP => "decrement-wrap",
        _ => "keep",
    }
}

fn samples(s: SDL_GPUSampleCount) -> u32 {
    if s == SDL_GPU_SAMPLECOUNT_1 { 1 } else { 4 }
}

fn load(o: SDL_GPULoadOp) -> u32 {
    match o {
        SDL_GPU_LOADOP_LOAD => 0,
        SDL_GPU_LOADOP_CLEAR => 1,
        _ => 2, // don't care
    }
}

fn store(o: SDL_GPUStoreOp) -> u32 {
    match o {
        SDL_GPU_STOREOP_STORE => 0,
        SDL_GPU_STOREOP_DONT_CARE => 1,
        SDL_GPU_STOREOP_RESOLVE => 2,
        _ => 3, // resolve and store
    }
}

// ---- the device ----

thread_local! {
    static DEVICE: Cell<*mut Device> = const { Cell::new(std::ptr::null_mut()) };
    /// The device's properties: its name, for the log.
    static DEVICE_PROPS: Cell<SDL_PropertiesID> = const { Cell::new(SDL_PropertiesID(0)) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_CreateGPUDeviceWithProperties(_props: SDL_PropertiesID) -> *mut SDL_GPUDevice {
    DEVICE.with(|d| {
        if d.get().is_null() {
            d.set(Box::into_raw(Box::new(Device { srgb: Cell::new(false) })));
        }
        d.get().cast()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_DestroyGPUDevice(_device: *mut SDL_GPUDevice) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetGPUDeviceProperties(_device: *mut SDL_GPUDevice) -> SDL_PropertiesID {
    DEVICE_PROPS.with(|p| {
        if p.get().0 == 0 {
            // SAFETY: our own properties
            unsafe {
                let props = SDL_CreateProperties();
                SDL_SetStringProperty(props, SDL_PROP_GPU_DEVICE_NAME_STRING, c"the browser's WebGPU".as_ptr());
                p.set(props);
            }
        }
        p.get()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_ClaimWindowForGPUDevice(_device: *mut SDL_GPUDevice, _window: *mut SDL_Window) -> bool {
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_ReleaseWindowFromGPUDevice(_device: *mut SDL_GPUDevice, _window: *mut SDL_Window) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_WindowSupportsGPUPresentMode(_device: *mut SDL_GPUDevice, _window: *mut SDL_Window, mode: SDL_GPUPresentMode) -> bool {
    // the browser shows a frame at each of its refreshes
    mode == SDL_GPU_PRESENTMODE_VSYNC
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_WindowSupportsGPUSwapchainComposition(_device: *mut SDL_GPUDevice, _window: *mut SDL_Window, composition: SDL_GPUSwapchainComposition) -> bool {
    composition == SDL_GPU_SWAPCHAINCOMPOSITION_SDR || composition == SDL_GPU_SWAPCHAINCOMPOSITION_SDR_LINEAR
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_SetGPUSwapchainParameters(device: *mut SDL_GPUDevice, _window: *mut SDL_Window, composition: SDL_GPUSwapchainComposition, mode: SDL_GPUPresentMode) -> bool {
    if mode != SDL_GPU_PRESENTMODE_VSYNC || !(composition == SDL_GPU_SWAPCHAINCOMPOSITION_SDR || composition == SDL_GPU_SWAPCHAINCOMPOSITION_SDR_LINEAR) {
        super::set_error("WebGPU's canvas: vsync and SDR only");
        return false;
    }
    let linear = composition == SDL_GPU_SWAPCHAINCOMPOSITION_SDR_LINEAR;
    if let Some(d) = of::<Device, _>(device) {
        d.srgb.set(linear);
    }
    // SAFETY: a plain call into the page
    unsafe { gpu_set_canvas_srgb(u32::from(linear)) };
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_SetGPUAllowedFramesInFlight(_device: *mut SDL_GPUDevice, _allowed: Uint32) -> bool {
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GetGPUSwapchainTextureFormat(device: *mut SDL_GPUDevice, _window: *mut SDL_Window) -> SDL_GPUTextureFormat {
    let srgb = of::<Device, _>(device).is_some_and(|d| d.srgb.get());
    // SAFETY: a plain call into the page
    let rgba = unsafe { gpu_canvas_format() } == 1;
    match (rgba, srgb) {
        (false, false) => SDL_GPU_TEXTUREFORMAT_B8G8R8A8_UNORM,
        (false, true) => SDL_GPU_TEXTUREFORMAT_B8G8R8A8_UNORM_SRGB,
        (true, false) => SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
        (true, true) => SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GPUTextureSupportsFormat(_device: *mut SDL_GPUDevice, format: SDL_GPUTextureFormat, kind: SDL_GPUTextureType, usage: SDL_GPUTextureUsageFlags) -> bool {
    let Some((name, _)) = format_info(format) else { return false };
    if kind == SDL_GPU_TEXTURETYPE_3D && is_depth(format) {
        return false;
    }
    let target = (usage & (SDL_GPU_TEXTUREUSAGE_COLOR_TARGET | SDL_GPU_TEXTUREUSAGE_DEPTH_STENCIL_TARGET)).0 != 0;
    let storage = (usage & (SDL_GPU_TEXTUREUSAGE_GRAPHICS_STORAGE_READ | SDL_GPU_TEXTUREUSAGE_COMPUTE_STORAGE_READ | SDL_GPU_TEXTUREUSAGE_COMPUTE_STORAGE_WRITE)).0 != 0;
    // (VR's shading rate image, our SDL's usage bit 1 << 16, among the rest WebGPU has no room for)
    if (usage.0 & !(SDL_GPU_TEXTUREUSAGE_SAMPLER | SDL_GPU_TEXTUREUSAGE_COLOR_TARGET | SDL_GPU_TEXTUREUSAGE_DEPTH_STENCIL_TARGET).0) != 0 || storage {
        return false;
    }
    // drawn into without WebGPU's optional features: all but rg11b10 (its own feature) and the
    // integer and sRGB BGRA formats, which draw too
    !(target && name == "rg11b10ufloat")
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GPUTextureSupportsSampleCount(_device: *mut SDL_GPUDevice, format: SDL_GPUTextureFormat, count: SDL_GPUSampleCount) -> bool {
    format_info(format).is_some() && (count == SDL_GPU_SAMPLECOUNT_1 || count == SDL_GPU_SAMPLECOUNT_4)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GPUTextureFormatTexelBlockSize(format: SDL_GPUTextureFormat) -> Uint32 {
    format_info(format).map_or(0, |(_, size)| size)
}

/// Whether the page's WebGPU has one of its optional features (`gpu_has_feature`).
pub fn has_feature(feature: u32) -> bool {
    // SAFETY: a plain call into the page
    unsafe { gpu_has_feature(feature) != 0 }
}

// ---- resources ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_CreateGPUTexture(_device: *mut SDL_GPUDevice, info: *const SDL_GPUTextureCreateInfo) -> *mut SDL_GPUTexture {
    // SAFETY: the renderer's description
    let i = unsafe { &*info };
    let Some((format, _)) = format_info(i.format) else {
        super::set_error(&format!("WebGPU has no texture format {}", i.format.0));
        return std::ptr::null_mut();
    };
    let kind = match i.r#type {
        SDL_GPU_TEXTURETYPE_2D_ARRAY => "2d-array",
        SDL_GPU_TEXTURETYPE_CUBE => "cube",
        SDL_GPU_TEXTURETYPE_CUBE_ARRAY => "cube-array",
        SDL_GPU_TEXTURETYPE_3D => "3d",
        _ => "2d",
    };
    let u = i.usage;
    let has = |f: SDL_GPUTextureUsageFlags| (u & f).0 != 0;
    let json = format!(
        r#"{{"format":"{format}","kind":"{kind}","width":{},"height":{},"layers":{},"levels":{},"samples":{},"sampled":{},"target":{}}}"#,
        i.width.max(1),
        i.height.max(1),
        i.layer_count_or_depth.max(1),
        i.num_levels.max(1),
        samples(i.sample_count),
        has(SDL_GPU_TEXTUREUSAGE_SAMPLER),
        has(SDL_GPU_TEXTUREUSAGE_COLOR_TARGET) || has(SDL_GPU_TEXTUREUSAGE_DEPTH_STENCIL_TARGET),
    );
    let id = json_call(&json, gpu_create_texture);
    if id == 0 {
        super::set_error("WebGPU could not make the texture");
        return std::ptr::null_mut();
    }
    boxed(Texture { id, format: i.format })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_ReleaseGPUTexture(_device: *mut SDL_GPUDevice, texture: *mut SDL_GPUTexture) {
    if texture.is_null() || texture as usize == SWAPCHAIN_HANDLE {
        return;
    }
    // SAFETY: ours, released once
    let t = unsafe { Box::from_raw(texture.cast::<Texture>()) };
    // SAFETY: a plain call into the page
    unsafe { gpu_release(0, t.id) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_CreateGPUBuffer(_device: *mut SDL_GPUDevice, info: *const SDL_GPUBufferCreateInfo) -> *mut SDL_GPUBuffer {
    // SAFETY: the renderer's description
    let i = unsafe { &*info };
    let usage = (if (i.usage & SDL_GPU_BUFFERUSAGE_VERTEX).0 != 0 { 1 } else { 0 }) | (if (i.usage & SDL_GPU_BUFFERUSAGE_INDEX).0 != 0 { 2 } else { 0 });
    // SAFETY: a plain call into the page
    let id = unsafe { gpu_create_buffer(i.size.next_multiple_of(4), usage) };
    if id == 0 {
        super::set_error("WebGPU could not make the buffer");
        return std::ptr::null_mut();
    }
    boxed(Buffer { id })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_ReleaseGPUBuffer(_device: *mut SDL_GPUDevice, buffer: *mut SDL_GPUBuffer) {
    if buffer.is_null() {
        return;
    }
    // SAFETY: ours, released once
    let b = unsafe { Box::from_raw(buffer.cast::<Buffer>()) };
    // SAFETY: a plain call into the page
    unsafe { gpu_release(1, b.id) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_CreateGPUTransferBuffer(_device: *mut SDL_GPUDevice, info: *const SDL_GPUTransferBufferCreateInfo) -> *mut SDL_GPUTransferBuffer {
    // SAFETY: the renderer's description
    let size = unsafe { (*info).size } as usize;
    boxed(TransferBuffer { data: RefCell::new(Rc::new(UnsafeCell::new(vec![0; size]))), size })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_ReleaseGPUTransferBuffer(_device: *mut SDL_GPUDevice, buffer: *mut SDL_GPUTransferBuffer) {
    if !buffer.is_null() {
        // SAFETY: ours, released once; a command buffer still holding its data keeps it
        drop(unsafe { Box::from_raw(buffer.cast::<TransferBuffer>()) });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_MapGPUTransferBuffer(_device: *mut SDL_GPUDevice, buffer: *mut SDL_GPUTransferBuffer, cycle: bool) -> *mut c_void {
    let Some(t) = of::<TransferBuffer, _>(buffer) else { return std::ptr::null_mut() };
    let mut data = t.data.borrow_mut();
    // a command buffer not yet played still reads the old block: a new one for this
    if cycle && Rc::strong_count(&data) > 1 {
        *data = Rc::new(UnsafeCell::new(vec![0; t.size]));
    }
    // SAFETY: the block lives while the transfer buffer (or a command buffer) holds it
    unsafe { (*data.get()).as_mut_ptr().cast() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_UnmapGPUTransferBuffer(_device: *mut SDL_GPUDevice, _buffer: *mut SDL_GPUTransferBuffer) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_CreateGPUSampler(_device: *mut SDL_GPUDevice, info: *const SDL_GPUSamplerCreateInfo) -> *mut SDL_GPUSampler {
    // SAFETY: the renderer's description
    let i = unsafe { &*info };
    let filter = |f: SDL_GPUFilter| if f == SDL_GPU_FILTER_LINEAR { "linear" } else { "nearest" };
    let address = |a: SDL_GPUSamplerAddressMode| match a {
        SDL_GPU_SAMPLERADDRESSMODE_REPEAT => "repeat",
        SDL_GPU_SAMPLERADDRESSMODE_MIRRORED_REPEAT => "mirror-repeat",
        _ => "clamp-to-edge",
    };
    let mipmap = if i.mipmap_mode == SDL_GPU_SAMPLERMIPMAPMODE_LINEAR { "linear" } else { "nearest" };
    // WebGPU's anisotropy wants every filter linear
    let all_linear = i.min_filter == SDL_GPU_FILTER_LINEAR && i.mag_filter == SDL_GPU_FILTER_LINEAR && i.mipmap_mode == SDL_GPU_SAMPLERMIPMAPMODE_LINEAR;
    let anisotropy = if i.enable_anisotropy && all_linear { i.max_anisotropy.clamp(1.0, 16.0) as u32 } else { 1 };
    let max_lod = if i.max_lod > 0.0 { i.max_lod.min(32.0) } else { 32.0 };
    let mut json = format!(
        r#"{{"minFilter":"{}","magFilter":"{}","mipmapFilter":"{mipmap}","addressModeU":"{}","addressModeV":"{}","addressModeW":"{}","lodMinClamp":{},"lodMaxClamp":{max_lod},"maxAnisotropy":{anisotropy}"#,
        filter(i.min_filter),
        filter(i.mag_filter),
        address(i.address_mode_u),
        address(i.address_mode_v),
        address(i.address_mode_w),
        i.min_lod.max(0.0),
    );
    if i.enable_compare {
        write!(json, r#","compare":"{}""#, compare(i.compare_op)).unwrap();
    }
    json.push('}');
    boxed(Sampler { id: json_call(&json, gpu_create_sampler) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_ReleaseGPUSampler(_device: *mut SDL_GPUDevice, sampler: *mut SDL_GPUSampler) {
    if sampler.is_null() {
        return;
    }
    // SAFETY: ours, released once
    let s = unsafe { Box::from_raw(sampler.cast::<Sampler>()) };
    // SAFETY: a plain call into the page
    unsafe { gpu_release(2, s.id) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_CreateGPUShader(_device: *mut SDL_GPUDevice, info: *const SDL_GPUShaderCreateInfo) -> *mut SDL_GPUShader {
    // SAFETY: the renderer's description: its code (WGSL made by build.rs) and entry point
    let (code, entry) = unsafe {
        let i = &*info;
        (std::slice::from_raw_parts(i.code, i.code_size), CStr::from_ptr(i.entrypoint).to_bytes())
    };
    // SAFETY: the page reads both while the call lasts
    let id = unsafe { gpu_create_shader(code.as_ptr(), code.len(), entry.as_ptr(), entry.len()) };
    if id == 0 {
        super::set_error("WebGPU could not make the shader");
        return std::ptr::null_mut();
    }
    boxed(Shader { id })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_ReleaseGPUShader(_device: *mut SDL_GPUDevice, shader: *mut SDL_GPUShader) {
    if shader.is_null() {
        return;
    }
    // SAFETY: ours, released once
    let s = unsafe { Box::from_raw(shader.cast::<Shader>()) };
    // SAFETY: a plain call into the page
    unsafe { gpu_release(3, s.id) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_CreateGPUGraphicsPipeline(_device: *mut SDL_GPUDevice, info: *const SDL_GPUGraphicsPipelineCreateInfo) -> *mut SDL_GPUGraphicsPipeline {
    // SAFETY: the renderer's description, its lists as long as it says
    let i = unsafe { &*info };
    let (Some(vs), Some(fs)) = (of::<Shader, _>(i.vertex_shader), of::<Shader, _>(i.fragment_shader)) else {
        super::set_error("a pipeline needs both its shaders");
        return std::ptr::null_mut();
    };
    let buffers: &[SDL_GPUVertexBufferDescription] = list(i.vertex_input_state.vertex_buffer_descriptions, i.vertex_input_state.num_vertex_buffers);
    let attributes: &[SDL_GPUVertexAttribute] = list(i.vertex_input_state.vertex_attributes, i.vertex_input_state.num_vertex_attributes);
    let targets: &[SDL_GPUColorTargetDescription] = list(i.target_info.color_target_descriptions, i.target_info.num_color_targets);

    let mut json = format!(r#"{{"vs":{},"fs":{},"buffers":["#, vs.id, fs.id);
    for (k, b) in buffers.iter().enumerate() {
        if k > 0 {
            json.push(',');
        }
        let step = if b.input_rate == SDL_GPU_VERTEXINPUTRATE_INSTANCE { "instance" } else { "vertex" };
        write!(json, r#"{{"slot":{},"stride":{},"step":"{step}","attributes":["#, b.slot, b.pitch).unwrap();
        let mut first = true;
        for a in attributes.iter().filter(|a| a.buffer_slot == b.slot) {
            if !first {
                json.push(',');
            }
            first = false;
            write!(json, r#"{{"location":{},"offset":{},"format":"{}"}}"#, a.location, a.offset, vertex_format(a.format)).unwrap();
        }
        json.push_str("]}");
    }
    let topology = match i.primitive_type {
        SDL_GPU_PRIMITIVETYPE_TRIANGLESTRIP => "triangle-strip",
        SDL_GPU_PRIMITIVETYPE_LINELIST => "line-list",
        SDL_GPU_PRIMITIVETYPE_LINESTRIP => "line-strip",
        SDL_GPU_PRIMITIVETYPE_POINTLIST => "point-list",
        _ => "triangle-list",
    };
    let r = &i.rasterizer_state;
    let cull = match r.cull_mode {
        SDL_GPU_CULLMODE_FRONT => "front",
        SDL_GPU_CULLMODE_BACK => "back",
        _ => "none",
    };
    let front = if r.front_face == SDL_GPU_FRONTFACE_CLOCKWISE { "cw" } else { "ccw" };
    write!(json, r#"],"topology":"{topology}","cull":"{cull}","front":"{front}","unclipped":{},"targets":["#, !r.enable_depth_clip).unwrap();
    for (k, t) in targets.iter().enumerate() {
        if k > 0 {
            json.push(',');
        }
        let format = format_info(t.format).map_or("bgra8unorm", |(n, _)| n);
        let b = &t.blend_state;
        let mask = if b.enable_color_write_mask { b.color_write_mask.0 } else { 0xF };
        write!(json, r#"{{"format":"{format}","mask":{mask}"#).unwrap();
        if b.enable_blend {
            write!(
                json,
                r#","blend":{{"color":{{"srcFactor":"{}","dstFactor":"{}","operation":"{}"}},"alpha":{{"srcFactor":"{}","dstFactor":"{}","operation":"{}"}}}}"#,
                blend_factor(b.src_color_blendfactor),
                blend_factor(b.dst_color_blendfactor),
                blend_op(b.color_blend_op),
                blend_factor(b.src_alpha_blendfactor),
                blend_factor(b.dst_alpha_blendfactor),
                blend_op(b.alpha_blend_op),
            )
            .unwrap();
        }
        json.push('}');
    }
    json.push(']');
    if i.target_info.has_depth_stencil_target {
        let d = &i.depth_stencil_state;
        let format = format_info(i.target_info.depth_stencil_format).map_or("depth32float", |(n, _)| n);
        let face = |s: &SDL_GPUStencilOpState| {
            format!(
                r#"{{"compare":"{}","failOp":"{}","depthFailOp":"{}","passOp":"{}"}}"#,
                compare(s.compare_op),
                stencil_op(s.fail_op),
                stencil_op(s.depth_fail_op),
                stencil_op(s.pass_op)
            )
        };
        let stencil = d.enable_stencil_test && format.contains("stencil");
        write!(
            json,
            r#","depth":{{"format":"{format}","depthWriteEnabled":{},"depthCompare":"{}","depthBias":{},"depthBiasSlopeScale":{},"depthBiasClamp":{}"#,
            d.enable_depth_write && d.enable_depth_test,
            if d.enable_depth_test { compare(d.compare_op) } else { "always" },
            if r.enable_depth_bias { r.depth_bias_constant_factor.round() as i32 } else { 0 },
            if r.enable_depth_bias { r.depth_bias_slope_factor } else { 0.0 },
            if r.enable_depth_bias { r.depth_bias_clamp } else { 0.0 },
        )
        .unwrap();
        if stencil {
            write!(json, r#","stencilFront":{},"stencilBack":{},"stencilReadMask":{},"stencilWriteMask":{}"#, face(&d.front_stencil_state), face(&d.back_stencil_state), d.compare_mask, d.write_mask).unwrap();
        }
        json.push('}');
    }
    let m = &i.multisample_state;
    write!(json, r#","samples":{},"alphaToCoverage":{}}}"#, samples(m.sample_count), m.enable_alpha_to_coverage).unwrap();
    let id = json_call(&json, gpu_create_pipeline);
    if id == 0 {
        super::set_error("WebGPU could not make the pipeline");
        return std::ptr::null_mut();
    }
    boxed(Pipeline { id })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_ReleaseGPUGraphicsPipeline(_device: *mut SDL_GPUDevice, pipeline: *mut SDL_GPUGraphicsPipeline) {
    if pipeline.is_null() {
        return;
    }
    // SAFETY: ours, released once
    let p = unsafe { Box::from_raw(pipeline.cast::<Pipeline>()) };
    // SAFETY: a plain call into the page
    unsafe { gpu_release(4, p.id) };
}

/// A list the renderer passes, as long as it says.
fn list<'a, T>(p: *const T, n: u32) -> &'a [T] {
    // SAFETY: the renderer's list, `n` long
    if p.is_null() || n == 0 { &[] } else { unsafe { std::slice::from_raw_parts(p, n as usize) } }
}

// ---- command buffers ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_AcquireGPUCommandBuffer(_device: *mut SDL_GPUDevice) -> *mut SDL_GPUCommandBuffer {
    boxed(Commands::default())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_AcquireGPUSwapchainTexture(command_buffer: *mut SDL_GPUCommandBuffer, _window: *mut SDL_Window, texture: *mut *mut SDL_GPUTexture, width: *mut Uint32, height: *mut Uint32) -> bool {
    let (mut w, mut h) = (0, 0);
    // SAFETY: a plain call into the page
    unsafe { gpu_canvas_size(&mut w, &mut h) };
    if let Some(c) = commands(command_buffer) {
        c.swapchain = true;
    }
    // SAFETY: the renderer's out-parameters
    unsafe {
        *texture = if w == 0 || h == 0 { std::ptr::null_mut() } else { SWAPCHAIN_HANDLE as *mut SDL_GPUTexture };
        if !width.is_null() {
            *width = w;
        }
        if !height.is_null() {
            *height = h;
        }
    }
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_SubmitGPUCommandBuffer(command_buffer: *mut SDL_GPUCommandBuffer) -> bool {
    if command_buffer.is_null() {
        return false;
    }
    // SAFETY: ours, submitted once
    let c = unsafe { Box::from_raw(command_buffer.cast::<Commands>()) };
    // SAFETY: the page reads both while the call lasts; the upload data `c` holds stays until after
    let ok = unsafe { gpu_submit(c.words.as_ptr(), c.words.len(), c.uniforms.as_ptr(), c.uniforms.len()) } != 0;
    drop(c);
    if !ok {
        super::set_error("WebGPU could not play the command buffer");
    }
    ok
}

/// The fence a submit hands back: WebGPU's work is never waited for here, so it's always signalled.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_SubmitGPUCommandBufferAndAcquireFence(command_buffer: *mut SDL_GPUCommandBuffer) -> *mut SDL_GPUFence {
    // SAFETY: as the renderer passes it
    if unsafe { SDL_SubmitGPUCommandBuffer(command_buffer) } { SWAPCHAIN_HANDLE as *mut SDL_GPUFence } else { std::ptr::null_mut() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_WaitForGPUFences(_device: *mut SDL_GPUDevice, _wait_all: bool, _fences: *const *mut SDL_GPUFence, _count: Uint32) -> bool {
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_ReleaseGPUFence(_device: *mut SDL_GPUDevice, _fence: *mut SDL_GPUFence) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_WaitForGPUIdle(_device: *mut SDL_GPUDevice) -> bool {
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_CancelGPUCommandBuffer(command_buffer: *mut SDL_GPUCommandBuffer) -> bool {
    if !command_buffer.is_null() {
        // SAFETY: ours, cancelled once
        drop(unsafe { Box::from_raw(command_buffer.cast::<Commands>()) });
    }
    true
}

fn push_uniform(command_buffer: *mut SDL_GPUCommandBuffer, stage: u32, slot: Uint32, data: *const c_void, length: Uint32) {
    let Some(c) = commands(command_buffer) else { return };
    let at = c.uniforms.len().next_multiple_of(256);
    c.uniforms.resize(at, 0);
    // SAFETY: the renderer's data, `length` bytes
    c.uniforms.extend_from_slice(unsafe { std::slice::from_raw_parts(data.cast::<u8>(), length as usize) });
    c.push(&[op::UNIFORM, stage, slot, at as u32, length]);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_PushGPUVertexUniformData(command_buffer: *mut SDL_GPUCommandBuffer, slot_index: Uint32, data: *const c_void, length: Uint32) {
    push_uniform(command_buffer, 0, slot_index, data, length);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_PushGPUFragmentUniformData(command_buffer: *mut SDL_GPUCommandBuffer, slot_index: Uint32, data: *const c_void, length: Uint32) {
    push_uniform(command_buffer, 1, slot_index, data, length);
}

// ---- render passes ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_BeginGPURenderPass(command_buffer: *mut SDL_GPUCommandBuffer, color_target_infos: *const SDL_GPUColorTargetInfo, num_color_targets: Uint32, depth_stencil_target_info: *const SDL_GPUDepthStencilTargetInfo) -> *mut SDL_GPURenderPass {
    let Some(c) = commands(command_buffer) else { return std::ptr::null_mut() };
    let targets = if color_target_infos.is_null() { &[][..] } else { unsafe { std::slice::from_raw_parts(color_target_infos, num_color_targets as usize) } };
    c.push(&[op::BEGIN_RENDER, targets.len() as u32]);
    for t in targets {
        let k = t.clear_color;
        c.push(&[
            texture_id(t.texture),
            t.mip_level,
            t.layer_or_depth_plane,
            load(t.load_op),
            store(t.store_op),
            k.r.to_bits(),
            k.g.to_bits(),
            k.b.to_bits(),
            k.a.to_bits(),
            texture_id(t.resolve_texture),
            t.resolve_mip_level,
            t.resolve_layer,
        ]);
    }
    // SAFETY: the renderer's depth target, if any
    match unsafe { depth_stencil_target_info.as_ref() } {
        Some(d) => c.push(&[
            1,
            texture_id(d.texture),
            load(d.load_op),
            store(d.store_op),
            d.clear_depth.to_bits(),
            load(d.stencil_load_op),
            store(d.stencil_store_op),
            u32::from(d.clear_stencil),
            u32::from(d.mip_level),
            u32::from(d.layer),
        ]),
        None => c.push(&[0]),
    }
    command_buffer.cast()
}

fn pass_commands<'a>(pass: *mut SDL_GPURenderPass) -> Option<&'a mut Commands> {
    commands(pass.cast())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_EndGPURenderPass(render_pass: *mut SDL_GPURenderPass) {
    if let Some(c) = pass_commands(render_pass) {
        c.push(&[op::END_RENDER]);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_BindGPUGraphicsPipeline(render_pass: *mut SDL_GPURenderPass, graphics_pipeline: *mut SDL_GPUGraphicsPipeline) {
    if let (Some(c), Some(p)) = (pass_commands(render_pass), of::<Pipeline, _>(graphics_pipeline)) {
        c.push(&[op::PIPELINE, p.id]);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_BindGPUVertexBuffers(render_pass: *mut SDL_GPURenderPass, first_slot: Uint32, bindings: *const SDL_GPUBufferBinding, num_bindings: Uint32) {
    let Some(c) = pass_commands(render_pass) else { return };
    // SAFETY: the renderer's bindings, as many as it says
    let bindings = unsafe { std::slice::from_raw_parts(bindings, num_bindings as usize) };
    c.push(&[op::VERTEX_BUFFERS, first_slot, num_bindings]);
    for b in bindings {
        c.push(&[of::<Buffer, _>(b.buffer).map_or(0, |b| b.id), b.offset]);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_BindGPUIndexBuffer(render_pass: *mut SDL_GPURenderPass, binding: *const SDL_GPUBufferBinding, index_element_size: SDL_GPUIndexElementSize) {
    let Some(c) = pass_commands(render_pass) else { return };
    // SAFETY: the renderer's binding
    let b = unsafe { &*binding };
    let size = if index_element_size == SDL_GPU_INDEXELEMENTSIZE_32BIT { 32 } else { 16 };
    c.push(&[op::INDEX_BUFFER, of::<Buffer, _>(b.buffer).map_or(0, |b| b.id), b.offset, size]);
}

fn bind_textures(render_pass: *mut SDL_GPURenderPass, code: u32, first_slot: Uint32, bindings: *const SDL_GPUTextureSamplerBinding, num_bindings: Uint32) {
    let Some(c) = pass_commands(render_pass) else { return };
    // SAFETY: the renderer's bindings, as many as it says
    let bindings = unsafe { std::slice::from_raw_parts(bindings, num_bindings as usize) };
    c.push(&[code, first_slot, num_bindings]);
    for b in bindings {
        c.push(&[texture_id(b.texture), of::<Sampler, _>(b.sampler).map_or(0, |s| s.id)]);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_BindGPUFragmentSamplers(render_pass: *mut SDL_GPURenderPass, first_slot: Uint32, texture_sampler_bindings: *const SDL_GPUTextureSamplerBinding, num_bindings: Uint32) {
    bind_textures(render_pass, op::FRAGMENT_TEXTURES, first_slot, texture_sampler_bindings, num_bindings);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_BindGPUVertexSamplers(render_pass: *mut SDL_GPURenderPass, first_slot: Uint32, texture_sampler_bindings: *const SDL_GPUTextureSamplerBinding, num_bindings: Uint32) {
    bind_textures(render_pass, op::VERTEX_TEXTURES, first_slot, texture_sampler_bindings, num_bindings);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_DrawGPUPrimitives(render_pass: *mut SDL_GPURenderPass, num_vertices: Uint32, num_instances: Uint32, first_vertex: Uint32, first_instance: Uint32) {
    if let Some(c) = pass_commands(render_pass) {
        c.push(&[op::DRAW, num_vertices, num_instances, first_vertex, first_instance]);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_DrawGPUIndexedPrimitives(render_pass: *mut SDL_GPURenderPass, num_indices: Uint32, num_instances: Uint32, first_index: Uint32, vertex_offset: Sint32, first_instance: Uint32) {
    if let Some(c) = pass_commands(render_pass) {
        c.push(&[op::DRAW_INDEXED, num_indices, num_instances, first_index, vertex_offset as u32, first_instance]);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_SetGPUViewport(render_pass: *mut SDL_GPURenderPass, viewport: *const SDL_GPUViewport) {
    let Some(c) = pass_commands(render_pass) else { return };
    // SAFETY: the renderer's viewport
    let v = unsafe { &*viewport };
    c.push(&[op::VIEWPORT, v.x.to_bits(), v.y.to_bits(), v.w.to_bits(), v.h.to_bits(), v.min_depth.to_bits(), v.max_depth.to_bits()]);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_SetGPUScissor(render_pass: *mut SDL_GPURenderPass, scissor: *const SDL_Rect) {
    let Some(c) = pass_commands(render_pass) else { return };
    // SAFETY: the renderer's rectangle
    let r = unsafe { &*scissor };
    c.push(&[op::SCISSOR, r.x.max(0) as u32, r.y.max(0) as u32, r.w.max(0) as u32, r.h.max(0) as u32]);
}

/// VR's shading rate image (`render/mod.rs` declares it itself): never in the browser.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_SetGPUShadingRateImage(_command_buffer: *mut SDL_GPUCommandBuffer, _texture: *mut SDL_GPUTexture) {}

// ---- copy passes ----

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_BeginGPUCopyPass(command_buffer: *mut SDL_GPUCommandBuffer) -> *mut SDL_GPUCopyPass {
    command_buffer.cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_EndGPUCopyPass(_copy_pass: *mut SDL_GPUCopyPass) {}

fn copy_commands<'a>(pass: *mut SDL_GPUCopyPass) -> Option<&'a mut Commands> {
    commands(pass.cast())
}

/// The block a transfer buffer reads from now, held by the command buffer until it's played,
/// and where in it `offset` is.
fn hold(c: &mut Commands, buffer: *mut SDL_GPUTransferBuffer, offset: Uint32) -> Option<(u32, usize)> {
    let t = of::<TransferBuffer, _>(buffer)?;
    let block = t.data.borrow().clone();
    // SAFETY: the block stays while `c` holds it
    let base = unsafe { (*block.get()).as_ptr() } as usize;
    c.holds.push(block);
    Some(((base + offset as usize) as u32, t.size.saturating_sub(offset as usize)))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_UploadToGPUBuffer(copy_pass: *mut SDL_GPUCopyPass, source: *const SDL_GPUTransferBufferLocation, destination: *const SDL_GPUBufferRegion, _cycle: bool) {
    let Some(c) = copy_commands(copy_pass) else { return };
    // SAFETY: the renderer's locations
    let (s, d) = unsafe { (&*source, &*destination) };
    let Some((at, _)) = hold(c, s.transfer_buffer, s.offset) else { return };
    let Some(b) = of::<Buffer, _>(d.buffer) else { return };
    c.push(&[op::UPLOAD_BUFFER, at, d.size, b.id, d.offset]);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_UploadToGPUTexture(copy_pass: *mut SDL_GPUCopyPass, source: *const SDL_GPUTextureTransferInfo, destination: *const SDL_GPUTextureRegion, _cycle: bool) {
    let Some(c) = copy_commands(copy_pass) else { return };
    // SAFETY: the renderer's locations
    let (s, d) = unsafe { (&*source, &*destination) };
    let Some(t) = of::<Texture, _>(d.texture) else { return };
    let Some((at, available)) = hold(c, s.transfer_buffer, s.offset) else { return };
    let texel = format_info(t.format).map_or(4, |(_, size)| size);
    let row = if s.pixels_per_row > 0 { s.pixels_per_row } else { d.w } * texel;
    let rows = if s.rows_per_layer > 0 { s.rows_per_layer } else { d.h };
    let depth = d.d.max(1);
    let length = (row * rows * (depth - 1) + row * (d.h.max(1) - 1) + d.w * texel).min(available as u32);
    c.push(&[op::UPLOAD_TEXTURE, at, length, row, rows, t.id, d.mip_level, d.layer, d.x, d.y, d.z, d.w, d.h, depth]);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_CopyGPUTextureToTexture(copy_pass: *mut SDL_GPUCopyPass, source: *const SDL_GPUTextureLocation, destination: *const SDL_GPUTextureLocation, w: Uint32, h: Uint32, d: Uint32, _cycle: bool) {
    let Some(c) = copy_commands(copy_pass) else { return };
    // SAFETY: the renderer's locations
    let (s, t) = unsafe { (&*source, &*destination) };
    c.push(&[op::COPY_TEXTURE, texture_id(s.texture), s.mip_level, s.layer, s.x, s.y, s.z, texture_id(t.texture), t.mip_level, t.layer, t.x, t.y, t.z, w, h, d.max(1)]);
}

/// The player's captures read the picture back; the browser's never ask for one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_DownloadFromGPUTexture(_copy_pass: *mut SDL_GPUCopyPass, _source: *const SDL_GPUTextureRegion, _destination: *const SDL_GPUTextureTransferInfo) {
    super::once("SDL_DownloadFromGPUTexture: WebGPU's pictures aren't read back in the browser");
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn SDL_GenerateMipmapsForGPUTexture(command_buffer: *mut SDL_GPUCommandBuffer, texture: *mut SDL_GPUTexture) {
    if let (Some(c), Some(t)) = (commands(command_buffer), of::<Texture, _>(texture)) {
        c.push(&[op::MIPMAPS, t.id]);
    }
}

/// The canvas's size in pixels now.
pub fn canvas_size() -> (u32, u32) {
    let (mut w, mut h) = (0, 0);
    // SAFETY: a plain call into the page
    unsafe { gpu_canvas_size(&mut w, &mut h) };
    (w, h)
}
