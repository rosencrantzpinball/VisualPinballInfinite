//! The headset's panel (PLAN decision 20): `vpi-panel`, the process that draws the app's screens with
//! CEF, started beside the player with a link of its own (`panel-link`), which the player makes and
//! names. The panel's pictures come through the link as it draws them, the player's pointer and keys go
//! back through it, and so do the panel's requests: play a table, close the panel, quit. `vr` draws it
//! into the eyes, standing in the room. Its output goes to the player's own. Ours.
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub use panel_link::{Frame, Input, Request};
use panel_link::{Link, state};

/// The panel's pictures, in pixels, and the scale its pages are drawn at: the app's screens 1280 by 800
/// CSS pixels, as on a laptop's screen, drawn at twice that for the text's sake in the headset (the
/// runtime samples the quad layer at the screens' own pixels).
pub const SIZE: (u32, u32) = (2560, 1600);
pub const SCALE: u32 = 200;

pub struct Panel {
    link: Link,
    child: Child,
    started: Instant,
}

/// Chromium draws the panel's pages through ANGLE, GLX and zink, on Vulkan (Turnip), and the panel starts
/// with what Vulkan is asked for the tables' sake changed (`VPI_PANEL_KEEP_ENV=1` leaves it, for tests);
/// the player and its tables keep it all:
///
/// - Turnip renders each of the panel's passes directly (`TU_DEBUG=sysmem`): its tiled rendering drew
///   the pages wrong. Chosen often by Turnip's autotune by profiling (`TU_AUTOTUNE_ALGO=profiled`,
///   run.sh's), it broke the library's left column up into noise as it was repainted under the pointer,
///   parts of the column drawn again in the wrong places (`hover-tune-1`, 2026-10-31: 7 of 14 rows
///   hovered), what the owner saw in the headset; at twice the pixels, with Turnip's own choice, it drew
///   the page's toolbar twice from the first picture (`hover2x-plain-1`). Rendering directly, neither
///   (`hover2x-sysmem-1`). The autotune setting goes too.
/// - Valve's Vulkan layers that Steam loads into games on the Steam Frame (`VK_INSTANCE_LAYERS`) and that
///   rewrite what a game draws, its Renderpass Optimizer and its FDM injection, at the owner's word: a
///   page needs neither.
const AUTOTUNE: &str = "TU_AUTOTUNE_ALGO";
const TU_DEBUG: &str = "TU_DEBUG";
const REWRITING_LAYERS: [&str; 2] = ["VK_LAYER_VALVE_rpo", "VK_LAYER_VALVE_fdm_injection"];

/// Turnip's debug options asked for (`TU_DEBUG`, `,` between them) with direct rendering among them.
fn rendering_directly(asked: Option<&str>) -> String {
    match asked.map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) if a.split(',').any(|o| o.trim() == "sysmem") => a.to_string(),
        Some(a) => format!("{a},sysmem"),
        None => "sysmem".to_string(),
    }
}

/// The instance layers asked for (`VK_INSTANCE_LAYERS`, `:` between them) without `REWRITING_LAYERS`;
/// none if nothing's left.
fn layers_kept(asked: &str) -> Option<String> {
    let kept: Vec<&str> = asked.split(':').filter(|l| !l.is_empty() && !REWRITING_LAYERS.contains(l)).collect();
    (!kept.is_empty()).then(|| kept.join(":"))
}

impl Panel {
    /// The panel's program: in the game's folder, `panel/vpi-panel` with CEF's files; in a development
    /// build, beside the player, where its build puts it.
    pub fn program() -> Option<PathBuf> {
        let here = std::env::current_exe().ok()?.parent()?.to_path_buf();
        [here.join("panel").join("vpi-panel"), here.join("vpi-panel")].into_iter().find(|p| p.is_file())
    }

    /// Starts the panel on the library under `library` (VPX's layout's folder: `Tables` in it), its
    /// "Back to" going to `back`, the table played last (none: no such button).
    pub fn start(library: &Path, back: Option<&Path>) -> Result<Panel, String> {
        let program = Panel::program().ok_or("no vpi-panel beside the player")?;
        let name = format!("vpi-panel-{}", std::process::id());
        let link = Link::create(&name, SIZE.0, SIZE.1, SCALE).map_err(|e| format!("the panel's link: {e}"))?;
        let mut command = Command::new(&program);
        command.arg("--link").arg(&name).arg("--library").arg(library).stdin(Stdio::null());
        if let Some(back) = back {
            command.arg("--back").arg(back);
        }
        if let Some(data) = vpi_settings::data_folder() {
            command.arg("--data").arg(data.join("panel"));
        }
        // the screens: beside the panel (`ui/`, the game's folder), else the app's own build of them
        // (a development build: target/release beside app/ui/dist)
        let dir = program.parent().map(Path::to_path_buf).unwrap_or_default();
        if !dir.join("ui").join("index.html").is_file() {
            let built = dir.join("../../app/ui/dist");
            if built.join("index.html").is_file() {
                command.arg("--ui").arg(built);
            }
        }
        // Chromium kept from what Vulkan is asked for the tables' sake, and rendering directly
        if std::env::var_os("VPI_PANEL_KEEP_ENV").is_none() {
            let debug = rendering_directly(std::env::var(TU_DEBUG).ok().as_deref());
            println!("panel: started with {TU_DEBUG}={debug}, Turnip rendering each pass directly");
            command.env(TU_DEBUG, debug);
            if let Ok(algo) = std::env::var(AUTOTUNE) {
                println!("panel: started without {AUTOTUNE}={algo}, Turnip choosing for itself");
                command.env_remove(AUTOTUNE);
            }
            if let Ok(asked) = std::env::var("VK_INSTANCE_LAYERS") {
                let kept = layers_kept(&asked);
                if kept.as_deref() != Some(asked.as_str()) {
                    println!("panel: started without the Vulkan layers {}, keeping {}", REWRITING_LAYERS.join(" and "), kept.as_deref().unwrap_or("none"));
                }
                match kept {
                    Some(kept) => command.env("VK_INSTANCE_LAYERS", kept),
                    None => command.env_remove("VK_INSTANCE_LAYERS"),
                };
            }
        }
        let child = command.spawn().map_err(|e| format!("{}: {e}", program.display()))?;
        println!("panel: {} started on {}", program.display(), library.display());
        Ok(Panel { link, child, started: Instant::now() })
    }

    /// The panel's newest picture, when it drew one since the last taken.
    pub fn picture(&self) -> Option<Frame<'_>> {
        self.link.take_frame()
    }

    pub fn send(&self, input: Input) {
        if !self.link.send(input) {
            eprintln!("panel: its input is full, {input:?} dropped");
        }
    }

    /// The panel's next request.
    pub fn request(&self) -> Option<Request> {
        self.link.request()
    }

    /// The panel still runs: not ended, not failed, and drawing within 30 s of starting.
    pub fn running(&mut self) -> bool {
        let alive = matches!(self.child.try_wait(), Ok(None));
        let state = self.link.state();
        alive && state != state::FAILED && state != state::ENDED && (state == state::READY || self.started.elapsed() < Duration::from_secs(30))
    }
}

/// The side of the blocks `solid_blocks` looks at, in pixels.
pub const BLOCK: u32 = 8;

/// Which blocks of a BGRA picture (`BLOCK` pixels square, row by row) are pure black or pure white
/// all over: what a GPU tile drawn wrong looks like, and never the screens' own (their darkest is the
/// page's 16, 18, 22, and no text is that solid), though a table's picture may be.
pub fn solid_blocks(pixels: &[u8], width: u32, height: u32) -> Vec<bool> {
    let (across, down, side) = ((width / BLOCK) as usize, (height / BLOCK) as usize, BLOCK as usize);
    let row = width as usize * 4;
    let mut solid = vec![false; across * down];
    if pixels.len() < row * height as usize {
        return solid;
    }
    for by in 0..down {
        for bx in 0..across {
            let first = by * side * row + bx * side * 4;
            let colour = [pixels[first], pixels[first + 1], pixels[first + 2]];
            if colour != [0, 0, 0] && colour != [255, 255, 255] {
                continue;
            }
            solid[by * across + bx] = (0..side).all(|y| pixels[first + y * row..][..side * 4].chunks_exact(4).all(|p| p[..3] == colour));
        }
    }
    solid
}

impl Drop for Panel {
    /// Asked to end, then made to, after 3 s.
    fn drop(&mut self) {
        self.link.request_stop();
        let until = Instant::now() + Duration::from_secs(3);
        while Instant::now() < until {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        eprintln!("panel: it didn't end when asked: ended");
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_renders_directly_whatever_else_turnip_is_asked() {
        assert_eq!(rendering_directly(None), "sysmem");
        assert_eq!(rendering_directly(Some(" ")), "sysmem");
        assert_eq!(rendering_directly(Some("noubwc")), "noubwc,sysmem");
        assert_eq!(rendering_directly(Some("sysmem,noubwc")), "sysmem,noubwc");
    }

    #[test]
    fn the_panel_keeps_the_layers_that_dont_rewrite_drawing() {
        assert_eq!(layers_kept("VK_LAYER_VALVE_rpo:VK_LAYER_VALVE_fdm_injection"), None);
        assert_eq!(layers_kept("VK_LAYER_A:VK_LAYER_VALVE_rpo:VK_LAYER_B"), Some("VK_LAYER_A:VK_LAYER_B".into()));
        assert_eq!(layers_kept("VK_LAYER_A"), Some("VK_LAYER_A".into()));
        assert_eq!(layers_kept(""), None);
    }

    /// A 32 by 16 picture of the page's own dark, with `blocks` (block column, row) painted `colour`.
    fn picture(blocks: &[(usize, usize)], colour: [u8; 3]) -> Vec<u8> {
        let (w, h) = (32usize, 16usize);
        let mut pixels: Vec<u8> = std::iter::repeat_n([22, 18, 16, 255], w * h).flatten().collect();
        for &(bx, by) in blocks {
            for y in by * 8..by * 8 + 8 {
                for x in bx * 8..bx * 8 + 8 {
                    pixels[(y * w + x) * 4..][..3].copy_from_slice(&colour);
                }
            }
        }
        pixels
    }

    #[test]
    fn only_wholly_black_or_white_blocks_are_solid() {
        let none = solid_blocks(&picture(&[], [0; 3]), 32, 16);
        assert_eq!(none, vec![false; 8]);
        let white = solid_blocks(&picture(&[(1, 0), (3, 1)], [255; 3]), 32, 16);
        assert_eq!(white.iter().enumerate().filter(|(_, s)| **s).map(|(i, _)| i).collect::<Vec<_>>(), vec![1, 7]);
        // a black block with one pixel of the page's dark in it: text, not a broken tile
        let mut pixels = picture(&[(2, 0)], [0; 3]);
        pixels[(3 * 32 + 20) * 4..][..3].copy_from_slice(&[22, 18, 16]);
        assert_eq!(solid_blocks(&pixels, 32, 16), vec![false; 8]);
        // too short for its size: nothing judged
        assert_eq!(solid_blocks(&pixels[..100], 32, 16), vec![false; 8]);
    }
}
