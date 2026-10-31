//! On Windows, builds the player's one piece of C++: the bridge to FFmpeg for PinUP packs' media
//! (`native/pup_av.cpp`), compiled against FFmpeg's headers in `vendor/ffmpeg`. FFmpeg
//! itself isn't linked: the bridge loads its libraries when a pack plays.
//!
//! Elsewhere (Linux, for the Steam Frame), compiles the player's HLSL shaders to SPIR-V with DXC,
//! for SDL_GPU's Vulkan backend: on Windows FXC compiles them as the player starts
//! (`src/render/fxc.rs`); Linux has no FXC, so every entry point the renderer asks for is compiled
//! here and built into the program (`src/render/spirv.rs` finds them). DXC is `dxc` on the path,
//! or the one `VPI_DXC` names.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        ffmpeg_bridge();
    } else {
        spirv_shaders();
    }
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
    (
        &[File("post_common.hlsl"), File("post.hlsl")],
        &["post_vs", "fxaa1_fs", "fxaa2_fs", "fxaa3_fs", "faaa_fs", "nfaa_fs", "dlaa_edge_fs", "dlaa_fs", "cas_fs", "bilateral_cas_fs", "environment_alpha_fs"],
    ),
    (
        &[File("post_common.hlsl"), File("smaa.hlsl"), File("smaa_passes.hlsl")],
        &["smaa_edge_vs", "smaa_edge_fs", "smaa_weight_vs", "smaa_weight_fs", "smaa_blend_vs", "smaa_blend_fs"],
    ),
];

fn spirv_shaders() {
    let render = Path::new("src/render");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let dxc = std::env::var_os("VPI_DXC").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("dxc"));
    println!("cargo:rerun-if-env-changed=VPI_DXC");
    println!("cargo:rerun-if-changed=src/render");

    let mut table = String::from("// Made by build.rs: each shader as DXC compiled it to SPIR-V, by its source's hash and its entry point.\n");
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
        std::fs::write(&hlsl, for_vulkan(&source)).unwrap();
        for entry in entries.iter() {
            let profile = if entry.starts_with("vs_") || entry.ends_with("_vs") { "vs_6_0" } else { "ps_6_0" };
            let spv = out.join(format!("shader{n}.{entry}.spv"));
            let run = Command::new(&dxc)
                // HLSL 2018, the language FXC compiles (a vector `?:` picks per component), and
                // FXC's packing of constant buffers, which the renderer fills
                .args(["-spirv", "-HV", "2018", "-fvk-use-dx-layout", "-O3", "-T", profile, "-E", entry, "-Fo"])
                .arg(&spv)
                .arg(&hlsl)
                .output()
                .unwrap_or_else(|e| panic!("DXC ({}) could not run: {e}; set VPI_DXC to the HLSL compiler", dxc.display()));
            if !run.status.success() {
                panic!("DXC could not compile {entry} of {}:\n{}", hlsl.display(), String::from_utf8_lossy(&run.stderr));
            }
            writeln!(table, "    (0x{key:016x}, {entry:?}, include_bytes!({:?})),", spv.display().to_string()).unwrap();
        }
    }
    table.push_str("];\n");
    std::fs::write(out.join("spirv.rs"), table).unwrap();
}

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
