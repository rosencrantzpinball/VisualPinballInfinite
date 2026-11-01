//! Pictures standing in the room, drawn into the headset's eye pictures (`vr.rs`), ours: the library's
//! panel and the pause menu, which reach the runtime as the tables do, in the eyes' own layer (as the
//! runtime's quad layers before, while blocks over the panel were blamed on SteamVR's showing of them;
//! they were Chromium's own drawing: `crate::panel`, `docs/steam-frame/PLAN.md` decision 20). Each picture
//! is uploaded with its mipmaps and sampled trilinear with anisotropy, and its edges are smoothed by how
//! much of each pixel they cover.

use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use sdl3_sys::everything::*;

use super::{device_shader, non_null, sdl_error};
use crate::math::{Mat4, Vec3};

const ROOM_HLSL: &str = include_str!("room.hlsl");

/// How far beyond a quad's edges it's drawn, as a share of its size: room for the smoothing of its
/// edges, a pixel or two at any distance it's seen from.
const MARGIN: f32 = 0.01;

/// A quad this frame: which picture, and where it stands in the room — its centre, its turn (x, y, z,
/// w), its width and height in metres. Its front (+z) faces the viewer, its picture's top left at its
/// top left.
pub struct RoomQuad {
    pub picture: usize,
    pub position: [f32; 3],
    pub orientation: [f32; 4],
    pub size: (f32, f32),
}

/// A picture on the graphics card, with its mipmaps.
struct Picture {
    texture: *mut SDL_GPUTexture,
    size: (u32, u32),
    /// The texture's colours blue first (BGRA, as the panel's come); else red first.
    bgra: bool,
    /// Its alpha taken (straight): the pause menu's corners see through; the panel is shown whole.
    alpha: bool,
}

/// The pictures in the room, and what draws them.
pub struct RoomPictures {
    device: *mut SDL_GPUDevice,
    pictures: Vec<Option<Picture>>,
    sampler: *mut SDL_GPUSampler,
    pipelines: Vec<(SDL_GPUTextureFormat, *mut SDL_GPUGraphicsPipeline)>,
    transfer: *mut SDL_GPUTransferBuffer,
    transfer_size: u32,
}

#[repr(C)]
struct QuadPlace {
    corners: [[f32; 4]; 4],
    uv: [f32; 4],
}

impl RoomPictures {
    /// Room for `slots` pictures on `device`, none uploaded yet.
    pub fn new(device: *mut SDL_GPUDevice, slots: usize) -> Result<RoomPictures, String> {
        // SAFETY: the device is live
        let sampler = unsafe {
            let mut info: SDL_GPUSamplerCreateInfo = zeroed();
            info.min_filter = SDL_GPU_FILTER_LINEAR;
            info.mag_filter = SDL_GPU_FILTER_LINEAR;
            info.mipmap_mode = SDL_GPU_SAMPLERMIPMAPMODE_LINEAR;
            info.address_mode_u = SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE;
            info.address_mode_v = SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE;
            info.address_mode_w = SDL_GPU_SAMPLERADDRESSMODE_CLAMP_TO_EDGE;
            info.max_lod = 1000.0;
            info.enable_anisotropy = true;
            info.max_anisotropy = 16.0;
            non_null(SDL_CreateGPUSampler(device, &info), "create the room's sampler")?
        };
        Ok(RoomPictures { device, pictures: (0..slots).map(|_| None).collect(), sampler, pipelines: Vec::new(), transfer: null_mut(), transfer_size: 0 })
    }

    /// Whether `slot` has a picture.
    pub fn has(&self, slot: usize) -> bool {
        self.pictures.get(slot).is_some_and(Option::is_some)
    }

    /// `slot`'s picture from now on: `pixels`, 4 bytes each, blue first where `bgra`, their alpha taken
    /// (straight) where `alpha`. Uploaded with its mipmaps; frames already drawn keep the picture before.
    pub fn set(&mut self, slot: usize, pixels: &[u8], width: u32, height: u32, bgra: bool, alpha: bool) -> Result<(), String> {
        let bytes = width * height * 4;
        if width == 0 || height == 0 || slot >= self.pictures.len() || pixels.len() < bytes as usize {
            return Ok(());
        }
        // SAFETY: the device is live; the texture and the transfer buffer are ours
        unsafe {
            if self.pictures[slot].as_ref().is_none_or(|p| p.size != (width, height)) {
                if let Some(p) = self.pictures[slot].take() {
                    SDL_ReleaseGPUTexture(self.device, p.texture);
                }
                let usage = SDL_GPU_TEXTUREUSAGE_SAMPLER | SDL_GPU_TEXTUREUSAGE_COLOR_TARGET;
                let blue_first = bgra && SDL_GPUTextureSupportsFormat(self.device, SDL_GPU_TEXTUREFORMAT_B8G8R8A8_UNORM_SRGB, SDL_GPU_TEXTURETYPE_2D, usage);
                let mut info: SDL_GPUTextureCreateInfo = zeroed();
                info.r#type = SDL_GPU_TEXTURETYPE_2D;
                info.format = if blue_first { SDL_GPU_TEXTUREFORMAT_B8G8R8A8_UNORM_SRGB } else { SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB };
                info.usage = usage;
                info.width = width;
                info.height = height;
                info.layer_count_or_depth = 1;
                // every mipmap down to a pixel
                info.num_levels = 32 - width.max(height).leading_zeros();
                info.sample_count = SDL_GPU_SAMPLECOUNT_1;
                let texture = non_null(SDL_CreateGPUTexture(self.device, &info), "create a room picture's texture")?;
                self.pictures[slot] = Some(Picture { texture, size: (width, height), bgra: blue_first, alpha });
            }
            if self.transfer.is_null() || self.transfer_size < bytes {
                if !self.transfer.is_null() {
                    SDL_ReleaseGPUTransferBuffer(self.device, self.transfer);
                    self.transfer = null_mut();
                }
                let mut info: SDL_GPUTransferBufferCreateInfo = zeroed();
                info.usage = SDL_GPU_TRANSFERBUFFERUSAGE_UPLOAD;
                info.size = bytes;
                self.transfer =non_null(SDL_CreateGPUTransferBuffer(self.device, &info), "create the room's transfer buffer")?;
                self.transfer_size = bytes;
            }
            let Some(picture) = self.pictures[slot].as_mut() else { return Ok(()) };
            picture.alpha = alpha;
            // a fresh buffer when the last upload is still being read
            let mapped = non_null(SDL_MapGPUTransferBuffer(self.device, self.transfer, true).cast::<u8>(), "map the room's transfer buffer")?;
            let into = std::slice::from_raw_parts_mut(mapped, bytes as usize);
            if picture.bgra == bgra {
                into.copy_from_slice(&pixels[..bytes as usize]);
            } else {
                for (to, from) in into.chunks_exact_mut(4).zip(pixels.chunks_exact(4)) {
                    to.copy_from_slice(&[from[2], from[1], from[0], from[3]]);
                }
            }
            SDL_UnmapGPUTransferBuffer(self.device, self.transfer);
            let command = non_null(SDL_AcquireGPUCommandBuffer(self.device), "acquire a command buffer for a room picture")?;
            let pass = SDL_BeginGPUCopyPass(command);
            let source = SDL_GPUTextureTransferInfo { transfer_buffer: self.transfer, offset: 0, pixels_per_row: width, rows_per_layer: height };
            let mut region: SDL_GPUTextureRegion = zeroed();
            region.texture = picture.texture;
            region.w = width;
            region.h = height;
            region.d = 1;
            // into a fresh texture when frames still to be shown read this one
            SDL_UploadToGPUTexture(pass, &source, &region, true);
            SDL_EndGPUCopyPass(pass);
            SDL_GenerateMipmapsForGPUTexture(command, picture.texture);
            if !SDL_SubmitGPUCommandBuffer(command) {
                return Err(format!("upload a room picture: {}", sdl_error()));
            }
        }
        Ok(())
    }

    /// Draws `quads` (those with pictures) into each eye's image, `targets` (`format`, the top left
    /// `size` pixels of each its picture), as seen through `eyes` (each the room into that eye's clip
    /// space): cleared first to `clear`, else over what the image holds.
    pub fn draw(&mut self, targets: [*mut SDL_GPUTexture; 2], format: SDL_GPUTextureFormat, size: (u32, u32), clear: Option<SDL_FColor>, eyes: [Mat4; 2], quads: &[RoomQuad]) -> Result<(), String> {
        let pipeline = self.pipeline(format)?;
        // a target holding sRGB values takes the picture's linear colours encoded
        let encode = super::embed::holds_srgb_values(format);
        // SAFETY: the device is live; the eyes' images are acquired for this frame; the rest is ours
        unsafe {
            let command = non_null(SDL_AcquireGPUCommandBuffer(self.device), "acquire a command buffer for the room")?;
            for (eye, &target) in targets.iter().enumerate() {
                let mut color: SDL_GPUColorTargetInfo = zeroed();
                color.texture = target;
                color.load_op = if clear.is_some() { SDL_GPU_LOADOP_CLEAR } else { SDL_GPU_LOADOP_LOAD };
                color.clear_color = clear.unwrap_or(SDL_FColor { r: 0.0, g: 0.0, b: 0.0, a: 0.0 });
                color.store_op = SDL_GPU_STOREOP_STORE;
                let pass = SDL_BeginGPURenderPass(command, &color, 1, null());
                if pass.is_null() {
                    let e = format!("draw the room: {}", sdl_error());
                    SDL_CancelGPUCommandBuffer(command);
                    return Err(e);
                }
                let viewport = SDL_GPUViewport { x: 0.0, y: 0.0, w: size.0 as f32, h: size.1 as f32, min_depth: 0.0, max_depth: 1.0 };
                SDL_SetGPUViewport(pass, &viewport);
                SDL_SetGPUScissor(pass, &SDL_Rect { x: 0, y: 0, w: size.0 as i32, h: size.1 as i32 });
                SDL_BindGPUGraphicsPipeline(pass, pipeline);
                for q in quads {
                    let Some(p) = self.pictures.get(q.picture).and_then(Option::as_ref) else { continue };
                    let place = place(q, &eyes[eye]);
                    SDL_PushGPUVertexUniformData(command, 0, (&raw const place).cast(), size_of::<QuadPlace>() as u32);
                    let look = [if p.alpha { 1.0 } else { 0.0 }, if encode { 1.0 } else { 0.0 }, 0.0, 0.0f32];
                    SDL_PushGPUFragmentUniformData(command, 0, look.as_ptr().cast(), 16);
                    let binding = SDL_GPUTextureSamplerBinding { texture: p.texture, sampler: self.sampler };
                    SDL_BindGPUFragmentSamplers(pass, 0, &binding, 1);
                    SDL_DrawGPUPrimitives(pass, 6, 1, 0, 0);
                }
                SDL_EndGPURenderPass(pass);
            }
            if !SDL_SubmitGPUCommandBuffer(command) {
                return Err(format!("draw the room: {}", sdl_error()));
            }
        }
        Ok(())
    }

    /// The pipeline drawing into `format`, made the first time: each picture blended over by its
    /// coverage (and its alpha, where taken).
    fn pipeline(&mut self, format: SDL_GPUTextureFormat) -> Result<*mut SDL_GPUGraphicsPipeline, String> {
        if let Some(&(_, p)) = self.pipelines.iter().find(|(f, _)| *f == format) {
            return Ok(p);
        }
        let vs = device_shader(self.device, ROOM_HLSL, c"room_vs", c"vs_5_1", SDL_GPU_SHADERSTAGE_VERTEX, 0, 1)?;
        let fs = match device_shader(self.device, ROOM_HLSL, c"room_fs", c"ps_5_1", SDL_GPU_SHADERSTAGE_FRAGMENT, 1, 1) {
            Ok(fs) => fs,
            Err(e) => {
                // SAFETY: made above, not used
                unsafe { SDL_ReleaseGPUShader(self.device, vs) };
                return Err(e);
            }
        };
        // SAFETY: the device is live; the shaders are released once the pipeline holds them
        let pipeline = unsafe {
            let mut color: SDL_GPUColorTargetDescription = zeroed();
            color.format = format;
            color.blend_state.enable_blend = true;
            color.blend_state.src_color_blendfactor = SDL_GPU_BLENDFACTOR_SRC_ALPHA;
            color.blend_state.dst_color_blendfactor = SDL_GPU_BLENDFACTOR_ONE_MINUS_SRC_ALPHA;
            color.blend_state.color_blend_op = SDL_GPU_BLENDOP_ADD;
            color.blend_state.src_alpha_blendfactor = SDL_GPU_BLENDFACTOR_ONE;
            color.blend_state.dst_alpha_blendfactor = SDL_GPU_BLENDFACTOR_ONE_MINUS_SRC_ALPHA;
            color.blend_state.alpha_blend_op = SDL_GPU_BLENDOP_ADD;
            let mut info: SDL_GPUGraphicsPipelineCreateInfo = zeroed();
            info.vertex_shader = vs;
            info.fragment_shader = fs;
            info.primitive_type = SDL_GPU_PRIMITIVETYPE_TRIANGLELIST;
            info.rasterizer_state.fill_mode = SDL_GPU_FILLMODE_FILL;
            info.rasterizer_state.cull_mode = SDL_GPU_CULLMODE_NONE;
            info.multisample_state.sample_count = SDL_GPU_SAMPLECOUNT_1;
            info.target_info.color_target_descriptions = &color;
            info.target_info.num_color_targets = 1;
            let pipeline = SDL_CreateGPUGraphicsPipeline(self.device, &info);
            SDL_ReleaseGPUShader(self.device, vs);
            SDL_ReleaseGPUShader(self.device, fs);
            non_null(pipeline, "create the room's pipeline")?
        };
        self.pipelines.push((format, pipeline));
        Ok(pipeline)
    }
}

impl Drop for RoomPictures {
    fn drop(&mut self) {
        // SAFETY: ours; SDL keeps each until the frames using it are done
        unsafe {
            for p in self.pictures.drain(..).flatten() {
                SDL_ReleaseGPUTexture(self.device, p.texture);
            }
            for (_, p) in self.pipelines.drain(..) {
                SDL_ReleaseGPUGraphicsPipeline(self.device, p);
            }
            if !self.transfer.is_null() {
                SDL_ReleaseGPUTransferBuffer(self.device, self.transfer);
            }
            SDL_ReleaseGPUSampler(self.device, self.sampler);
        }
    }
}

/// `texture`'s top left `size` pixels (`format`, 4 bytes each) as RGBA, read back from the graphics card
/// once what's drawn into it is done: test runs' pictures of the eyes.
pub fn read_back(device: *mut SDL_GPUDevice, texture: *mut SDL_GPUTexture, size: (u32, u32), format: SDL_GPUTextureFormat) -> Result<Vec<u8>, String> {
    let bytes = size.0 * size.1 * 4;
    // SAFETY: the device is live; the texture is drawn on it; the transfer buffer is ours
    unsafe {
        let mut info: SDL_GPUTransferBufferCreateInfo = zeroed();
        info.usage = SDL_GPU_TRANSFERBUFFERUSAGE_DOWNLOAD;
        info.size = bytes;
        let transfer =non_null(SDL_CreateGPUTransferBuffer(device, &info), "create a buffer to read a picture back")?;
        let read = || -> Result<Vec<u8>, String> {
            let command = non_null(SDL_AcquireGPUCommandBuffer(device), "acquire a command buffer to read a picture back")?;
            let pass = SDL_BeginGPUCopyPass(command);
            let mut region: SDL_GPUTextureRegion = zeroed();
            region.texture = texture;
            region.w = size.0;
            region.h = size.1;
            region.d = 1;
            let to = SDL_GPUTextureTransferInfo { transfer_buffer: transfer, offset: 0, pixels_per_row: size.0, rows_per_layer: size.1 };
            SDL_DownloadFromGPUTexture(pass, &region, &to);
            SDL_EndGPUCopyPass(pass);
            let fence = non_null(SDL_SubmitGPUCommandBufferAndAcquireFence(command), "read a picture back")?;
            let done = SDL_WaitForGPUFences(device, true, &fence, 1);
            SDL_ReleaseGPUFence(device, fence);
            if !done {
                return Err(format!("read a picture back: {}", sdl_error()));
            }
            let mapped = non_null(SDL_MapGPUTransferBuffer(device, transfer, false).cast::<u8>(), "map a picture read back")?;
            let mut rgba = std::slice::from_raw_parts(mapped, bytes as usize).to_vec();
            SDL_UnmapGPUTransferBuffer(device, transfer);
            if format == SDL_GPU_TEXTUREFORMAT_B8G8R8A8_UNORM || format == SDL_GPU_TEXTUREFORMAT_B8G8R8A8_UNORM_SRGB {
                rgba.chunks_exact_mut(4).for_each(|p| p.swap(0, 2));
            }
            Ok(rgba)
        };
        let rgba = read();
        SDL_ReleaseGPUTransferBuffer(device, transfer);
        rgba
    }
}

/// A quad's corners in an eye's clip space (`eye`: the room into it), its margin included, and where
/// they fall on its picture.
fn place(q: &RoomQuad, eye: &Mat4) -> QuadPlace {
    let to_clip = Mat4::from_quaternion(q.orientation) * Mat4::translate(q.position[0], q.position[1], q.position[2]) * *eye;
    let (near, far) = (-MARGIN, 1.0 + MARGIN);
    let corners = [(near, near), (far, near), (near, far), (far, far)].map(|(u, v)| clip(&to_clip, Vec3::new((u - 0.5) * q.size.0, (0.5 - v) * q.size.1, 0.0)));
    QuadPlace { corners, uv: [near, near, far, far] }
}

/// A point through the matrix, its w kept: a position in clip space.
fn clip(m: &Mat4, v: Vec3) -> [f32; 4] {
    let m = &m.m;
    [0, 1, 2, 3].map(|j| m[0][j] * v.x + m[1][j] * v.y + m[2][j] * v.z + m[3][j])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An eye at the room's origin turned by `orientation` (looking down -z unturned), 90 degrees across
    /// both ways.
    fn eye(orientation: [f32; 4]) -> Mat4 {
        let quarter = std::f32::consts::FRAC_PI_4;
        let fov = crate::xr::place::Fov { left: -quarter, right: quarter, up: quarter, down: -quarter };
        let pose = crate::xr::place::Pose { position: [0.0; 3], orientation };
        pose.view_matrix() * crate::xr::place::projection(fov, 0.05, 100.0)
    }

    fn ndc(c: [f32; 4]) -> (f32, f32, f32) {
        (c[0] / c[3], c[1] / c[3], c[2] / c[3])
    }

    #[test]
    fn a_quad_ahead_fills_the_middle_its_top_left_up_and_left() {
        // 2 m wide and high, 2 m ahead: from -0.5 to 0.5 of the eye's picture both ways
        let q = RoomQuad { picture: 0, position: [0.0, 0.0, -2.0], orientation: [0.0, 0.0, 0.0, 1.0], size: (2.0, 2.0) };
        let p = place(&q, &eye([0.0, 0.0, 0.0, 1.0]));
        let grown = 0.5 * (1.0 + 2.0 * MARGIN);
        let (x, y, z) = ndc(p.corners[0]);
        assert!((x + grown).abs() < 1e-4 && (y - grown).abs() < 1e-4, "top left at ({x}, {y})");
        assert!(z > 0.0 && z < 1.0, "depth {z}");
        let (x, y, _) = ndc(p.corners[3]);
        assert!((x - grown).abs() < 1e-4 && (y + grown).abs() < 1e-4, "bottom right at ({x}, {y})");
        assert_eq!(p.uv, [-MARGIN, -MARGIN, 1.0 + MARGIN, 1.0 + MARGIN]);
    }

    #[test]
    fn a_quad_to_the_right_faces_an_eye_turned_to_it() {
        // 2 m to the right (+x), turned a quarter about y so its front faces the origin (-x); an eye
        // turned the same quarter looks at it (+x)
        let quarter = [0.0, -std::f32::consts::FRAC_PI_4.sin(), 0.0, std::f32::consts::FRAC_PI_4.cos()];
        let q = RoomQuad { picture: 0, position: [2.0, 0.0, 0.0], orientation: quarter, size: (1.0, 1.0) };
        let p = place(&q, &eye(quarter));
        // its middle in the eye's middle
        let mid = [0, 1, 2, 3].map(|i| p.corners.iter().map(|c| c[i]).sum::<f32>() / 4.0);
        let (x, y, _) = ndc(mid);
        assert!(x.abs() < 1e-4 && y.abs() < 1e-4, "middle at ({x}, {y})");
        // its picture's left edge on the eye's left, its top up
        let (left, top, _) = ndc(p.corners[0]);
        let (right, _, _) = ndc(p.corners[1]);
        let (_, bottom, _) = ndc(p.corners[2]);
        assert!(left < right && top > bottom, "left {left}, right {right}, top {top}, bottom {bottom}");
    }
}
