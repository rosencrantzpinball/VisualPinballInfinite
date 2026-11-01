//! What the player's frame loop asks of the headset in VR (`vpi-player --vr`): on Linux through OpenXR
//! ([`crate::xr`]), the Steam Frame's way; elsewhere VR doesn't start yet. Each frame waits for the
//! headset's, takes the newest game state, then draws both eyes with VPX's placement of the table in the
//! room and hands them over. Test runs can pin the eyes where a player stands (drawn even while nobody
//! wears the headset), save both eyes' pictures at set times, and write each frame's timings; a line on
//! the headset's frames goes to the log every 5 s, as the VPX fork's does. Ours.

use std::path::{Path, PathBuf};

/// How VR runs, from the options.
#[derive(Clone, Debug, Default)]
pub struct VrOptions {
    /// The world shows where nothing is drawn (alpha blend), where the runtime can.
    pub passthrough: bool,
    /// Test runs: the eyes pinned where a player stands, every frame drawn.
    pub pin_eyes: bool,
    /// Test runs: both eyes' pictures saved in this folder at these times (ms after play starts).
    pub capture: Option<PathBuf>,
    pub capture_at: Vec<u64>,
    /// Each frame's timings, as CSV.
    pub timings: Option<PathBuf>,
    /// Each eye's picture this size, in place of the runtime's recommended one (test runs).
    pub eye_size: Option<(u32, u32)>,
    /// The library's panel alone (`--library`): each eye drawn larger than the runtime recommends, for
    /// the panel's text (`LIBRARY_EYE_SCALE`).
    pub library: bool,
}

/// The table's own measures VPX's placement reads.
#[derive(Clone, Copy, Debug)]
pub struct TableMeasures {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    /// The glass's height above the playfield at its bottom end, VPU (VPX's `m_glassBottomHeight`).
    pub glass_bottom: f32,
    /// The playfield's slope in degrees (VPX's `GetPlayfieldSlope`: the table's slopes by its difficulty).
    pub slope_deg: f32,
}

impl TableMeasures {
    pub fn of(vpx: &vpin::vpx::VPX) -> TableMeasures {
        let g = &vpx.gamedata;
        TableMeasures {
            left: g.left,
            top: g.top,
            right: g.right,
            bottom: g.bottom,
            glass_bottom: g.glass_bottom_height.unwrap_or(g.glass_top_height),
            slope_deg: g.angle_tilt_min + (g.angle_tilt_max - g.angle_tilt_min) * g.global_difficulty,
        }
    }
}

/// One eye's view of the table this frame, for the renderer's frame input.
#[derive(Clone, Copy, Debug)]
pub struct EyeView {
    pub view: crate::math::Mat4,
    pub projection: crate::math::Mat4,
    pub eye: crate::math::Vec3,
}

/// The tables the menu can change to in VR: those in the Frame's tables folder
/// (`~/Documents/VPinballX/Tables`, VPX's there), else beside the table playing's folder, each in its own
/// folder or not, by name.
pub fn tables(playing: &Path) -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).map(|h| h.join("Documents").join("VPinballX").join("Tables"));
    let beside = playing.parent().and_then(Path::parent).map(Path::to_path_buf);
    let Some(root) = home.filter(|h| h.is_dir()).or(beside) else { return Vec::new() };
    let is_table = |p: &Path| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("vpx"));
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&root).into_iter().flatten().flatten() {
        let p = entry.path();
        if p.is_dir() {
            found.extend(std::fs::read_dir(&p).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| is_table(p)));
        } else if is_table(&p) {
            found.push(p);
        }
    }
    found.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    found
}

/// The library the headset's panel shows: VPX's layout's folder (`Tables` in it): `VPI_LIBRARY`'s
/// (checks and other machines), the Frame's (`~/Documents/VPinballX`), else the one the table playing
/// is in (`<library>/Tables/<folder>/<table>`).
pub fn library_root(playing: Option<&Path>) -> PathBuf {
    if let Some(dir) = std::env::var_os("VPI_LIBRARY") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).map(|h| h.join("Documents").join("VPinballX"));
    if let Some(h) = home.as_ref().filter(|h| h.join("Tables").is_dir()) {
        return h.clone();
    }
    let beside = playing.and_then(|t| t.ancestors().find(|a| a.file_name().is_some_and(|n| n.eq_ignore_ascii_case("Tables")))).and_then(Path::parent);
    beside.map(Path::to_path_buf).or(home).unwrap_or_default()
}

#[cfg(not(windows))]
pub use imp::{Vr, VrFrame};

/// Each of the table's parts (by its index among the file's parts) placed in the room by its VPX 10.8.1
/// part group (`SpaceReference::Room`): the table's room, left out with passthrough.
pub fn room_parts(vpx: &vpin::vpx::VPX) -> Vec<bool> {
    use vpin::vpx::gameitem::GameItemEnum;
    use vpin::vpx::gameitem::partgroup::SpaceReference;
    let rooms: std::collections::HashSet<&str> = vpx
        .gameitems
        .iter()
        .filter_map(|item| match item {
            GameItemEnum::PartGroup(g) if matches!(g.space_reference, SpaceReference::Room) => Some(g.name.as_str()),
            _ => None,
        })
        .collect();
    vpx.gameitems.iter().map(|item| item.part_group_name().is_some_and(|g| rooms.contains(g))).collect()
}

#[cfg(not(windows))]
mod imp {
    use std::io::Write as _;
    use std::time::Instant;

    use sdl3_sys::everything::*;

    use super::{EyeView, VrOptions};
    use crate::math::{Mat4, Vec3};
    use crate::render::{FrameInput, Renderer};
    use super::TableMeasures;
    use crate::xr::place::{self, Placement, Pose};
    use crate::xr::controls::Controls;
    use crate::xr::fovea::{Foveation, Level};
    use crate::panel::{self, Input, Panel, Request};
    use crate::pause_menu::{Layout, Pointer, Spot};
    use crate::render::MenuPicture;
    use crate::render::room::{RoomPictures, RoomQuad};
    use crate::xr::{Frame, Layer, Quad, Runtime, Session};

    /// The headset: the OpenXR runtime, its session and the GPU device it made, which the renderer
    /// borrows. Dropped after the renderer: the session first, then the device, then the runtime.
    pub struct Vr {
        controls: Option<Controls>,
        session: Option<Session>,
        device: *mut SDL_GPUDevice,
        runtime: Box<Runtime>,
        placement: Placement,
        measures: TableMeasures,
        to_room: Mat4,
        scale: f32,
        far: f32,
        options: VrOptions,
        /// The next capture time's index in `options.capture_at`.
        next_capture: usize,
        timings: Option<std::io::BufWriter<std::fs::File>>,
        stats: Stats,
        started: Instant,
        frames: u64,
        foveation: Foveation,
        resolution: Resolution,
        /// The table was recentred this session: its placement is kept when VR ends, as VPX keeps it.
        recentred: bool,
        /// The app's screens on a panel in the room, once opened.
        panel: Option<PanelView>,
        /// The pause menu, a panel in the room while it shows.
        menu: Option<MenuView>,
        /// The presses kept from the game (`KEPT_KEYS`'), made at the pause menu.
        keys_kept: [bool; 6],
        /// The next capture time's index for the panel's pictures (test runs, `--vr-capture`).
        next_panel_capture: usize,
        /// The panel, the pause menu and the laser drawn into the eyes (`render::room`), by their `slot`s;
        /// none with them as the runtime's quad layers (`[PlayerVR]` `PanelQuadLayers`, the default).
        room: Option<RoomPictures>,
        /// The laser this frame: the pointing hand's aim, and where it meets the panel or the pause menu;
        /// and the dot where it meets the pause menu (the panel's page draws its own).
        laser: Option<(Pose, Option<[f32; 3]>)>,
        dot: Option<Pose>,
        /// The pictures that never change (the laser's, the dot's), uploaded once wanted.
        still_pictures: Vec<Layer>,
        /// Test runs (`VPI_XR_TEST_HAND=<from>-<to>:<degrees>`): the right hand's grip held from `from` to
        /// `to` ms into VR, as it turns by `degrees` (`tested_hands`), and the way it points before.
        test_hand: Option<(u64, u64, f32)>,
        test_way: Option<[f32; 3]>,
    }

    /// The panel's, the pause menu's, the laser's and the dot's pictures in the room's drawing.
    fn slot(layer: Layer) -> usize {
        match layer {
            Layer::Panel => 0,
            Layer::Menu => 1,
            Layer::Laser => 2,
            Layer::Dot => 3,
        }
    }

    /// The library's eyes, against the runtime's recommended size: SteamVR recommends 0.8 of the Steam
    /// Frame's screens (0.64 of their pixels), and the panel's text wants their own.
    const LIBRARY_EYE_SCALE: f32 = 1.25;
    /// The room's near and far planes, metres, for the pictures standing in it.
    const ROOM_NEAR: f32 = 0.05;
    const ROOM_FAR: f32 = 100.0;

    /// Each eye's view of the room this frame: the room into that eye's clip space.
    fn room_eyes(f: &Frame) -> [Mat4; 2] {
        [0, 1].map(|eye| f.views[eye].view_matrix() * place::projection(f.fov[eye], ROOM_NEAR, ROOM_FAR))
    }

    /// The quads as the room draws them.
    fn room_quads(quads: &[(Layer, Quad)]) -> Vec<RoomQuad> {
        quads.iter().map(|&(layer, q)| RoomQuad { picture: slot(layer), position: q.pose.position, orientation: q.pose.orientation, size: q.size }).collect()
    }

    /// The panel's width and height in the room, metres: 1.7 by 1.06, 1.5 m away, the size of a large
    /// television across a small room.
    const PANEL_METRES: (f32, f32) = (1.7, 1.0625);
    const PANEL_AWAY: f32 = 1.5;
    /// The pause menu's distance, and the height in the room of the screen it's laid out for (the
    /// window's): its picture as big as it would be on that screen.
    const MENU_AWAY: f32 = 1.2;
    const MENU_SCREEN_METRES: f32 = 1.0;
    /// The keys a hand presses at the pause menu, kept from the game there: the triggers' (the flippers
    /// and the staged flippers) and the grips' (the magna-saves, which take hold of the menu).
    const KEPT_KEYS: [u8; 6] = {
        use crate::dik::default_keys as k;
        [k::LEFT_FLIPPER, k::RIGHT_FLIPPER, k::STAGED_LEFT_FLIPPER, k::STAGED_RIGHT_FLIPPER, k::LEFT_MAGNA_SAVE, k::RIGHT_MAGNA_SAVE]
    };
    /// The library's room around the panel: near black (#07080b, linear), darker than the panel's own.
    const LIBRARY_DARK: SDL_FColor = SDL_FColor { r: 0.0022, g: 0.0025, b: 0.0034, a: 1.0 };

    /// The panel as the player shows it: the process, where it is in the room, and the pointer on it.
    struct PanelView {
        panel: Panel,
        /// Where it is, once placed: in front of the head, the frame after it opens.
        pose: Option<Pose>,
        shown: bool,
        /// The hand pointing (the last whose trigger was pulled), each trigger's state, the point
        /// pointed at (the panel's pixels).
        hand: usize,
        pressed: [bool; 2],
        at: Option<(f32, f32)>,
        /// The pictures it has sent, and in test runs with captures still to come, the newest (its pixels,
        /// size and number): the page may be still by a capture's time.
        pictures: u64,
        newest: Option<(Vec<u8>, u32, u32, u32)>,
        /// Placed while the headset had the controllers (focused): a panel placed before (behind the
        /// runtime's own menu, say) is placed again as they come.
        placed_focused: bool,
        /// The newest picture's blocks of solid black or white (`panel::solid_blocks`), and how many
        /// pictures with new ones were kept to look at.
        solid: Vec<bool>,
        broken_kept: u32,
        /// A grip's hold on it (`held`), and each grip as it was.
        grab: Option<Grab>,
        gripped: [bool; 2],
        /// Frames shown so far, the frame its two swapchains last changed places and the one the picture
        /// being written was sent, and since the last count, the pictures put up, the most frames one
        /// took to be written and the longest the runtime took to give an image.
        frames: u64,
        swapped_at: u64,
        sent_at: u64,
        put_up: u32,
        slowest: u64,
        longest_wait: std::time::Duration,
    }

    /// The pause menu as the player shows it in the room: its picture's size and its place on the
    /// screen it was laid out for, that screen's height, its rows' places, where it is, and the
    /// pointer on it.
    #[derive(Default)]
    struct MenuView {
        size: (u32, u32),
        at: (f32, f32),
        screen_height: f32,
        layout: Layout,
        pose: Option<Pose>,
        shown: bool,
        hand: usize,
        pressed: [bool; 2],
        hover: Option<usize>,
        dragging: Option<usize>,
        /// The stick's notches turned, not yet whole.
        wheel: f32,
        /// The pointing hand's ray meets it.
        aiming: bool,
        /// A grip's hold on it (`held`), and each grip as it was.
        grab: Option<Grab>,
        gripped: [bool; 2],
    }

    impl MenuView {
        fn metres(&self) -> (f32, f32) {
            let h = self.size.1 as f32 / self.screen_height.max(1.0) * MENU_SCREEN_METRES;
            (h * self.size.0 as f32 / self.size.1.max(1) as f32, h)
        }
    }

    fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
        let [x, y, z, w] = q;
        let u = [x, y, z];
        let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
        let t = cross(u, v).map(|c| c * 2.0);
        let c = cross(u, t);
        [v[0] + w * t[0] + c[0], v[1] + w * t[1] + c[1], v[2] + w * t[2] + c[2]]
    }

    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    /// A panel placed before a head: `away` ahead on the level, a little under the eyes, turned to face
    /// it (its front, +z, towards the head).
    fn before(head: &Pose, away: f32) -> Pose {
        let f = rotate(head.orientation, [0.0, 0.0, -1.0]);
        let len = (f[0] * f[0] + f[2] * f[2]).sqrt().max(1e-4);
        let (fx, fz) = (f[0] / len, f[2] / len);
        let yaw = (-fx).atan2(-fz);
        let p = head.position;
        Pose { position: [p[0] + fx * away, p[1] - 0.15, p[2] + fz * away], orientation: [0.0, (yaw / 2.0).sin(), 0.0, (yaw / 2.0).cos()] }
    }

    /// Where a ray from `aim` (along its -z) meets a panel at `pose`, `size` metres: the point on it,
    /// from its top left, as a share of its width and height; none off it or behind.
    fn hit(aim: &Pose, pose: &Pose, size: (f32, f32)) -> Option<(f32, f32)> {
        let d = rotate(aim.orientation, [0.0, 0.0, -1.0]);
        let n = rotate(pose.orientation, [0.0, 0.0, 1.0]);
        let denom = dot(d, n);
        if denom.abs() < 1e-5 {
            return None;
        }
        let o = aim.position;
        let c = pose.position;
        let t = dot([c[0] - o[0], c[1] - o[1], c[2] - o[2]], n) / denom;
        if t <= 0.0 {
            return None;
        }
        let hit = [o[0] + d[0] * t - c[0], o[1] + d[1] * t - c[1], o[2] + d[2] * t - c[2]];
        let [x, y, z, w] = pose.orientation;
        let local = rotate([-x, -y, -z, w], hit);
        let (u, v) = (local[0] / size.0 + 0.5, 0.5 - local[1] / size.1);
        ((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)).then_some((u, v))
    }

    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
    }

    /// Two turns (x, y, z, w) one after the other: `first`, then `then`.
    fn turned(then: [f32; 4], first: [f32; 4]) -> [f32; 4] {
        let ([ax, ay, az, aw], [bx, by, bz, bw]) = (then, first);
        [aw * bx + ax * bw + ay * bz - az * by, aw * by - ax * bz + ay * bw + az * bx, aw * bz + ax * by - ay * bx + az * bw, aw * bw - ax * bx - ay * by - az * bz]
    }

    /// `local`, a pose in `frame`'s own space, in the room.
    fn in_room(frame: &Pose, local: &Pose) -> Pose {
        let p = rotate(frame.orientation, local.position);
        Pose { position: [0, 1, 2].map(|i| frame.position[i] + p[i]), orientation: turned(frame.orientation, local.orientation) }
    }

    /// `pose`, in the room, in `frame`'s own space.
    fn in_frame(frame: &Pose, pose: &Pose) -> Pose {
        let [x, y, z, w] = frame.orientation;
        let back = [-x, -y, -z, w];
        Pose { position: rotate(back, [0, 1, 2].map(|i| pose.position[i] - frame.position[i])), orientation: turned(back, pose.orientation) }
    }

    /// The point (`u`, `v`) of a panel at `pose`, `size` metres (from its top left, as shares of its width
    /// and height), in the room.
    fn on_panel(pose: &Pose, size: (f32, f32), (u, v): (f32, f32)) -> [f32; 3] {
        let p = rotate(pose.orientation, [(u - 0.5) * size.0, (0.5 - v) * size.1, 0.0]);
        [0, 1, 2].map(|i| pose.position[i] + p[i])
    }

    /// The turn taking the room's axes to `x`, `y` and `z` (each a unit vector, at right angles).
    fn from_axes(x: [f32; 3], y: [f32; 3], z: [f32; 3]) -> [f32; 4] {
        let (m00, m11, m22) = (x[0], y[1], z[2]);
        let trace = m00 + m11 + m22;
        if trace > 0.0 {
            let s = 0.5 / (trace + 1.0).sqrt();
            [(y[2] - z[1]) * s, (z[0] - x[2]) * s, (x[1] - y[0]) * s, 0.25 / s]
        } else if m00 > m11 && m00 > m22 {
            let s = 2.0 * (1.0 + m00 - m11 - m22).sqrt();
            [0.25 * s, (y[0] + x[1]) / s, (z[0] + x[2]) / s, (y[2] - z[1]) / s]
        } else if m11 > m22 {
            let s = 2.0 * (1.0 + m11 - m00 - m22).sqrt();
            [(y[0] + x[1]) / s, 0.25 * s, (z[1] + y[2]) / s, (z[0] - x[2]) / s]
        } else {
            let s = 2.0 * (1.0 + m22 - m00 - m11).sqrt();
            [(z[0] + x[2]) / s, (z[1] + y[2]) / s, 0.25 * s, (x[1] - y[0]) / s]
        }
    }

    /// The laser: its width in the room, how far it reaches when it meets nothing, its picture's size.
    const LASER_WIDTH: f32 = 0.005;
    const LASER_MISS: f32 = 0.6;
    const LASER_PICTURE: (u32, u32) = (16, 256);

    /// The laser's picture, RGBA with straight alpha, sRGB, its top the far end: a white core in a pale
    /// blue glow across it, fading in over the fifth of it nearest the hand.
    fn laser_picture() -> Vec<u8> {
        let (w, h) = LASER_PICTURE;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for row in 0..h {
            let fade = ((1.0 - (row as f32 + 0.5) / h as f32) / 0.2).clamp(0.0, 1.0);
            for column in 0..w {
                // 0 in the middle, 1 at the edges
                let across = ((column as f32 + 0.5) / w as f32 - 0.5).abs() * 2.0;
                let core = (1.0 - across / 0.35).clamp(0.0, 1.0);
                let glow = (1.0 - across).clamp(0.0, 1.0).powi(2);
                let rgb = [0.6 + 0.4 * core, 0.82 + 0.18 * core, 1.0f32];
                out.extend(rgb.map(|c| (c * 255.0).round() as u8));
                out.push(((0.9 * core + 0.45 * glow).min(1.0) * fade * 255.0).round() as u8);
            }
        }
        out
    }

    /// The pointer's dot on the pause menu: its width in the room, how far before the menu it stands, and
    /// its picture's size.
    const DOT_WIDTH: f32 = 0.018;
    const DOT_BEFORE: f32 = 0.002;
    const DOT_PICTURE: (u32, u32) = (64, 64);

    /// The dot's picture, RGBA with straight alpha, sRGB: a near white disc in a dark rim, seen on the
    /// menu's dark rows and its picked one alike, its edges smooth.
    fn dot_picture() -> Vec<u8> {
        let (w, h) = DOT_PICTURE;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for row in 0..h {
            for column in 0..w {
                // distance from the middle, the picture's half width 1; a pixel is 2 / w across
                let (x, y) = ((column as f32 + 0.5) / w as f32 * 2.0 - 1.0, (row as f32 + 0.5) / h as f32 * 2.0 - 1.0);
                let r = (x * x + y * y).sqrt();
                let pixel = 2.0 / w as f32;
                let inside = |edge: f32| ((edge - r) / pixel + 0.5).clamp(0.0, 1.0);
                let (disc, rim) = (inside(0.62), inside(0.92));
                let rgb = [0.94 * disc + 0.04 * (1.0 - disc), 0.96 * disc + 0.05 * (1.0 - disc), 1.0 * disc + 0.08 * (1.0 - disc)];
                out.extend(rgb.map(|c| (c * 255.0).round() as u8));
                out.push(((disc + (rim - disc) * 0.85) * 255.0).round() as u8);
            }
        }
        out
    }

    /// The laser from `aim` (along its -z) to `to`, where it meets a panel, else `LASER_MISS` on: a thin
    /// quad along it, its +y towards the far end, turned about it to face `head`. Its pose and size.
    fn beam(aim: &Pose, to: Option<[f32; 3]>, head: [f32; 3]) -> Option<(Pose, (f32, f32))> {
        let start = aim.position;
        let d = rotate(aim.orientation, [0.0, 0.0, -1.0]);
        let end = to.unwrap_or([0, 1, 2].map(|i| start[i] + d[i] * LASER_MISS));
        let along = [0, 1, 2].map(|i| end[i] - start[i]);
        let length = dot(along, along).sqrt();
        if length < 0.01 {
            return None;
        }
        let y = along.map(|c| c / length);
        let middle = [0, 1, 2].map(|i| (start[i] + end[i]) * 0.5);
        // across it, square to the line of sight; any square to it when it's seen end on
        let mut x = cross(y, [0, 1, 2].map(|i| head[i] - middle[i]));
        for other in [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0]] {
            if dot(x, x) > 1e-10 {
                break;
            }
            x = cross(y, other);
        }
        let n = dot(x, x).sqrt().max(1e-6);
        let x = x.map(|c| c / n);
        Some((Pose { position: middle, orientation: from_axes(x, y, cross(x, y)) }, (LASER_WIDTH, length)))
    }

    /// How a grip takes a panel nearer or sends it away (the stick, up away): at most this fast, metres a
    /// second, and no nearer or farther than these.
    const GRAB_SPEED: f32 = 1.5;
    const GRAB_NEAREST: f32 = 0.3;
    const GRAB_FARTHEST: f32 = 5.0;

    /// A panel held by a hand's grip: the hand, and where the panel is in that hand's aim's own space.
    #[derive(Clone, Copy, Debug)]
    struct Grab {
        hand: usize,
        held: Pose,
    }

    /// The grips on a panel at `pose`, `size` metres, this frame, as SteamVR's grips move its windows: a
    /// grip pressed with its hand's ray on the panel takes hold of it, and it follows that hand's aim
    /// (`aims`) until the grip is let go, `reach` metres farther along (nearer, negative) each frame it's
    /// held. `gripped`: each grip as it was. The panel's pose now.
    fn held(grab: &mut Option<Grab>, gripped: &mut [bool; 2], pose: Pose, size: (f32, f32), aims: [Option<Pose>; 2], grips: [bool; 2], reach: f32) -> Pose {
        for hand in 0..2 {
            let pressed = grips[hand] && !gripped[hand];
            gripped[hand] = grips[hand];
            if pressed
                && grab.is_none()
                && let Some(aim) = aims[hand]
                && hit(&aim, &pose, size).is_some()
            {
                *grab = Some(Grab { hand, held: in_frame(&aim, &pose) });
            }
        }
        let Some(g) = grab.as_mut() else { return pose };
        if !grips[g.hand] {
            *grab = None;
            return pose;
        }
        let Some(aim) = aims[g.hand] else { return pose };
        if reach != 0.0 {
            let far = dot(g.held.position, g.held.position).sqrt().max(1e-4);
            let to = (far + reach).clamp(GRAB_NEAREST, GRAB_FARTHEST);
            g.held.position = g.held.position.map(|c| c * to / far);
        }
        in_room(&aim, &g.held)
    }

    /// The turn pointing -z along `d` (a unit vector), its x level: a hand aiming that way (test runs).
    fn look(d: [f32; 3]) -> [f32; 4] {
        let yaw = (-d[0]).atan2(-d[2]);
        let pitch = d[1].clamp(-1.0, 1.0).asin();
        turned([0.0, (yaw / 2.0).sin(), 0.0, (yaw / 2.0).cos()], [(pitch / 2.0).sin(), 0.0, 0.0, (pitch / 2.0).cos()])
    }

    /// VR's dynamic resolution, as the fork steers it (its `UpdateDynamicResolution` of 2026-10-28): each eye
    /// drawn at a scale of its swapchain's size (the table's resolution) so the runtime's GPU time stays near
    /// a share of the shortest frame period seen (`[PlayerVR]` `DynamicResolution`, `DynamicResolutionTarget`,
    /// `DynamicResolutionMinScale`). The GPU time averaged over about 5 frames, by small steps each frame: down
    /// as soon as it's over (at once for a frame over the whole period), up when the next step is expected to
    /// stay under 97 % of it (the cost following the pixels), a quarter of a second after the last step down,
    /// faster with more room. The renderer draws the top left part of pictures made at the full size, so a
    /// change costs nothing. Not in the session's first 6 s, which the runtime makes slow (PROGRESS.md).
    struct Resolution {
        on: bool,
        /// The GPU time aimed at, as a share of the frame period, and the lowest scale (per axis).
        target: f32,
        min_scale: f32,
        scale: f32,
        /// The shortest frame period of the session, ms (SteamVR doubles the predicted one while it holds
        /// the player at half rate, which isn't the budget).
        budget_ms: f32,
        /// The GPU time averaged over about 5 frames, and when the scale last went down.
        smoothed_ms: f32,
        lowered: Instant,
        /// Readings until then are left out: the session's first 6 s.
        settle_until: Option<Instant>,
        /// Test runs: the scale switched between two every so often (`VPI_XR_SCALE_STEPS=0.8,1.0,2`), the
        /// steering off, to measure what a change costs.
        steps: Option<(f32, f32, f32)>,
    }

    impl Resolution {
        /// Each eye's size this frame from its swapchain's: whole texels of 8 pixels.
        fn size(&self, (w, h): (u32, u32)) -> (u32, u32) {
            if self.scale >= 1.0 {
                return (w, h);
            }
            let part = |v: u32| (((v as f32 * self.scale) as u32) / 8 * 8).clamp(8.min(v), v);
            (part(w), part(h))
        }

        /// A frame's GPU time (the runtime's) and period; true when the scale changed.
        fn update(&mut self, gpu_ms: Option<f32>, period_ms: f32) -> bool {
            if let Some((a, b, every)) = self.steps {
                if self.lowered.elapsed().as_secs_f32() < every {
                    return false;
                }
                self.lowered = Instant::now();
                self.scale = if self.scale == a { b } else { a };
                return true;
            }
            if !self.on {
                return false;
            }
            if period_ms > 1.0 {
                self.budget_ms = self.budget_ms.min(period_ms);
            }
            // nothing drawn reads near 0 (unworn): not a frame's cost
            let Some(g) = gpu_ms.filter(|&g| g >= 1.0) else { return false };
            let now = Instant::now();
            if now < *self.settle_until.get_or_insert(now + std::time::Duration::from_secs(6)) {
                return false;
            }
            // single frames over the target are frequent on some tables: lowering the scale for each kept it
            // well under what the table sustains (the fork's finding), so it follows the average, unless a
            // frame took longer than the whole period
            self.smoothed_ms = if self.smoothed_ms <= 0.0 { g } else { self.smoothed_ms + (g - self.smoothed_ms) * 0.2 };
            let budget = self.budget_ms;
            let steer = if g > budget { g.max(self.smoothed_ms) } else { self.smoothed_ms };
            let target = self.target * budget;
            let before = self.scale;
            if steer > target {
                // faster the further over
                self.scale -= 0.02 * (1.0 + (steer - target) / target);
                self.lowered = now;
            } else if now.duration_since(self.lowered).as_secs_f32() >= 0.25 {
                // up when the next step is expected under the target, the GPU time following the pixels (an
                // overestimate: part of the frame doesn't depend on the size), by 0.3 % a frame near the target
                // up to 1 % with a fifth of it to spare
                const RISE: f32 = 0.003;
                let next = (self.scale + RISE) / self.scale;
                let predicted = steer * next * next;
                if predicted < target * 0.97 {
                    self.scale += RISE + 0.007 * ((target * 0.97 - predicted) / (target * 0.2)).clamp(0.0, 1.0);
                }
            }
            self.scale = self.scale.clamp(self.min_scale, 1.0);
            self.scale != before
        }
    }

    /// A frame the headset asked for.
    pub struct VrFrame {
        frame: Frame,
        began: Instant,
    }

    /// The headset's frames over the last 5 s, for the log.
    #[derive(Default)]
    struct Stats {
        since: Option<Instant>,
        cpu_ms: Vec<f32>,
        waited_ms: Vec<f32>,
        gpu_ms: Vec<f32>,
        drawn: u32,
        period_ms: f32,
        /// What placed the foveation's discs, by frame count.
        fovea: Vec<(&'static str, u32)>,
        /// An eye's triangles, those its culling left out, and those drawn into its mirrors (the last frame's).
        triangles: (u64, u64, u64),
        /// The narrowest and widest an eye was drawn (dynamic resolution).
        widths: Option<(u32, u32)>,
    }

    impl Vr {
        /// Starts VR: the runtime, the GPU device it makes and the session, each eye at the size the
        /// runtime recommends; the table placed by the settings (`[Player]` `LockbarWidth`/`Height`,
        /// `[PlayerVR]` `Orientation`, `TableX`/`Y`/`Z`). `bounds` are the scene's, for the far plane.
        pub fn start(settings: &crate::settings::Settings, options: VrOptions, measures: TableMeasures, bounds: (Vec3, Vec3)) -> Result<Vr, String> {
            // foveated rendering, as the fork's settings ask for it (`[PlayerVR]` `Foveation` Off, Low,
            // Medium, High; Medium on the Frame by default), eye-tracked when the headset can
            let level = Level::from_setting(settings.enum_or("PlayerVR", "Foveation", 2, 4));
            let mut foveation = Foveation::new(
                level,
                settings.get("PlayerVR", "FoveationEyeTracked").and_then(crate::settings::vpx_bool).unwrap_or(true),
                ["FoveationFlipX", "FoveationFlipY"].map(|k| settings.get("PlayerVR", k).and_then(crate::settings::vpx_bool).unwrap_or(false)),
            );
            // the gaze check's small sharp spot, from the settings too (`[PlayerVR]` `FoveationDebug`, ours)
            foveation.debug |= settings.get("PlayerVR", "FoveationDebug").and_then(crate::settings::vpx_bool).unwrap_or(false);
            // test runs hold a scale (`VPI_XR_SCALE=0.8`), the steering off
            let held = std::env::var("VPI_XR_SCALE").ok().and_then(|v| v.parse::<f32>().ok()).map(|s| s.clamp(0.3, 1.0));
            // or switch between two (`VPI_XR_SCALE_STEPS=0.8,1.0,2`: every 2 s)
            let steps = std::env::var("VPI_XR_SCALE_STEPS").ok().and_then(|v| {
                let p: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                (p.len() == 3).then(|| (p[0].clamp(0.3, 1.0), p[1].clamp(0.3, 1.0), p[2].max(0.1)))
            });
            let held = held.or(steps.map(|s| s.1));
            let resolution = Resolution {
                on: held.is_none() && settings.get("PlayerVR", "DynamicResolution").and_then(crate::settings::vpx_bool).unwrap_or(true),
                target: settings.float_or("PlayerVR", "DynamicResolutionTarget", 0.85, 0.5, 1.0, 0.0),
                min_scale: settings.float_or("PlayerVR", "DynamicResolutionMinScale", 0.70, 0.3, 1.0, 0.0),
                scale: held.unwrap_or(1.0),
                budget_ms: f32::MAX,
                smoothed_ms: 0.0,
                lowered: Instant::now(),
                settle_until: None,
                steps,
            };
            // the panel, the pause menu and the laser as the runtime's quad layers (`[PlayerVR]`
            // `PanelQuadLayers`, ours, on by default), else drawn into the eyes; `VPI_XR_QUAD_LAYERS` (1 or 0)
            // over it, for tests
            let quad_layers = match std::env::var("VPI_XR_QUAD_LAYERS").ok().as_deref() {
                Some("1") => true,
                Some("0") => false,
                _ => settings.get("PlayerVR", "PanelQuadLayers").and_then(crate::settings::vpx_bool).unwrap_or(true),
            };
            let mut runtime = Runtime::start(options.passthrough, level != Level::Off)?;
            let rec = &runtime.views[0];
            println!(
                "VR: {} with {}, each eye {}x{} as the runtime recommends (at most {}x{}), {:?}{}",
                runtime.name,
                runtime.headset,
                rec.recommended_image_rect_width,
                rec.recommended_image_rect_height,
                rec.max_image_rect_width,
                rec.max_image_rect_height,
                runtime.blend,
                if runtime.extensions.len() > 1 { format!(", with {}", runtime.extensions[1..].join(", ")) } else { String::new() }
            );
            // the player's resolution, as VPX takes it (`[PlayerVR]` `ResFactor`, a table's own .ini over
            // the settings'): that fraction of the largest the runtime takes, else its recommended size
            let res_factor = settings.float_or("PlayerVR", "ResFactor", -1.0, -1.0, 10.0, 0.0);
            let size = match options.eye_size {
                Some(s) => s,
                None if res_factor > 0.1 => (
                    ((rec.max_image_rect_width as f32 * res_factor) as u32).clamp(1, rec.max_image_rect_width),
                    ((rec.max_image_rect_height as f32 * res_factor) as u32).clamp(1, rec.max_image_rect_height),
                ),
                None if options.library && !quad_layers => (
                    ((rec.recommended_image_rect_width as f32 * LIBRARY_EYE_SCALE) as u32).clamp(1, rec.max_image_rect_width.max(1)),
                    ((rec.recommended_image_rect_height as f32 * LIBRARY_EYE_SCALE) as u32).clamp(1, rec.max_image_rect_height.max(1)),
                ),
                None => (rec.recommended_image_rect_width.max(1), rec.recommended_image_rect_height.max(1)),
            };
            let why = if res_factor > 0.1 {
                format!(" (ResFactor {res_factor})")
            } else if options.library && !quad_layers && options.eye_size.is_none() {
                format!(" ({LIBRARY_EYE_SCALE} times the recommended size, for the library's text)")
            } else {
                String::new()
            };
            println!("VR: each eye drawn {}x{}{why}", size.0, size.1);
            // SAFETY: SDL's video is started, on this thread; the runtime is boxed and lives as long
            // as the device (dropped after it, below)
            let device = unsafe { runtime.create_device()? };
            // SAFETY: the device was just made, on this thread
            let session = match unsafe { Session::new(&runtime, device, size) } {
                Ok(s) => s,
                Err(e) => {
                    // SAFETY: nothing else uses the device yet
                    unsafe { SDL_DestroyGPUDevice(device) };
                    return Err(e);
                }
            };
            if !session.floor {
                println!("VR: the runtime has no stage space: the table is placed from the headset's starting point, not the floor");
            }
            if level != Level::Off {
                // SAFETY: the device's own properties
                let texel = unsafe { SDL_GetNumberProperty(SDL_GetGPUDeviceProperties(device), crate::xr::VULKAN_SHADING_RATE_TEXEL.as_ptr(), 0) };
                if texel > 0 {
                    println!(
                        "VR foveation: {level:?}, shading rate images of {texel} px texels{}; {}",
                        if foveation.debug { ", the gaze check's small spot" } else { "" },
                        match (foveation.eye_tracked, runtime.eye_gaze, runtime.eye_tracked_foveation) {
                            (false, ..) => "fixed (FoveationEyeTracked off)",
                            (true, true, _) => "following the gaze when the eye tracker gives it",
                            (true, false, true) => "following the runtime's foveation centres when the eye tracker gives them",
                            (true, false, false) => "fixed: the runtime has no eye tracking",
                        }
                    );
                } else {
                    println!("VR foveation: off, the GPU has no shading rate images");
                    foveation.level = Level::Off;
                }
            }
            // the controllers; without them the table still shows
            let controls = match Controls::new(&runtime, session.handles().0) {
                Ok(c) => Some(c),
                Err(e) => {
                    eprintln!("VR: no controllers: {e}");
                    None
                }
            };
            let placement = Placement::from_settings(settings);
            let timings = match &options.timings {
                Some(path) => {
                    let mut f = std::io::BufWriter::new(std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?);
                    let _ = writeln!(f, "frame,ms,waited_ms,cpu_ms,period_ms,should_render,located,drawn,gpu_ms");
                    Some(f)
                }
                None => None,
            };
            // the panel, the pause menu and the laser drawn into the eyes when quad layers aren't asked for
            let room = if quad_layers {
                println!("VR: the panel, the pause menu and the laser as the runtime's quad layers");
                None
            } else {
                match RoomPictures::new(device, 4) {
                    Ok(r) => {
                        println!("VR: the panel, the pause menu and the laser drawn into the eyes ([PlayerVR] PanelQuadLayers 0)");
                        Some(r)
                    }
                    Err(e) => {
                        eprintln!("VR: the panel, the pause menu and the laser as the runtime's quad layers: {e}");
                        None
                    }
                }
            };
            // test runs: the right hand pointing at the panel and taking hold of it for a time
            let test_hand = std::env::var("VPI_XR_TEST_HAND").ok().and_then(|v| {
                let (span, degrees) = v.split_once(':')?;
                let (from, to) = span.split_once('-')?;
                Some((from.trim().parse().ok()?, to.trim().parse().ok()?, degrees.trim().parse().ok()?))
            });
            let mut vr = Vr {
                controls,
                session: Some(session),
                device,
                runtime,
                placement,
                measures,
                to_room: Mat4::IDENTITY,
                scale: 1.0,
                far: place::far_plane(bounds.0, bounds.1),
                options,
                next_capture: 0,
                timings,
                stats: Stats::default(),
                started: Instant::now(),
                frames: 0,
                foveation,
                resolution,
                recentred: false,
                panel: None,
                menu: None,
                keys_kept: [false; 6],
                next_panel_capture: 0,
                room,
                laser: None,
                dot: None,
                still_pictures: Vec::new(),
                test_hand,
                test_way: None,
            };
            vr.place();
            Ok(vr)
        }

        /// Starts VR with no table: the headset's panel alone (`--library`).
        pub fn start_alone(settings: &crate::settings::Settings, options: VrOptions) -> Result<Vr, String> {
            // a table of VPX's size, never drawn: the placement's sums only
            let measures = TableMeasures { left: 0.0, top: 0.0, right: 952.0, bottom: 2162.0, glass_bottom: 210.0, slope_deg: 6.0 };
            Vr::start(settings, VrOptions { library: true, ..options }, measures, (Vec3::new(0.0, 0.0, 0.0), Vec3::new(952.0, 2162.0, 400.0)))
        }

        /// Opens the panel on the library under `library`, the process started the first time (its "Back
        /// to" going to `back`); placed before the head on the next frame.
        pub fn open_panel(&mut self, library: &std::path::Path, back: Option<&std::path::Path>) -> Result<(), String> {
            let Some(session) = self.session.as_mut() else { return Err("no VR session".into()) };
            if self.panel.is_none() {
                if self.room.is_none() {
                    // SAFETY: on the thread that draws, the session running
                    unsafe { session.make_quad(&self.runtime, Layer::Panel, panel::SIZE.0, panel::SIZE.1)? };
                }
                self.panel = Some(PanelView {
                    panel: Panel::start(library, back)?,
                    pose: None,
                    shown: false,
                    hand: 1,
                    pressed: [false; 2],
                    at: None,
                    pictures: 0,
                    newest: None,
                    placed_focused: false,
                    solid: Vec::new(),
                    broken_kept: 0,
                    grab: None,
                    gripped: [false; 2],
                    frames: 0,
                    swapped_at: 0,
                    sent_at: 0,
                    put_up: 0,
                    slowest: 0,
                    longest_wait: std::time::Duration::ZERO,
                });
            }
            if let Some(view) = self.panel.as_mut() {
                view.shown = true;
                view.pose = None;
                view.panel.send(Input::Visible(true));
            }
            Ok(())
        }

        /// Puts the panel away (its process stays, for the next time).
        pub fn close_panel(&mut self) {
            if let Some(view) = self.panel.as_mut() {
                view.shown = false;
                view.at = None;
                view.panel.send(Input::Leave);
                view.panel.send(Input::Visible(false));
            }
        }

        /// Input for the panel from elsewhere than the controllers (test runs' clicks).
        pub fn panel_input(&self, input: Input) {
            if let Some(view) = self.panel.as_ref().filter(|v| v.shown) {
                view.panel.send(input);
            }
        }

        pub fn panel_shown(&self) -> bool {
            self.panel.as_ref().is_some_and(|v| v.shown)
        }

        /// The panel's requests since the last asked; Close too when it ended by itself or failed.
        pub fn panel_requests(&mut self) -> Vec<Request> {
            let Some(view) = self.panel.as_mut() else { return Vec::new() };
            let mut asked: Vec<Request> = std::iter::from_fn(|| view.panel.request()).collect();
            if !view.panel.running() {
                eprintln!("panel: it ended or failed: put away");
                self.panel = None;
                asked.push(Request::Close);
            }
            asked
        }

        /// The pointer on the panel and on the pause menu: where the pointing hand's ray meets it, its
        /// trigger the left button (the other hand's trigger takes the pointer), the right stick the
        /// wheel; a grip pressed on either takes hold of it to move it, the right stick then sending it
        /// away or bringing it nearer (`held`). The laser goes from the pointing hand to the panel, else
        /// to the pause menu.
        fn point(&mut self, frame: &VrFrame) {
            let (aims, p) = {
                let (Some(session), Some(controls)) = (self.session.as_ref(), self.controls.as_ref()) else { return };
                let (space, time) = (session.handles().1, frame.frame.display_time);
                ([0, 1].map(|hand| controls.aim(&self.runtime, space, time, hand)), controls.pointing)
            };
            let (aims, grips) = self.tested_hands(frame, aims, p.grip);
            let reach = if p.stick[1].abs() > 0.2 { (p.stick[1].abs() - 0.2) / 0.8 * GRAB_SPEED * frame.frame.period_ns as f32 / 1e9 * p.stick[1].signum() } else { 0.0 };
            (self.laser, self.dot) = (None, None);
            if let Some(view) = self.panel.as_mut().filter(|v| v.shown)
                && let Some(pose) = view.pose
            {
                let was_held = view.grab.is_some();
                let pose = held(&mut view.grab, &mut view.gripped, pose, PANEL_METRES, aims, grips, reach);
                view.pose = Some(pose);
                match view.grab {
                    Some(g) => view.hand = g.hand,
                    None if was_held => println!("panel: moved to {:.2}, {:.2}, {:.2}", pose.position[0], pose.position[1], pose.position[2]),
                    None => {}
                }
                for hand in 0..2 {
                    if p.trigger[hand] > 0.6 && !view.pressed[hand] && hand != view.hand {
                        view.hand = hand;
                    }
                }
                let aim = aims[view.hand];
                let (w, h) = (panel::SIZE.0 as f32, panel::SIZE.1 as f32);
                let on = aim.and_then(|a| hit(&a, &pose, PANEL_METRES));
                self.laser = aim.map(|a| (a, on.map(|uv| on_panel(&pose, PANEL_METRES, uv))));
                match on {
                    Some((u, v)) => {
                        let at = (u * w, v * h);
                        if view.at != Some(at) {
                            view.panel.send(Input::Move { x: at.0, y: at.1 });
                        }
                        view.at = Some(at);
                    }
                    None => {
                        if view.at.take().is_some() {
                            view.panel.send(Input::Leave);
                        }
                    }
                }
                for hand in 0..2 {
                    let down = if view.pressed[hand] { p.trigger[hand] > 0.5 } else { p.trigger[hand] > 0.6 };
                    if down != view.pressed[hand] {
                        view.pressed[hand] = down;
                        if hand == view.hand
                            && let Some((x, y)) = view.at
                        {
                            view.panel.send(Input::Button { button: 0, down, x, y });
                        }
                    }
                }
                if view.grab.is_none()
                    && p.stick[1].abs() > 0.2
                    && let Some((x, y)) = view.at
                {
                    // up to 40 px a frame, about 2900 px/s at 72 Hz, with a dead zone
                    let speed = (p.stick[1].abs() - 0.2) / 0.8;
                    view.panel.send(Input::Wheel { x, y, dx: 0.0, dy: speed.copysign(p.stick[1]) * 40.0 });
                }
            }
            // the pause menu as the desktop's mouse points at it (`pause_menu::point`): a row pointed at
            // is picked, the trigger chooses it (on a bar, sets it there, held to drag it)
            if let Some(m) = self.menu.as_mut().filter(|m| m.shown)
                && let Some(pose) = m.pose
            {
                use crate::pause_menu::point;
                let size = m.metres();
                let pose = held(&mut m.grab, &mut m.gripped, pose, size, aims, grips, reach);
                m.pose = Some(pose);
                if let Some(g) = m.grab {
                    m.hand = g.hand;
                }
                for hand in 0..2 {
                    if p.trigger[hand] > 0.6 && !m.pressed[hand] && hand != m.hand {
                        m.hand = hand;
                    }
                }
                let on = aims[m.hand].and_then(|a| hit(&a, &pose, size));
                if self.laser.is_none() {
                    self.laser = aims[m.hand].map(|a| (a, on.map(|uv| on_panel(&pose, size, uv))));
                    // the dot where it meets the menu, a little before it
                    self.dot = on.map(|uv| {
                        let (at, front) = (on_panel(&pose, size, uv), rotate(pose.orientation, [0.0, 0.0, 1.0]));
                        Pose { position: [0, 1, 2].map(|i| at[i] + front[i] * DOT_BEFORE), orientation: pose.orientation }
                    });
                }
                let at = on.map(|(u, v)| (m.at.0 + u * m.size.0 as f32, m.at.1 + v * m.size.1 as f32));
                m.aiming = at.is_some();
                let rows = m.layout.rows.len();
                if let Some(row) = m.dragging {
                    if let Some(along) = at.and_then(|(x, _)| m.layout.along(row, x)) {
                        point(Pointer::Drag(row, along, rows));
                    }
                } else {
                    let over = at.and_then(|(x, y)| m.layout.at(x, y)).map(|(k, _)| k);
                    if over != m.hover {
                        m.hover = over;
                        if let Some(k) = over {
                            point(Pointer::Over(k, rows));
                        }
                    }
                }
                for hand in 0..2 {
                    let down = if m.pressed[hand] { p.trigger[hand] > 0.5 } else { p.trigger[hand] > 0.6 };
                    if down != m.pressed[hand] {
                        m.pressed[hand] = down;
                        if hand != m.hand {
                            continue;
                        }
                        if !down {
                            m.dragging = None;
                        } else if let Some((k, spot)) = at.and_then(|(x, y)| m.layout.at(x, y)) {
                            point(Pointer::Click(k, spot, rows));
                            if matches!(spot, Spot::Bar(_)) {
                                m.dragging = Some(k);
                            }
                        }
                    }
                }
                // the stick scrolls the rules and the controls: up to 12 notches a second
                if m.grab.is_none() && p.stick[1].abs() > 0.2 && at.is_some() {
                    m.wheel += (p.stick[1].abs() - 0.2) / 0.8 * 12.0 / 72.0 * p.stick[1].signum();
                    while m.wheel.abs() >= 1.0 {
                        point(Pointer::Wheel(m.wheel.signum() as i32));
                        m.wheel -= m.wheel.signum();
                    }
                } else {
                    m.wheel = 0.0;
                }
            }
        }

        /// The pause menu's picture from now on (and where its rows are, for pointing), or none: a panel
        /// in the room, placed before the head as it opens. `screen`: the size of the screen it was laid
        /// out for (the window's), whose height stands for `MENU_SCREEN_METRES` in the room.
        pub fn set_menu(&mut self, made: Option<&(MenuPicture, Layout)>, screen: (u32, u32)) -> Result<(), String> {
            let Some(session) = self.session.as_mut() else { return Ok(()) };
            let Some((picture, layout)) = made else {
                if let Some(m) = self.menu.as_mut() {
                    (m.shown, m.pose, m.hover, m.dragging, m.aiming) = (false, None, None, None, false);
                }
                return Ok(());
            };
            if picture.width == 0 || picture.height == 0 {
                return Ok(());
            }
            match self.room.as_mut() {
                Some(room) => room.set(slot(Layer::Menu), &picture.pixels, picture.width, picture.height, false, true)?,
                None => {
                    // SAFETY: on the thread that draws, the session running
                    unsafe { session.make_quad(&self.runtime, Layer::Menu, picture.width, picture.height)? };
                    session.quad_picture(&self.runtime, Layer::Menu, &picture.pixels, picture.width, picture.height, false, true)?;
                }
            }
            let m = self.menu.get_or_insert_with(MenuView::default);
            (m.size, m.at, m.screen_height, m.layout, m.shown) = ((picture.width, picture.height), picture.at, screen.1 as f32, layout.clone(), true);
            // test runs: each picture of the menu's as it came
            if let Some(dir) = &self.options.capture {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                let path = dir.join(format!("menu-{}.png", self.started.elapsed().as_millis()));
                image::save_buffer(&path, &picture.pixels, picture.width, picture.height, image::ExtendedColorType::Rgba8).map_err(|e| format!("save {}: {e}", path.display()))?;
                println!("saved {} ({}x{}, the pause menu, {:.2} by {:.2} m)", path.display(), picture.width, picture.height, m.metres().0, m.metres().1);
            }
            Ok(())
        }

        /// The pause menu this frame: placed before the head as it opened.
        fn menu_quad(&mut self, frame: &VrFrame) -> Option<Quad> {
            let m = self.menu.as_mut().filter(|m| m.shown)?;
            if m.pose.is_none() && frame.frame.located {
                m.pose = Some(before(&Pose::middle(&frame.frame.views[0], &frame.frame.views[1]), MENU_AWAY));
            }
            Some(Quad { pose: m.pose?, size: m.metres(), rect: m.size })
        }

        /// The quads over the eyes this frame: the panel, then the pause menu, then the laser and the
        /// pointer's dot over them.
        fn quads(&mut self, frame: &VrFrame) -> Result<Vec<(Layer, Quad)>, String> {
            let mut quads = Vec::with_capacity(4);
            if let Some(q) = self.panel_quad(frame)? {
                quads.push((Layer::Panel, q));
            }
            if let Some(q) = self.menu_quad(frame) {
                quads.push((Layer::Menu, q));
            }
            if let Some(q) = self.laser_quad(frame)? {
                quads.push((Layer::Laser, q));
            }
            if let Some(pose) = self.dot.filter(|_| frame.frame.located) {
                self.still_picture(Layer::Dot, DOT_PICTURE, dot_picture)?;
                quads.push((Layer::Dot, Quad { pose, size: (DOT_WIDTH, DOT_WIDTH), rect: DOT_PICTURE }));
            }
            Ok(quads)
        }

        /// The laser this frame (`beam`), as the pointing hand aims.
        fn laser_quad(&mut self, frame: &VrFrame) -> Result<Option<Quad>, String> {
            let f = &frame.frame;
            let Some((aim, to)) = self.laser.filter(|_| f.located) else { return Ok(None) };
            let Some((pose, size)) = beam(&aim, to, Pose::middle(&f.views[0], &f.views[1]).position) else { return Ok(None) };
            self.still_picture(Layer::Laser, LASER_PICTURE, laser_picture)?;
            Ok(Some(Quad { pose, size, rect: LASER_PICTURE }))
        }

        /// A picture that never changes (`size`, RGBA with straight alpha, made by `pixels`), uploaded for
        /// `layer` the first time it's wanted: into the room's drawing, or the quad layer's swapchain.
        fn still_picture(&mut self, layer: Layer, (w, h): (u32, u32), pixels: fn() -> Vec<u8>) -> Result<(), String> {
            if self.still_pictures.contains(&layer) {
                return Ok(());
            }
            let Some(session) = self.session.as_mut() else { return Ok(()) };
            let pixels = pixels();
            match self.room.as_mut() {
                Some(room) => room.set(slot(layer), &pixels, w, h, false, true)?,
                None => {
                    // SAFETY: on the thread that draws, the session running
                    unsafe { session.make_quad(&self.runtime, layer, w, h)? };
                    session.quad_picture(&self.runtime, layer, &pixels, w, h, false, true)?;
                }
            }
            self.still_pictures.push(layer);
            Ok(())
        }

        /// Test runs (`VPI_XR_TEST_HAND`): the right hand held 0.2 m right, 0.3 m below and 0.25 m before
        /// the head, pointing at the middle of the panel (of the pause menu, with no panel) as first found,
        /// its grip held from `from` to `to` ms into VR as it turns `degrees` to the left about the
        /// vertical, carrying what it holds round. Otherwise the hands as the controllers have them.
        fn tested_hands(&mut self, frame: &VrFrame, mut aims: [Option<Pose>; 2], mut grips: [bool; 2]) -> ([Option<Pose>; 2], [bool; 2]) {
            let Some((from, to, degrees)) = self.test_hand.filter(|_| frame.frame.located) else { return (aims, grips) };
            let head = Pose::middle(&frame.frame.views[0], &frame.frame.views[1]).position;
            let hand = [head[0] + 0.2, head[1] - 0.3, head[2] - 0.25];
            if self.test_way.is_none()
                && let Some(t) = self.panel.as_ref().filter(|v| v.shown).and_then(|v| v.pose).or_else(|| self.menu.as_ref().filter(|m| m.shown).and_then(|m| m.pose))
            {
                let d = [0, 1, 2].map(|i| t.position[i] - hand[i]);
                let n = dot(d, d).sqrt().max(1e-6);
                self.test_way = Some(d.map(|c| c / n));
            }
            let Some(way) = self.test_way else { return (aims, grips) };
            let ms = self.started.elapsed().as_millis() as u64;
            let share = (ms.saturating_sub(from) as f32 / to.saturating_sub(from).max(1) as f32).clamp(0.0, 1.0);
            let angle = (degrees * share).to_radians();
            aims[1] = Some(Pose { position: hand, orientation: look(rotate([0.0, (angle / 2.0).sin(), 0.0, (angle / 2.0).cos()], way)) });
            grips[1] = (from..to).contains(&ms);
            (aims, grips)
        }

        /// The panel this frame: placed when it opened, its newest picture uploaded; none while it's away.
        fn panel_quad(&mut self, frame: &VrFrame) -> Result<Option<Quad>, String> {
            let (Some(session), Some(view)) = (self.session.as_mut(), self.panel.as_mut()) else { return Ok(None) };
            if !view.shown {
                return Ok(None);
            }
            // placed again as the headset gives the controllers, if it was placed before they came (behind
            // the runtime's own menu, as it is when the game is started from it)
            let focused = session.focused;
            if focused && !view.placed_focused && view.pose.is_some() {
                view.pose = None;
            }
            if view.pose.is_none() && frame.frame.located {
                let pose = before(&Pose::middle(&frame.frame.views[0], &frame.frame.views[1]), PANEL_AWAY);
                println!("panel: placed {PANEL_AWAY} m before the head, at {:.2}, {:.2}, {:.2}{}", pose.position[0], pose.position[1], pose.position[2], if focused { "" } else { " (not focused yet)" });
                view.pose = Some(pose);
                view.placed_focused = focused;
            }
            let elapsed_ms = self.started.elapsed().as_millis() as u64;
            let capturing = self.options.capture.is_some() && self.next_panel_capture < self.options.capture_at.len();
            // as a quad layer, the picture sent last shown from this frame on, once the GPU has written it
            // (the panel's two swapchains change places: `xr::QuadChain`)
            view.frames += 1;
            let layered = self.room.is_none();
            if layered && session.quad_settle(&self.runtime, Layer::Panel, false)? {
                view.slowest = view.slowest.max(view.frames - view.sent_at);
                view.swapped_at = view.frames;
            }
            if view.frames % 360 == 0 && view.put_up > 0 {
                if layered {
                    println!(
                        "panel: {} pictures put up in 5 s, each shown within {} frames of being sent, the runtime giving each image within {:.1} ms",
                        view.put_up,
                        view.slowest,
                        view.longest_wait.as_secs_f64() * 1000.0
                    );
                } else {
                    println!("panel: {} pictures in 5 s, drawn into the eyes", view.put_up);
                }
                (view.put_up, view.slowest, view.longest_wait) = (0, 0, std::time::Duration::ZERO);
            }
            // drawn into the eyes, its newest picture each frame; as a quad layer, the next picture
            // written into the swapchain not shown, a frame after they last changed places at the soonest,
            // as SteamVR may still show the other till then, the panel's newest picture waiting meanwhile
            if (!layered || (!session.quad_writing(Layer::Panel) && view.frames > view.swapped_at))
                && let Some(picture) = view.panel.picture()
            {
                match self.room.as_mut() {
                    Some(room) => room.set(slot(Layer::Panel), picture.pixels, picture.width, picture.height, true, false)?,
                    None => {
                        let waited = session.quad_picture(&self.runtime, Layer::Panel, picture.pixels, picture.width, picture.height, true, false)?;
                        view.sent_at = view.frames;
                        view.longest_wait = view.longest_wait.max(waited);
                    }
                }
                view.put_up += 1;
                view.pictures += 1;
                if view.pictures == 1 {
                    println!("panel: its first picture, {}x{}, {elapsed_ms} ms into VR", picture.width, picture.height);
                }
                // blocks gone solid black or white, as the owner saw in the headset over the library's
                // left column: logged, and the first few pictures kept, to tell Chromium's drawing wrong
                // from the runtime's showing it wrong
                let solid = panel::solid_blocks(picture.pixels, picture.width, picture.height);
                if view.pictures > 1 && solid.len() == view.solid.len() {
                    let across = (picture.width / panel::BLOCK) as usize;
                    let new: Vec<(usize, usize)> = solid.iter().zip(&view.solid).enumerate().filter(|(_, (now, before))| **now && !**before).map(|(i, _)| (i % across, i / across)).collect();
                    if !new.is_empty() {
                        let side = panel::BLOCK as usize;
                        let (x0, x1) = (new.iter().map(|b| b.0).min().unwrap_or(0) * side, (new.iter().map(|b| b.0).max().unwrap_or(0) + 1) * side);
                        let (y0, y1) = (new.iter().map(|b| b.1).min().unwrap_or(0) * side, (new.iter().map(|b| b.1).max().unwrap_or(0) + 1) * side);
                        println!("panel: picture {} has {} blocks of solid black or white it hadn't, within x {x0}..{x1}, y {y0}..{y1}, {elapsed_ms} ms into VR", picture.number, new.len());
                        let dir = self.options.capture.clone().or_else(|| vpi_settings::data_folder().map(|d| d.join("panel-checks")));
                        if view.broken_kept < 3
                            && let Some(dir) = dir
                        {
                            view.broken_kept += 1;
                            let (pixels, width, height, number) = (picture.pixels.to_vec(), picture.width, picture.height, picture.number);
                            // written aside, the frame not held up by it
                            std::thread::spawn(move || {
                                let rgba: Vec<u8> = pixels.chunks_exact(4).flat_map(|c| [c[2], c[1], c[0], 255]).collect();
                                let path = dir.join(format!("panel-solid-{number}.png"));
                                if let Err(e) = std::fs::create_dir_all(&dir).map_err(|e| e.to_string()).and_then(|()| image::save_buffer(&path, &rgba, width, height, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())) {
                                    eprintln!("panel: {}: {e}", path.display());
                                }
                            });
                        }
                    }
                }
                view.solid = solid;
                if capturing {
                    view.newest = Some((picture.pixels.to_vec(), picture.width, picture.height, picture.number));
                }
            }
            // test runs: the panel's newest picture as the player got it, at the capture times
            if let (Some(dir), Some(&at), Some((pixels, width, height, number))) = (&self.options.capture, self.options.capture_at.get(self.next_panel_capture), &view.newest)
                && elapsed_ms >= at
            {
                self.next_panel_capture += 1;
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                let rgba: Vec<u8> = pixels.chunks_exact(4).flat_map(|c| [c[2], c[1], c[0], 255]).collect();
                let path = dir.join(format!("panel-{at}.png"));
                image::save_buffer(&path, &rgba, *width, *height, image::ExtendedColorType::Rgba8).map_err(|e| format!("save {}: {e}", path.display()))?;
                println!("saved {} ({width}x{height}, the panel's picture {number})", path.display());
                if self.next_panel_capture >= self.options.capture_at.len() {
                    view.newest = None;
                }
            }
            Ok(view.pose.map(|pose| Quad { pose, size: PANEL_METRES, rect: panel::SIZE }))
        }

        /// Hands a frame over with nothing but the panel (`--library`): drawn into the eyes over near
        /// black, or over the room itself with passthrough; as the runtime's quad layer, over a projection
        /// layer of that alone, as compositors expect one. Test runs save the eyes' pictures at the
        /// capture times.
        pub fn end_frame_alone(&mut self, frame: VrFrame) -> Result<(), String> {
            let elapsed_ms = self.started.elapsed().as_millis() as u64;
            let quads = self.quads(&frame)?;
            let colour = if self.passthrough() { SDL_FColor { r: 0.0, g: 0.0, b: 0.0, a: 0.0 } } else { LIBRARY_DARK };
            let Some(session) = self.session.as_mut() else { return Ok(()) };
            let f = &frame.frame;
            let rect = if !(f.should_render && f.located) {
                (0, 0)
            } else if let Some(room) = self.room.as_mut() {
                let targets = [session.eye_texture(&self.runtime, 0)?, session.eye_texture(&self.runtime, 1)?];
                let size = session.eye_size();
                room.draw(targets, session.format, size, Some(colour), room_eyes(f), &room_quads(&quads))?;
                if let (Some(dir), Some(&at)) = (&self.options.capture, self.options.capture_at.get(self.next_capture))
                    && elapsed_ms >= at
                {
                    self.next_capture += 1;
                    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                    for (eye, name) in [(0, "left"), (1, "right")] {
                        let rgba = crate::render::room::read_back(self.device, targets[eye], size, session.format)?;
                        let path = dir.join(format!("eye-{at}-{name}.png"));
                        image::save_buffer(&path, &rgba, size.0, size.1, image::ExtendedColorType::Rgba8).map_err(|e| format!("save {}: {e}", path.display()))?;
                        println!("saved {} ({}x{}, frame {})", path.display(), size.0, size.1, self.frames);
                    }
                }
                size
            } else {
                session.clear_eyes(&self.runtime, colour)?
            };
            let layers: &[(Layer, Quad)] = if self.room.is_some() { &[] } else { &quads };
            session.end_frame(&self.runtime, f, rect != (0, 0), rect, layers)?;
            let (gpu_ms, size) = (session.gpu_ms(), session.eye_size());
            self.count_frame(&frame, elapsed_ms, rect != (0, 0), size, gpu_ms);
            Ok(())
        }

        /// The device the renderer draws with (the session's).
        pub fn device(&self) -> *mut SDL_GPUDevice {
            self.device
        }

        /// Whether the world shows where nothing is drawn (the runtime blends by the picture's alpha).
        pub fn passthrough(&self) -> bool {
            self.runtime.blend == openxr_sys::EnvironmentBlendMode::ALPHA_BLEND
        }

        /// The format of the eyes' images.
        pub fn format(&self) -> SDL_GPUTextureFormat {
            self.session.as_ref().map_or(SDL_GPU_TEXTUREFORMAT_INVALID, |s| s.format)
        }

        fn place(&mut self) {
            self.scale = self.placement.scale(&self.measures);
            self.to_room = self.placement.playfield_to_room(&self.measures);
        }

        /// Hears the headset's events; false once the session is over (the player quit from the
        /// headset's menu, or the runtime ended it).
        pub fn poll(&mut self) -> bool {
            let Some(session) = self.session.as_mut() else { return false };
            let mut profile_changed = false;
            session.poll(&self.runtime, |e| profile_changed |= e.ty == openxr_sys::StructureType::EVENT_DATA_INTERACTION_PROFILE_CHANGED);
            if profile_changed && let Some(c) = &self.controls {
                println!("VR controls: {}", c.profiles(&self.runtime, session.handles().0));
            }
            !session.exit
        }

        /// Ends the session (the player quit): it ends through the headset's events.
        pub fn request_exit(&mut self) {
            if let Some(s) = self.session.as_mut() {
                s.request_exit(&self.runtime);
            }
        }

        /// Faces the table where the head looks and moves it to where the player stands, as VPX's
        /// headset view centering does, from the middle of the eyes' poses now.
        pub fn recentre(&mut self, frame: &VrFrame) {
            if !frame.frame.located {
                return;
            }
            let middle = Pose::middle(&frame.frame.views[0], &frame.frame.views[1]);
            // the panel and the pause menu come before the head again too
            if let Some(view) = self.panel.as_mut() {
                view.pose = None;
            }
            if let Some(m) = self.menu.as_mut() {
                m.pose = None;
            }
            self.placement = self.placement.recentred(middle, [0.0, 0.0]);
            self.place();
            self.recentred = true;
            println!(
                "VR: table recentred: turned {:.1} degrees, at {:.1}, {:.1} cm",
                self.placement.orientation_deg, self.placement.table_cm[0], self.placement.table_cm[1]
            );
        }

        /// Waits for the headset's next frame; None while it isn't showing yet (the session hasn't
        /// begun): try again shortly.
        pub fn begin_frame(&mut self) -> Result<Option<VrFrame>, String> {
            let Some(session) = self.session.as_mut() else { return Ok(None) };
            let Some(mut frame) = session.begin_frame(&self.runtime)? else { return Ok(None) };
            if self.options.pin_eyes {
                frame.pin_eyes();
            }
            Ok(Some(VrFrame { frame, began: Instant::now() }))
        }

        /// Reads the controllers for this frame: each key they press or let go handed to `key` (VPX's
        /// actions by their keys, as the keyboard's), the table recentred when its button is pressed.
        pub fn read_controls(&mut self, frame: &VrFrame, mut key: impl FnMut(u8, bool)) {
            // a trigger pulled at the pause menu chooses on it: its flipper press, which changes the row
            // picked there, is kept from the game, and so is its release; so is a grip taking hold of it
            let aiming = self.menu.as_ref().is_some_and(|m| m.shown && (m.aiming || m.grab.is_some()));
            let kept = &mut self.keys_kept;
            let (Some(session), Some(controls)) = (self.session.as_ref(), self.controls.as_mut()) else { return };
            let recentre = controls.sync(&self.runtime, session.handles().0, session.focused, |dik, down| {
                if let Some(i) = KEPT_KEYS.iter().position(|&k| k == dik) {
                    if down && aiming {
                        kept[i] = true;
                        return;
                    }
                    if !down && std::mem::take(&mut kept[i]) {
                        return;
                    }
                }
                key(dik, down);
            });
            if recentre {
                self.recentre(frame);
            }
            if self.panel_shown() || self.menu.as_ref().is_some_and(|m| m.shown) {
                self.point(frame);
            } else {
                (self.laser, self.dot) = (None, None);
            }
        }

        /// Each eye's view of the table this frame; None when the headset wants nothing drawn (not worn,
        /// the eyes not found).
        pub fn eyes(&self, frame: &VrFrame) -> Option<[EyeView; 2]> {
            let f = &frame.frame;
            if !f.should_render || !f.located {
                return None;
            }
            Some([0, 1].map(|eye| {
                let m = place::eye_matrices(&f.views[eye], f.fov[eye], &self.to_room, self.scale, self.far);
                EyeView { view: m.view, projection: m.projection, eye: m.eye }
            }))
        }

        /// Draws the frame into the headset's images (when `inputs` has both eyes), the pause menu over
        /// it, and hands it over; saves the eyes' pictures when a capture is due.
        pub fn end_frame(&mut self, frame: VrFrame, renderer: &mut Renderer, inputs: Option<[&FrameInput; 2]>) -> Result<(), String> {
            let quads = self.quads(&frame)?;
            let Some(session) = self.session.as_mut() else { return Ok(()) };
            // the part of each eye's image drawn this frame (dynamic resolution), which the runtime shows
            let (w, h) = self.resolution.size(session.eye_size());
            let elapsed_ms = self.started.elapsed().as_millis() as u64;
            let mut drawn = false;
            if let Some(inputs) = inputs {
                let targets = [session.eye_texture(&self.runtime, 0)?, session.eye_texture(&self.runtime, 1)?];
                // where each eye looks: the gaze ray, else the runtime's centres
                let f = &frame.frame;
                let fovea = if self.foveation.level == Level::Off || !renderer.foveation_possible() {
                    None
                } else {
                    let ray = self.controls.as_ref().and_then(|c| c.gaze(&self.runtime, session.handles().1, f.display_time));
                    self.foveation.discs(&f.views, &f.fov, ray, session.foveation_centres())
                };
                if fovea.is_some() {
                    self.stats.fovea(self.foveation.source);
                }
                renderer.draw_eyes(inputs, targets, w, h, session.eye_size(), fovea)?;
                if let Some(room) = self.room.as_mut().filter(|_| !quads.is_empty()) {
                    room.draw(targets, session.format, (w, h), None, room_eyes(f), &room_quads(&quads))?;
                }
                self.stats.triangles = renderer.triangles.get();
                drawn = true;
                if let (Some(dir), Some(&at)) = (&self.options.capture, self.options.capture_at.get(self.next_capture))
                    && elapsed_ms >= at
                {
                    self.next_capture += 1;
                    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                    for (eye, name) in [(0, "left"), (1, "right")] {
                        let rgba = renderer.capture_eye(targets[eye], w, h)?;
                        let path = dir.join(format!("eye-{at}-{name}.png"));
                        image::save_buffer(&path, &rgba, w, h, image::ExtendedColorType::Rgba8).map_err(|e| format!("save {}: {e}", path.display()))?;
                        println!("saved {} ({}x{}, frame {})", path.display(), w, h, self.frames);
                    }
                }
            }
            let layers: &[(Layer, Quad)] = if self.room.is_some() { &[] } else { &quads };
            session.end_frame(&self.runtime, &frame.frame, drawn, (w, h), layers)?;
            let gpu_ms = session.gpu_ms();
            let period_ms = self.count_frame(&frame, elapsed_ms, drawn, (w, h), gpu_ms);
            // the sizes it went through are in the perf line every 5 s
            self.resolution.update(gpu_ms, period_ms);
            Ok(())
        }

        /// A frame handed over: its line in the timings and its numbers in the perf line every 5 s. Its
        /// period, ms.
        fn count_frame(&mut self, frame: &VrFrame, elapsed_ms: u64, drawn: bool, size: (u32, u32), gpu_ms: Option<f32>) -> f32 {
            self.frames += 1;
            let cpu_ms = frame.began.elapsed().as_secs_f32() * 1e3;
            let waited_ms = frame.frame.waited_ns as f32 / 1e6;
            let period_ms = frame.frame.period_ns as f32 / 1e6;
            if let Some(t) = self.timings.as_mut() {
                let _ = writeln!(
                    t,
                    "{},{elapsed_ms},{waited_ms:.3},{cpu_ms:.3},{period_ms:.3},{},{},{},{}",
                    self.frames,
                    u8::from(frame.frame.should_render),
                    u8::from(frame.frame.located),
                    u8::from(drawn),
                    gpu_ms.map_or(String::new(), |g| format!("{g:.3}"))
                );
            }
            self.stats.add(cpu_ms, waited_ms, period_ms, drawn, gpu_ms, size);
            period_ms
        }
    }

    impl Stats {
        /// What placed a frame's foveation discs.
        fn fovea(&mut self, source: &'static str) {
            match self.fovea.iter_mut().find(|(s, _)| *s == source) {
                Some((_, n)) => *n += 1,
                None => self.fovea.push((source, 1)),
            }
        }

        /// A frame's numbers; every 5 s, the line in the log.
        fn add(&mut self, cpu_ms: f32, waited_ms: f32, period_ms: f32, drawn: bool, gpu_ms: Option<f32>, size: (u32, u32)) {
            let since = *self.since.get_or_insert_with(Instant::now);
            self.cpu_ms.push(cpu_ms);
            self.gpu_ms.extend(gpu_ms);
            self.waited_ms.push(waited_ms);
            self.drawn += u32::from(drawn);
            self.period_ms = period_ms;
            self.widths = Some(self.widths.map_or((size.0, size.0), |(a, b)| (a.min(size.0), b.max(size.0))));
            let secs = since.elapsed().as_secs_f32();
            if secs >= 5.0 {
                let pct = |v: &mut Vec<f32>, p: f32| {
                    v.sort_by(f32::total_cmp);
                    v.get(((v.len() as f32 - 1.0) * p).round() as usize).copied().unwrap_or(0.0)
                };
                let n = self.cpu_ms.len();
                let gpu = if self.gpu_ms.is_empty() {
                    String::new()
                } else {
                    format!("; GPU {:.1}/{:.1}/{:.1} ms (the runtime's count)", pct(&mut self.gpu_ms, 0.5), pct(&mut self.gpu_ms, 0.9), pct(&mut self.gpu_ms, 1.0))
                };
                let triangles = if self.triangles.0 > 0 {
                    format!(
                        "; triangles {:.2} M an eye, {:.0} % culled, {:.2} M in its mirrors",
                        self.triangles.0 as f64 / 1e6,
                        100.0 * self.triangles.1 as f64 / self.triangles.0 as f64,
                        self.triangles.2 as f64 / 1e6
                    )
                } else {
                    String::new()
                };
                let widths = match self.widths {
                    Some((a, b)) if a != b => format!(" ({a} to {b} wide in these)"),
                    _ => String::new(),
                };
                let fovea = if self.fovea.is_empty() {
                    String::new()
                } else {
                    format!("; foveation by {}", self.fovea.iter().map(|(s, n)| format!("{s} ({n})")).collect::<Vec<_>>().join(", "))
                };
                println!(
                    "VR perf: {n} frames in {secs:.1} s ({:.1} fps, {} drawn), each eye {}x{}{widths}, period {:.1} ms; CPU per frame {:.1}/{:.1}/{:.1} ms (p50/p90/max), waiting for the headset {:.1} ms p50{gpu}{triangles}{fovea}",
                    n as f32 / secs,
                    self.drawn,
                    size.0,
                    size.1,
                    self.period_ms,
                    pct(&mut self.cpu_ms, 0.5),
                    pct(&mut self.cpu_ms, 0.9),
                    pct(&mut self.cpu_ms, 1.0),
                    pct(&mut self.waited_ms, 0.5),
                );
                *self = Stats::default();
            }
        }
    }

    /// The settings that keep a table's placement (VPX's `SaveVRSettings`).
    pub(super) fn placement_changes(p: &Placement) -> [vpi_settings::Change; 6] {
        [
            vpi_settings::Change::set("PlayerVR", "Orientation", p.orientation_deg),
            vpi_settings::Change::set("PlayerVR", "TableX", p.table_cm[0]),
            vpi_settings::Change::set("PlayerVR", "TableY", p.table_cm[1]),
            vpi_settings::Change::set("PlayerVR", "TableZ", p.table_cm[2]),
            vpi_settings::Change::set("Player", "LockbarWidth", p.lockbar_width_cm),
            vpi_settings::Change::set("Player", "LockbarHeight", p.lockbar_height_cm),
        ]
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn near(a: [f32; 3], b: [f32; 3]) -> bool {
            (0..3).all(|i| (a[i] - b[i]).abs() < 1e-4)
        }

        /// A turn of `degrees` about `axis` (a unit vector).
        fn about(axis: [f32; 3], degrees: f32) -> [f32; 4] {
            let h = degrees.to_radians() / 2.0;
            [axis[0] * h.sin(), axis[1] * h.sin(), axis[2] * h.sin(), h.cos()]
        }

        #[test]
        fn a_pose_taken_into_a_frame_and_back_is_as_it_was() {
            let frame = Pose { position: [0.3, 1.2, -0.4], orientation: turned(about([0.0, 1.0, 0.0], 40.0), about([1.0, 0.0, 0.0], -25.0)) };
            let pose = Pose { position: [-0.5, 1.4, -1.6], orientation: about([0.0, 0.0, 1.0], 15.0) };
            let back = in_room(&frame, &in_frame(&frame, &pose));
            assert!(near(back.position, pose.position), "{back:?}");
            let v = [0.2, -0.7, 0.4];
            assert!(near(rotate(back.orientation, v), rotate(pose.orientation, v)), "{back:?}");
        }

        #[test]
        fn axes_make_the_turn_that_takes_the_rooms_to_them() {
            for q in [about([0.0, 1.0, 0.0], 170.0), about([1.0, 0.0, 0.0], -120.0), turned(about([0.0, 0.0, 1.0], 95.0), about([0.0, 1.0, 0.0], 60.0))] {
                let axes = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]].map(|a| rotate(q, a));
                let made = from_axes(axes[0], axes[1], axes[2]);
                for (a, want) in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]].iter().zip(axes) {
                    assert!(near(rotate(made, *a), want), "{q:?} -> {made:?}");
                }
            }
        }

        #[test]
        fn the_laser_runs_from_the_hand_to_the_panel_facing_the_head() {
            // a hand below and right of the head, aiming at a point ahead
            let head = [0.0, 1.6, 0.0];
            let start = [0.2, 1.3, -0.25];
            let to = [-0.1, 1.5, -1.5];
            let d = [0, 1, 2].map(|i| to[i] - start[i]);
            let n = dot(d, d).sqrt();
            let aim = Pose { position: start, orientation: look(d.map(|c| c / n)) };
            let (pose, size) = beam(&aim, Some(to), head).expect("a beam");
            assert!((size.1 - n).abs() < 1e-4 && size.0 == LASER_WIDTH, "{size:?}");
            // its top edge's middle at the panel, its bottom's at the hand
            let top = [0, 1, 2].map(|i| pose.position[i] + rotate(pose.orientation, [0.0, size.1 / 2.0, 0.0])[i]);
            let bottom = [0, 1, 2].map(|i| pose.position[i] + rotate(pose.orientation, [0.0, -size.1 / 2.0, 0.0])[i]);
            assert!(near(top, to) && near(bottom, start), "{top:?} {bottom:?}");
            // its face square to the line of sight across it
            let x = rotate(pose.orientation, [1.0, 0.0, 0.0]);
            let sight = [0, 1, 2].map(|i| head[i] - pose.position[i]);
            assert!(dot(x, sight).abs() < 1e-4, "{x:?}");
            // meeting nothing, it reaches LASER_MISS
            let (_, missed) = beam(&aim, None, head).expect("a beam");
            assert!((missed.1 - LASER_MISS).abs() < 1e-4);
        }

        #[test]
        fn the_dot_is_light_in_a_dark_rim_and_clear_beyond() {
            let (w, _) = DOT_PICTURE;
            let p = dot_picture();
            let at = |x: u32, y: u32| &p[((y * w + x) * 4) as usize..][..4];
            let middle = at(w / 2, w / 2);
            assert!(middle[0] > 230 && middle[3] == 255, "{middle:?}");
            // a quarter of the way in from the edge: the rim, dark and nearly opaque
            let rim = at(w / 2, w / 2 - (w as f32 * 0.38) as u32);
            assert!(rim[0] < 30 && rim[3] > 200, "{rim:?}");
            assert_eq!(at(0, 0)[3], 0);
        }

        #[test]
        fn a_gripped_panel_follows_the_hand_and_stays_where_let_go() {
            let size = (1.7, 1.0625);
            let panel = Pose { position: [0.0, 1.45, -1.5], orientation: [0.0, 0.0, 0.0, 1.0] };
            let hand = [0.2, 1.3, -0.25];
            let way = |degrees: f32| {
                let d = [0, 1, 2].map(|i| panel.position[i] - hand[i]);
                let n = dot(d, d).sqrt();
                Pose { position: hand, orientation: look(rotate(about([0.0, 1.0, 0.0], degrees), d.map(|c| c / n))) }
            };
            let (mut grab, mut gripped) = (None, [false; 2]);
            // a grip pressed off the panel takes nothing
            let away = Pose { position: hand, orientation: look([0.0, 0.0, 1.0]) };
            let p = held(&mut grab, &mut gripped, panel, size, [None, Some(away)], [false, true], 0.0);
            assert!(grab.is_none() && p == panel);
            // let go, then pressed on it: it's held, and turns round the hand with it
            held(&mut grab, &mut gripped, panel, size, [None, Some(way(0.0))], [false, false], 0.0);
            held(&mut grab, &mut gripped, panel, size, [None, Some(way(0.0))], [false, true], 0.0);
            assert!(grab.is_some());
            let turned_round = held(&mut grab, &mut gripped, panel, size, [None, Some(way(30.0))], [false, true], 0.0);
            let from_hand = |p: [f32; 3]| [0, 1, 2].map(|i| p[i] - hand[i]);
            let want = rotate(about([0.0, 1.0, 0.0], 30.0), from_hand(panel.position));
            assert!(near(from_hand(turned_round.position), want), "{turned_round:?}");
            // the stick sends it away along the hand's line to it, no farther than GRAB_FARTHEST
            let farther = held(&mut grab, &mut gripped, turned_round, size, [None, Some(way(30.0))], [false, true], 100.0);
            let reach = dot(from_hand(farther.position), from_hand(farther.position)).sqrt();
            assert!((reach - GRAB_FARTHEST).abs() < 1e-3, "{reach}");
            // let go: it stays
            let kept = held(&mut grab, &mut gripped, farther, size, [None, Some(way(60.0))], [false, false], 0.0);
            assert!(grab.is_none() && kept == farther);
        }

        #[test]
        fn a_kept_placement_reads_back_as_it_was() {
            let p = Placement { lockbar_width_cm: 68.5, lockbar_height_cm: 91.25, orientation_deg: -37.125, table_cm: [12.5, -40.75, 3.0] };
            let text = vpi_settings::edit("; mine\n[Player]\nSoundVolume = 80\n", &placement_changes(&p));
            assert!(text.starts_with("; mine\n[Player]\nSoundVolume = 80\n"), "{text}");
            assert_eq!(Placement::from_settings(&crate::settings::Settings::from_text(&text)), p);
        }
    }

    impl Vr {
        /// The table's placement kept in the settings file when VR ends, after a recentring (VPX's
        /// `VRDevice::SaveVRSettings` as its player ends: `[PlayerVR]` `Orientation`, `TableX/Y/Z`,
        /// `[Player]` `LockbarWidth/Height`), so the table stands there next time.
        fn keep_placement(&self) {
            let Some(file) = vpi_settings::settings_path().filter(|_| self.recentred) else { return };
            let before = std::fs::read(&file).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
            let after = vpi_settings::edit(&before, &placement_changes(&self.placement));
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let mut new_name = file.file_name().map(|n| n.to_os_string()).unwrap_or_default();
            new_name.push(".new");
            let new = file.with_file_name(new_name);
            match std::fs::write(&new, after).and_then(|_| std::fs::rename(&new, &file)) {
                Ok(()) => println!("VR: the table's placement kept in {}", file.display()),
                Err(e) => eprintln!("VR: the table's placement couldn't be kept ({}: {e})", file.display()),
            }
        }
    }

    impl Drop for Vr {
        fn drop(&mut self) {
            self.keep_placement();
            if let Some(mut c) = self.controls.take() {
                c.destroy(&self.runtime);
            }
            if let Some(mut s) = self.session.take() {
                s.close(&self.runtime);
            }
            // the room's pictures on the device, released before it
            drop(self.room.take());
            // SAFETY: the renderer that borrowed it is gone, the session too; the runtime goes after
            unsafe { SDL_DestroyGPUDevice(self.device) };
        }
    }
}

/// Elsewhere (Windows) VR doesn't start yet.
#[cfg(windows)]
pub use none::{Vr, VrFrame};

#[cfg(windows)]
mod none {
    use super::{EyeView, TableMeasures, VrOptions};
    use crate::math::Vec3;
    use crate::render::{FrameInput, Renderer};
    use sdl3_sys::everything::*;

    pub enum Vr {}
    pub enum VrFrame {}

    impl Vr {
        pub fn start(_settings: &crate::settings::Settings, _options: VrOptions, _measures: TableMeasures, _bounds: (Vec3, Vec3)) -> Result<Vr, String> {
            Err("VR plays on Linux only so far (the Steam Frame)".into())
        }
        pub fn device(&self) -> *mut SDL_GPUDevice {
            match *self {}
        }
        pub fn format(&self) -> SDL_GPUTextureFormat {
            match *self {}
        }
        pub fn passthrough(&self) -> bool {
            match *self {}
        }
        pub fn poll(&mut self) -> bool {
            match *self {}
        }
        pub fn request_exit(&mut self) {
            match *self {}
        }
        pub fn recentre(&mut self, _frame: &VrFrame) {
            match *self {}
        }
        pub fn set_menu(&mut self, _made: Option<&(crate::render::MenuPicture, crate::pause_menu::Layout)>, _screen: (u32, u32)) -> Result<(), String> {
            match *self {}
        }
        pub fn read_controls(&mut self, _frame: &VrFrame, _key: impl FnMut(u8, bool)) {
            match *self {}
        }
        pub fn begin_frame(&mut self) -> Result<Option<VrFrame>, String> {
            match *self {}
        }
        pub fn eyes(&self, _frame: &VrFrame) -> Option<[EyeView; 2]> {
            match *self {}
        }
        pub fn end_frame(&mut self, _frame: VrFrame, _renderer: &mut Renderer, _inputs: Option<[&FrameInput; 2]>) -> Result<(), String> {
            match *self {}
        }
    }
}
