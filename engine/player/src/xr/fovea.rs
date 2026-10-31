//! Foveated rendering in VR: full shading only around where each eye looks, coarser beyond, as the
//! owner's VPX fork does it on the Steam Frame (its "Foveated Rendering on the Steam Frame", §11): a
//! fragment shading rate image for each eye (the vendored SDL's fourth addition), its full rate disc
//! on the eye's gaze, 2x2 around it, 4x4 beyond. The gaze: `XR_EXT_eye_gaze_interaction`'s ray, its
//! fixation point 1 m along it smoothed over 35 ms (started afresh on a saccade), projected into each
//! eye; else the runtime's foveation centres (`XR_META_foveation_eye_tracked`); else each eye's straight
//! ahead, with the fork's wide fixed disc. The eye tracker is turned on by an eye-tracked foveation
//! profile at level NONE on the swapchains (`XR_FB_foveation`), which asks the runtime for nothing else.
//! Ours.

use std::time::Instant;

use super::place::{Fov, Pose};
pub use crate::render::Fovea;

/// `[PlayerVR]` `Foveation` (the fork's setting): Off, Low, Medium, High.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Off,
    Low,
    Medium,
    High,
}

impl Level {
    pub fn from_setting(v: i32) -> Level {
        match v {
            1 => Level::Low,
            2 => Level::Medium,
            3 => Level::High,
            _ => Level::Off,
        }
    }

    /// The full rate disc's radius and the 2x2 disc's around the gaze, as fractions of half the
    /// picture's width (the fork's eye-tracked rings).
    fn gaze_radii(self) -> (f32, f32) {
        match self {
            Level::Low => (0.30, 0.55),
            Level::Medium => (0.20, 0.38),
            Level::High => (0.15, 0.28),
            Level::Off => (2.0, 2.0),
        }
    }
}

/// Without a gaze: the fork's wide disc of its fixed Low profile, at each eye's straight ahead.
const FIXED_RADII: (f32, f32) = (0.45, 0.80);

/// Checking the gaze in the headset (`VPI_XR_FOVEA_DEBUG=1` or `[PlayerVR]` `FoveationDebug`, the fork's
/// `VPX_FOVEATION_DEBUG`): a small
/// sharp spot, about 3 degrees, coarse everywhere else, so it shows where the player believes the eye looks.
const DEBUG_RADII: (f32, f32) = (0.06, 0.09);

/// The gaze ray this frame, in the room: where it starts (when the runtime says; SteamVR gives the
/// direction only) and how it's turned (looking down its -z).
#[derive(Clone, Copy, Debug)]
pub struct GazeRay {
    pub position: Option<[f32; 3]>,
    pub orientation: [f32; 4],
}

/// The discs' placing from frame to frame.
pub struct Foveation {
    pub level: Level,
    /// Follow the eyes (`[PlayerVR]` `FoveationEyeTracked`) when the runtime can.
    pub eye_tracked: bool,
    /// The runtime's centres' axes mirrored (`FoveationFlipX`, `FoveationFlipY`).
    pub flip: [bool; 2],
    /// Test runs' discs (`VPI_XR_FOVEA=full,half`), fixed at each eye's straight ahead.
    pub test: Option<(f32, f32)>,
    /// The small spot of `VPI_XR_FOVEA_DEBUG`.
    pub debug: bool,
    point: Option<([f32; 3], Instant)>,
    /// What placed the discs last, for the log.
    pub source: &'static str,
}

impl Foveation {
    pub fn new(level: Level, eye_tracked: bool, flip: [bool; 2]) -> Foveation {
        let test = std::env::var("VPI_XR_FOVEA").ok().and_then(|v| {
            let (a, b) = v.split_once(',')?;
            Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
        });
        let debug = std::env::var_os("VPI_XR_FOVEA_DEBUG").is_some_and(|v| v == "1");
        Foveation { level, eye_tracked, flip, test, debug, point: None, source: "none yet" }
    }

    /// Each eye's disc this frame: on the gaze ray's fixation point, else the runtime's centres (each
    /// eye's, -1 to 1, x right, y down), else straight ahead; None when off.
    pub fn discs(&mut self, views: &[Pose; 2], fov: &[Fov; 2], ray: Option<GazeRay>, centres: Option<[[f32; 2]; 2]>) -> Option<Fovea> {
        let discs = self.placed(views, fov, ray, centres);
        if self.debug {
            return discs.map(|d| Fovea { full: DEBUG_RADII.0, half: DEBUG_RADII.1, ..d });
        }
        discs
    }

    fn placed(&mut self, views: &[Pose; 2], fov: &[Fov; 2], ray: Option<GazeRay>, centres: Option<[[f32; 2]; 2]>) -> Option<Fovea> {
        if self.level == Level::Off {
            return None;
        }
        if let Some((full, half)) = self.test {
            self.source = "fixed (test)";
            return Some(Fovea { centres: [straight_ahead(fov[0]), straight_ahead(fov[1])], full, half });
        }
        if self.eye_tracked {
            let (full, half) = self.level.gaze_radii();
            if let Some(ray) = ray {
                let point = self.fixation(views, ray);
                if let (Some(l), Some(r)) = (project(&views[0], fov[0], point), project(&views[1], fov[1], point)) {
                    self.source = "gaze";
                    return Some(Fovea { centres: [l, r], full, half });
                }
            }
            if let Some(c) = centres {
                self.source = "the runtime's centres";
                let axis = |v: f32, flip: bool| (if flip { -v } else { v } + 1.0) * 0.5;
                let centres = c.map(|[x, y]| [axis(x, self.flip[0]), axis(y, self.flip[1])]);
                return Some(Fovea { centres, full, half });
            }
        }
        self.point = None;
        self.source = "fixed";
        Some(Fovea { centres: [straight_ahead(fov[0]), straight_ahead(fov[1])], full: FIXED_RADII.0, half: FIXED_RADII.1 })
    }

    /// The fixation point: 1 m along the gaze ray (from between the eyes when the runtime gives no
    /// origin), smoothed over 35 ms, started afresh when it jumps more than 8 cm (a saccade).
    fn fixation(&mut self, views: &[Pose; 2], ray: GazeRay) -> [f32; 3] {
        let origin = ray.position.unwrap_or_else(|| Pose::middle(&views[0], &views[1]).position);
        let d = rotate(ray.orientation, [0.0, 0.0, -1.0]);
        let p = [0, 1, 2].map(|i| origin[i] + d[i]);
        let now = Instant::now();
        let point = match self.point {
            Some((q, at)) if distance(p, q) <= 0.08 => {
                let k = 1.0 - (-now.duration_since(at).as_secs_f32() / 0.035).exp();
                [0, 1, 2].map(|i| q[i] + (p[i] - q[i]) * k)
            }
            _ => p,
        };
        self.point = Some((point, now));
        point
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum::<f32>().sqrt()
}

/// `v` turned by the quaternion `q` (x, y, z, w).
fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [x, y, z, w] = q;
    let u = [x, y, z];
    let c1 = cross(u, v);
    let c2 = cross(u, c1);
    [0, 1, 2].map(|i| v[i] + 2.0 * (w * c1[i] + c2[i]))
}

/// Where a point of the room shows in an eye's picture, 0 to 1 across and down; None behind the eye.
fn project(eye: &Pose, fov: Fov, p: [f32; 3]) -> Option<[f32; 2]> {
    let [x, y, z, w] = eye.orientation;
    // into the eye's own space, looking down its -z, y up
    let q = rotate([-x, -y, -z, w], [0, 1, 2].map(|i| p[i] - eye.position[i]));
    if q[2] >= -1e-4 {
        return None;
    }
    let (tx, ty) = (q[0] / -q[2], q[1] / -q[2]);
    let (l, r, u, d) = (fov.left.tan(), fov.right.tan(), fov.up.tan(), fov.down.tan());
    Some([(tx - l) / (r - l), (u - ty) / (u - d)])
}

/// An eye's straight ahead in its picture (each eye's field of view is lopsided), 0 to 1 across and down.
fn straight_ahead(fov: Fov) -> [f32; 2] {
    let (l, r, u, d) = (fov.left.tan(), fov.right.tan(), fov.up.tan(), fov.down.tan());
    [-l / (r - l), u / (u - d)]
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOPSIDED: Fov = Fov { left: -0.85, right: 0.75, up: 0.85, down: -0.9 };

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5
    }

    #[test]
    fn straight_ahead_is_the_middle_of_an_even_view() {
        assert!(close(straight_ahead(Fov { left: -0.8, right: 0.8, up: 0.7, down: -0.7 }), [0.5, 0.5]));
        // more to the left than the right: straight ahead sits right of the middle
        assert!(straight_ahead(LOPSIDED)[0] > 0.5);
    }

    #[test]
    fn a_point_ahead_projects_straight_ahead() {
        let eye = Pose { position: [0.1, 1.5, 0.2], orientation: [0.0, 0.0, 0.0, 1.0] };
        let p = project(&eye, LOPSIDED, [0.1, 1.5, -0.8]).unwrap();
        assert!(close(p, straight_ahead(LOPSIDED)));
        // right and up of it: further right, higher
        let q = project(&eye, LOPSIDED, [0.3, 1.7, -0.8]).unwrap();
        assert!(q[0] > p[0] && q[1] < p[1]);
        // behind the eye: nowhere
        assert!(project(&eye, LOPSIDED, [0.1, 1.5, 1.0]).is_none());
    }

    #[test]
    fn a_quarter_turn_left_looks_down_minus_x() {
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let d = rotate([0.0, h, 0.0, h], [0.0, 0.0, -1.0]);
        assert!((d[0] + 1.0).abs() < 1e-5 && d[1].abs() < 1e-5 && d[2].abs() < 1e-5);
    }

    #[test]
    fn the_gaze_ray_places_both_discs_on_its_fixation_point() {
        let eyes = [
            Pose { position: [-0.032, 1.6, 0.0], orientation: [0.0, 0.0, 0.0, 1.0] },
            Pose { position: [0.032, 1.6, 0.0], orientation: [0.0, 0.0, 0.0, 1.0] },
        ];
        let fov = [LOPSIDED, Fov { left: -LOPSIDED.right, right: -LOPSIDED.left, ..LOPSIDED }];
        let mut f = Foveation { level: Level::Medium, eye_tracked: true, flip: [false; 2], test: None, debug: false, point: None, source: "" };
        let ray = GazeRay { position: None, orientation: [0.0, 0.0, 0.0, 1.0] };
        let discs = f.discs(&eyes, &fov, Some(ray), None).unwrap();
        assert_eq!(f.source, "gaze");
        // a point 1 m ahead of the middle: each eye sees it a little inward of its straight ahead
        assert!(discs.centres[0][0] > straight_ahead(fov[0])[0]);
        assert!(discs.centres[1][0] < straight_ahead(fov[1])[0]);
        assert_eq!((discs.full, discs.half), Level::Medium.gaze_radii());
        // no gaze, no centres: fixed, wide
        let fixed = f.discs(&eyes, &fov, None, None).unwrap();
        assert_eq!(f.source, "fixed");
        assert_eq!((fixed.full, fixed.half), FIXED_RADII);
    }
}
