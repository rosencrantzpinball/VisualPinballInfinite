//! On Windows, builds the player's one piece of C++: the bridge to FFmpeg for PinUP packs' media
//! (`native/pup_av.cpp`), compiled against FFmpeg's headers in `vendor/ffmpeg`. FFmpeg
//! itself isn't linked: the bridge loads its libraries when a pack plays.
//!
//! Elsewhere (Linux, for the Steam Frame), compiles the player's HLSL shaders to SPIR-V with DXC,
//! for SDL_GPU's Vulkan backend: on Windows FXC compiles them as the player starts
//! (`src/render/fxc.rs`); Linux has no FXC, so every entry point the renderer asks for is compiled
//! here and built into the program (`src/render/spirv.rs` finds them). DXC is `dxc` on the path,
//! or the one `VPI_DXC` names.
//!
//! In the browser (wasm32, `engine/web`, with this crate's `web` feature), each SPIR-V shader is made
//! WGSL for WebGPU by naga, in its place: the web crate's SDL hands WGSL to WebGPU, with what its
//! pipelines must know of each shader (its bindings and outputs) in a header of comments.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        ffmpeg_bridge();
    } else if std::env::var("CARGO_CFG_TARGET_FAMILY").is_ok_and(|f| f.split(',').any(|f| f == "wasm")) {
        shaders(Output::Wgsl);
    } else {
        shaders(Output::Spirv);
    }
}

/// What the shaders are built as: SPIR-V for Vulkan, or WGSL for the browser's WebGPU.
#[derive(Clone, Copy, PartialEq)]
enum Output {
    Spirv,
    Wgsl,
}

fn ffmpeg_bridge() {
    println!("cargo:rerun-if-changed=native/pup_av.cpp");
    println!("cargo:rerun-if-changed=../../vendor/ffmpeg/include");
    cc::Build::new()
        .cpp(true)
        .file("native/pup_av.cpp")
        .include("../../vendor/ffmpeg/include")
        .flag_if_supported("/std:c++17")
        .warnings(true)
        .compile("vpi_pup_av");
}

/// A piece of a shader's source as the renderer puts it together: a file of `src/render`, or text
/// it adds (a `#define` before a file, a line between two).
enum Part {
    File(&'static str),
    Text(&'static str),
}
use Part::{File, Text};

/// Every source the renderer compiles, put together exactly as it does (`render/*.rs`), with the
/// entry points it asks of each. A shader whose source or entry point isn't here can't be made on
/// Linux: `spirv::compiled` says which. Left out: the HDR10 picture's shaders (`*_pq_fs`, HDR10
/// is off on Linux).
const SOURCES: &[(&[Part], &[&str])] = &[
    (&[File("player.hlsl")], &["vs_main", "vs_kicker", "vs_null", "vs_position", "fs_main", "fs_light", "fs_bulb", "fs_flasher", "fs_unshaded"]),
    // the parts as drawn outside the mirrors' passes: without the hardware clip distance
    // (`Renderer::part_shader`)
    (&[Text("#define NO_CLIP\n"), File("player.hlsl")], &["vs_main", "vs_kicker", "vs_null", "vs_position", "fs_main", "fs_light", "fs_bulb", "fs_flasher", "fs_unshaded"]),
    (&[Text("#define NO_CLIP\n#define DMD_SCENE\n"), File("player.hlsl"), Text("\n"), File("dmd.hlsl")], &["fs_dmd_flasher"]),
    (&[Text("#define NO_CLIP\n"), File("material.hlsl")], &["fs_vpx", "fs_vpx_reflection"]),
    (&[Text("#define NO_CLIP\n#define REFRACTION\n"), File("material.hlsl")], &["fs_vpx"]),
    (&[Text("#define NO_CLIP\n#define BALL\n"), File("material.hlsl")], &["fs_vpx_ball"]),
    (&[Text("#define NO_CLIP\n#define CLASSIC_LIGHT\n"), File("material.hlsl")], &["fs_vpx_classic_light"]),
    (&[File("overlay.hlsl")], &["overlay_vs", "overlay_fs", "sprite_fs", "backdrop_light_fs", "backdrop_bulb_fs", "backglass_fs"]),
    (&[Text("#define DMD_SCENE\n"), File("player.hlsl"), Text("\n"), File("dmd.hlsl")], &["fs_dmd_flasher"]),
    (&[Text("#define DMD_OVERLAY\n"), File("overlay.hlsl"), Text("\n"), File("dmd.hlsl")], &["dmd_backdrop_fs"]),
    (&[File("display.hlsl")], &["display_vs", "display_dmd_fs", "display_seg_fs"]),
    (&[File("tonemap.hlsl")], &["tonemap_vs", "tonemap_fs", "tonemap_environment_alpha_fs"]),
    (&[File("material.hlsl")], &["fs_vpx", "fs_vpx_reflection"]),
    (&[Text("#define REFRACTION\n"), File("material.hlsl")], &["fs_vpx"]),
    (&[Text("#define BALL\n"), File("material.hlsl")], &["fs_vpx_ball"]),
    (&[Text("#define CLASSIC_LIGHT\n"), File("material.hlsl")], &["fs_vpx_classic_light"]),
    (&[Text("#define IRRADIANCE\n"), File("material.hlsl")], &["irradiance_vs", "irradiance_fs"]),
    (
        &[File("bloom.hlsl")],
        &["bloom_vs", "bloom_cut_fs", "blur_h_fs", "blur_v_fs", "blur19_h_fs", "blur19_v_fs", "blur_kernel_h_fs", "blur_kernel_v_fs"],
    ),
    (&[File("embed.hlsl")], &["embed_vs", "embed_fill_fs", "embed_copy_fs"]),
    (&[File("effects.hlsl")], &["effect_vs", "ao_fs", "ao_apply_fs", "motion_blur_fs"]),
    (&[File("still.hlsl")], &["still_vs", "still_accumulate_fs", "still_restore_fs", "still_restore_color_fs"]),
    (&[File("menu.hlsl")], &["menu_vs", "menu_fs"]),
    (&[File("room.hlsl")], &["room_vs", "room_fs"]),
    (
        &[File("post_common.hlsl"), File("post.hlsl")],
        &["post_vs", "fxaa1_fs", "fxaa2_fs", "fxaa3_fs", "faaa_fs", "nfaa_fs", "dlaa_edge_fs", "dlaa_fs", "cas_fs", "bilateral_cas_fs", "environment_alpha_fs"],
    ),
    (
        &[File("post_common.hlsl"), File("smaa.hlsl"), File("smaa_passes.hlsl")],
        &["smaa_edge_vs", "smaa_edge_fs", "smaa_weight_vs", "smaa_weight_fs", "smaa_blend_vs", "smaa_blend_fs"],
    ),
];

fn shaders(output: Output) {
    let render = Path::new("src/render");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let dxc = std::env::var_os("VPI_DXC").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("dxc"));
    println!("cargo:rerun-if-env-changed=VPI_DXC");
    println!("cargo:rerun-if-changed=src/render");

    let mut table = String::from(match output {
        Output::Spirv => "// Made by build.rs: each shader as DXC compiled it to SPIR-V, by its source's hash and its entry point.\n",
        Output::Wgsl => "// Made by build.rs: each shader as DXC compiled it to SPIR-V and naga made it WGSL, by its source's hash and its entry point.\n",
    });
    table.push_str("pub static SPIRV: &[(u64, &str, &[u8])] = &[\n");
    for (n, (parts, entries)) in SOURCES.iter().enumerate() {
        let mut source = String::new();
        for part in parts.iter() {
            match part {
                File(name) => source.push_str(&std::fs::read_to_string(render.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))),
                Text(text) => source.push_str(text),
            }
        }
        let key = source_hash(&source);
        let hlsl = out.join(format!("shader{n}.hlsl"));
        std::fs::write(&hlsl, if output == Output::Wgsl { for_webgpu(&source) } else { for_vulkan(&source) }).unwrap();
        for entry in entries.iter() {
            let profile = if entry.starts_with("vs_") || entry.ends_with("_vs") { "vs_6_0" } else { "ps_6_0" };
            let spv = out.join(format!("shader{n}.{entry}.spv"));
            dxc_compile(&dxc, profile, entry, &hlsl, &spv);
            let code = match output {
                Output::Spirv => spv,
                Output::Wgsl => {
                    let wgsl = out.join(format!("shader{n}.{entry}.wgsl"));
                    let mut text = to_wgsl(&std::fs::read(&spv).unwrap(), entry);
                    // a mirror's vertex shader again for a WebGPU without clip distances: the
                    // distance an output nothing reads, the mirror unclipped
                    if text.contains("enable clip_distances;") {
                        let plain = out.join(format!("shader{n}.{entry}.unclipped.hlsl"));
                        std::fs::write(&plain, for_webgpu(&source.replace(": SV_ClipDistance0", ": TEXCOORD7"))).unwrap();
                        let spv = out.join(format!("shader{n}.{entry}.unclipped.spv"));
                        dxc_compile(&dxc, profile, entry, &plain, &spv);
                        text.push_str(WITHOUT_CLIP_DISTANCES);
                        text.push_str(&to_wgsl(&std::fs::read(&spv).unwrap(), entry));
                    }
                    std::fs::write(&wgsl, text).unwrap();
                    wgsl
                }
            };
            writeln!(table, "    (0x{key:016x}, {entry:?}, include_bytes!({:?})),", code.display().to_string()).unwrap();
        }
    }
    table.push_str("];\n");
    std::fs::write(out.join("spirv.rs"), table).unwrap();
}

/// `entry` of the HLSL file `hlsl` compiled by DXC to SPIR-V in `spv`.
fn dxc_compile(dxc: &Path, profile: &str, entry: &str, hlsl: &Path, spv: &Path) {
    let run = Command::new(dxc)
        // HLSL 2018, the language FXC compiles (a vector `?:` picks per component), and
        // FXC's packing of constant buffers, which the renderer fills
        .args(["-spirv", "-HV", "2018", "-fvk-use-dx-layout", "-O3", "-T", profile, "-E", entry, "-Fo"])
        .arg(spv)
        .arg(hlsl)
        .output()
        .unwrap_or_else(|e| panic!("DXC ({}) could not run: {e}; set VPI_DXC to the HLSL compiler", dxc.display()));
    if !run.status.success() {
        panic!("DXC could not compile {entry} of {}:\n{}", hlsl.display(), String::from_utf8_lossy(&run.stderr));
    }
}

/// Where a shader's WGSL for a WebGPU without clip distances follows its own (`site/gpu.js` takes
/// the one the device can compile).
const WITHOUT_CLIP_DISTANCES: &str = "\n//// vpi without clip distances\n";

/// FNV-1a over the source's bytes, as `spirv::compiled` reckons it.
fn source_hash(source: &str) -> u64 {
    source.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// The source with what Vulkan needs added, the HLSL itself unchanged:
/// - each texture and its sampler, declared at the same register (`t0` and `s0` of a space), made
///   one combined image sampler at that binding of the space's set: SDL's Vulkan backend binds a
///   texture and its sampler together, in set 0 for vertex shaders and set 2 for fragment shaders,
///   the spaces our declarations already name for its Direct3D 12 backend;
/// - each `TEXCOORDn` given location n: SDL's Direct3D 12 backend names a vertex attribute at
///   location n `TEXCOORDn`, and Direct3D matches a stage's outputs to the next stage's inputs by
///   name, where Vulkan matches them by location.
fn for_vulkan(source: &str) -> String {
    let mut out = String::with_capacity(source.len() + 4096);
    for line in source.lines() {
        let trimmed = line.trim_start();
        let resource = (trimmed.starts_with("Texture") || trimmed.starts_with("SamplerState"))
            .then(|| register(trimmed))
            .flatten();
        if let Some((slot, space)) = resource {
            write!(out, "[[vk::combinedImageSampler]] [[vk::binding({slot}, {space})]] ").unwrap();
        } else if let Some(at) = trimmed.find(": TEXCOORD") {
            let index: String = trimmed[at + ": TEXCOORD".len()..].chars().take_while(char::is_ascii_digit).collect();
            if !index.is_empty() && !trimmed.starts_with('#') && !trimmed.starts_with("//") {
                write!(out, "[[vk::location({index})]] ").unwrap();
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// `(n, space)` of a declaration's `register(t<n>, space<space>)` or `register(s<n>, …)`.
fn register(line: &str) -> Option<(u32, u32)> {
    let at = line.find("register(")? + "register(".len();
    let inside = &line[at..line[at..].find(')')? + at];
    let (slot, space) = inside.split_once(',')?;
    let slot = slot.trim().strip_prefix('t').or_else(|| slot.trim().strip_prefix('s'))?.parse().ok()?;
    let space = space.trim().strip_prefix("space")?.parse().ok()?;
    Some((slot, space))
}

/// The source with what WebGPU needs added, the HLSL itself unchanged: each texture and its sampler,
/// declared at the same register of a space, at two bindings of the space's bind group, the texture's
/// 2n and its sampler's 2n + 1 (WebGPU has no combined image samplers); each `TEXCOORDn` given
/// location n, as for Vulkan. The uniform buffers keep their register's binding, the space's group:
/// groups 0 and 1 the vertex shader's textures and uniforms, 2 and 3 the fragment shader's, as SDL's
/// spaces have them.
fn for_webgpu(source: &str) -> String {
    let mut out = String::with_capacity(source.len() + 4096);
    for line in source.lines() {
        let trimmed = line.trim_start();
        let texture = trimmed.starts_with("Texture");
        let resource = (texture || trimmed.starts_with("SamplerState")).then(|| register(trimmed)).flatten();
        if let Some((slot, space)) = resource {
            write!(out, "[[vk::binding({}, {space})]] ", 2 * slot + u32::from(!texture)).unwrap();
        } else if let Some(at) = trimmed.find(": TEXCOORD") {
            let index: String = trimmed[at + ": TEXCOORD".len()..].chars().take_while(char::is_ascii_digit).collect();
            if !index.is_empty() && !trimmed.starts_with('#') && !trimmed.starts_with("//") {
                write!(out, "[[vk::location({index})]] ").unwrap();
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// A shader's SPIR-V as WGSL, by naga, after a header the web crate's SDL reads to make its pipelines
/// (comments, so the WGSL is whole as it is):
///
/// - `// vpi texture <group> <binding> <dimension> <kind> <multisampled>` (kind: `float`, `sint`,
///   `uint` or `depth`), `// vpi sampler <group> <binding> <comparison>`, `// vpi uniform <group>
///   <binding> <size>`: what each binding holds;
/// - `// vpi output <location>`: each colour the fragment shader writes.
///
/// The WGSL turns off the uniformity analysis of derivatives: HLSL samples textures wherever it likes,
/// and the shaders are as FXC and DXC take them.
#[cfg(feature = "web")]
fn to_wgsl(spirv: &[u8], entry: &str) -> String {
    use naga::{AddressSpace, Binding, ImageClass, ImageDimension, ScalarKind, TypeInner};
    let options = naga::front::spv::Options { adjust_coordinate_space: false, strict_capabilities: false, ..Default::default() };
    let module = naga::front::spv::parse_u8_slice(spirv, &options).unwrap_or_else(|e| panic!("naga could not read {entry}'s SPIR-V: {e}"));
    let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .unwrap_or_else(|e| panic!("naga found {entry}'s SPIR-V wrong: {e:?}"));
    let wgsl = naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty()).unwrap_or_else(|e| panic!("naga could not write {entry} as WGSL: {e}"));
    let mut header = String::new();
    for (_, global) in module.global_variables.iter() {
        let Some(at) = &global.binding else { continue };
        let (group, binding) = (at.group, at.binding);
        match &module.types[global.ty].inner {
            TypeInner::Image { dim, arrayed, class } => {
                let dimension = match (dim, arrayed) {
                    (ImageDimension::D1, _) => "1d",
                    (ImageDimension::D2, false) => "2d",
                    (ImageDimension::D2, true) => "2d-array",
                    (ImageDimension::D3, _) => "3d",
                    (ImageDimension::Cube, false) => "cube",
                    (ImageDimension::Cube, true) => "cube-array",
                };
                let (kind, multi) = match class {
                    ImageClass::Sampled { kind: ScalarKind::Sint, multi } => ("sint", *multi),
                    ImageClass::Sampled { kind: ScalarKind::Uint, multi } => ("uint", *multi),
                    ImageClass::Sampled { multi, .. } => ("float", *multi),
                    ImageClass::Depth { multi } => ("depth", *multi),
                    other => panic!("{entry}: a texture WebGPU can't take here ({other:?})"),
                };
                writeln!(header, "// vpi texture {group} {binding} {dimension} {kind} {}", u8::from(multi)).unwrap();
            }
            TypeInner::Sampler { comparison } => writeln!(header, "// vpi sampler {group} {binding} {}", u8::from(*comparison)).unwrap(),
            inner if global.space == AddressSpace::Uniform => {
                let size = match inner {
                    TypeInner::Struct { span, .. } => *span,
                    other => other.size(module.to_ctx()),
                };
                writeln!(header, "// vpi uniform {group} {binding} {size}").unwrap();
            }
            other => panic!("{entry}: a binding WebGPU can't take here ({other:?})"),
        }
    }
    let point = module.entry_points.iter().find(|p| p.name == entry).unwrap_or_else(|| panic!("no entry point {entry} in its own SPIR-V"));
    if point.stage == naga::ShaderStage::Fragment
        && let Some(result) = &point.function.result
    {
        let mut locations = Vec::new();
        match (&result.binding, &module.types[result.ty].inner) {
            (Some(Binding::Location { location, .. }), _) => locations.push(*location),
            (None, TypeInner::Struct { members, .. }) => {
                locations.extend(members.iter().filter_map(|m| match m.binding {
                    Some(Binding::Location { location, .. }) => Some(location),
                    _ => None,
                }));
            }
            _ => {}
        }
        for location in locations {
            writeln!(header, "// vpi output {location}").unwrap();
        }
    }
    format!("{header}diagnostic(off, derivative_uniformity);\n\n{wgsl}")
}

#[cfg(not(feature = "web"))]
fn to_wgsl(_spirv: &[u8], _entry: &str) -> String {
    panic!("the browser's build of the player needs its `web` feature (naga, for the shaders' WGSL): build engine/web")
}
