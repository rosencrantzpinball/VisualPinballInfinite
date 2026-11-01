//! The player: plays a table with a window, input, physics, drawing, sound
//! and the table's script.

/// Whether this is a development build, which also looks in the repository's
/// `vpx-build` folders for the libraries and scripts it loads. A release
/// build, compiled with `VPI_RELEASE_BUILD` set (`tools\release`), looks
/// beside the program alone.
pub const DEVELOPMENT: bool = option_env!("VPI_RELEASE_BUILD").is_none();

/// How many threads the player's loading spreads over: the processors, up to eight; one in the
/// browser (`engine/web`), whose program has no threads of its own, so the work is done in turn.
pub fn threads() -> usize {
    if cfg!(target_family = "wasm") { 1 } else { std::thread::available_parallelism().map_or(4, std::num::NonZeroUsize::get).clamp(1, 8) }
}

pub mod addon_settings;
pub mod alphadmd;
pub mod alphanumeric;
pub mod anim;
pub mod audio;
pub mod b2s;
pub mod ball_mesh;
pub mod bulb;
pub mod camera;
pub mod colour;
pub mod decode;
pub mod dmd;
pub mod dmd_device;
pub mod dmd_panel;
pub mod dmd_overlay;
pub mod dik;
pub mod dof;
pub mod achievements;
pub mod events;
pub mod flexdmd;
pub mod focus;
pub mod frame_dump;
pub mod font;
pub mod game;
pub mod game_state;
pub mod gltf;
pub mod input;
pub mod input_map;
pub mod opd;
pub mod pause_menu;
pub mod light;
#[cfg(not(windows))]
pub mod local_time;
pub mod lightseq;
pub mod machine_scores;
pub mod machine_settings;
pub mod saved_values;
pub mod look;
pub mod pinmame;
pub mod math;
pub mod pictures;
pub mod model;
pub mod nudge_count;
pub mod plunger_input;
pub mod polygon;
pub mod prim_anim;
pub mod pup;
pub mod reel;
pub mod replay;
pub mod render;
pub mod rumble;
pub mod screenshot;
pub mod scene;
pub mod score_window;
pub mod scoreview;
pub mod script;
pub mod segments;
pub mod sensors;
pub mod settings;
pub mod shapes;
pub mod table_meshes;
pub mod textbox;
pub mod upscale;
pub mod vblank;
pub mod volume;
pub mod vr;
pub mod vpx_settings;
pub mod waker;
pub mod world;
/// VR through OpenXR (Linux, the Steam Frame).
#[cfg(not(windows))]
pub mod xr;
/// The app's screens in the headset (`vpi-panel`), shown by `vr`.
#[cfg(not(windows))]
pub mod panel;
