//! The controllers, through OpenXR's actions: each pressed as the keyboard's keys are, VPX 5436's actions
//! by their keys, as the VPX fork maps the Steam Frame's controllers (`XRInputHandler`'s default mapping):
//! the triggers flip (the staged flipper past 60 %), the grips are the magna-saves, A starts, B adds a
//! credit, the right stick pulled back (or clicked) is the plunger, the bumpers nudge, the left stick
//! clicked recentres the table; the left view button opens the menu, which the d-pad moves through and A
//! picks in; the right menu button opens it with Quit picked. Bindings are offered for the Frame's
//! controllers (`XR_VALVE_frame_controller_interaction`), Touch, Index and the simple controller, each
//! kept where the profile has it. Ours.

use std::ffi::{CStr, CString, c_char};
use std::ptr::{null, null_mut};

use openxr_sys as xr;
use openxr_sys::Handle as _;

use super::{Runtime, VALVE_FRAME_CONTROLLER};
use crate::dik::default_keys as k;
use crate::pause_menu::code as menu;

/// What an action does.
#[derive(Clone, Copy)]
enum Does {
    /// Holds a key while pressed.
    Key(u8),
    /// Holds a key while a trigger is past `press`, until it's back under `release`.
    Trigger { key: u8, press: f32, release: f32 },
    /// Holds a key while a stick is pulled back past half way.
    Pull(u8),
    /// Recentres the table, when pressed.
    Recentre,
    /// Where a hand points (0 left, 1 right): the panel's pointer.
    Aim(usize),
}

/// The actions: name, the paths they're offered on (each kept where a profile has it), what they do.
const ACTIONS: &[(&str, &[&str], Does)] = &[
    ("left_flipper", &["/user/hand/left/input/trigger/value", "/user/hand/left/input/select/click"], Does::Trigger { key: k::LEFT_FLIPPER, press: 0.3, release: 0.25 }),
    ("right_flipper", &["/user/hand/right/input/trigger/value", "/user/hand/right/input/select/click"], Does::Trigger { key: k::RIGHT_FLIPPER, press: 0.3, release: 0.25 }),
    ("left_staged_flipper", &["/user/hand/left/input/trigger/value"], Does::Trigger { key: k::STAGED_LEFT_FLIPPER, press: 0.6, release: 0.55 }),
    ("right_staged_flipper", &["/user/hand/right/input/trigger/value"], Does::Trigger { key: k::STAGED_RIGHT_FLIPPER, press: 0.6, release: 0.55 }),
    ("left_magna_save", &["/user/hand/left/input/squeeze/click", "/user/hand/left/input/squeeze/value"], Does::Key(k::LEFT_MAGNA_SAVE)),
    ("right_magna_save", &["/user/hand/right/input/squeeze/click", "/user/hand/right/input/squeeze/value"], Does::Key(k::RIGHT_MAGNA_SAVE)),
    ("start", &["/user/hand/right/input/a/click"], Does::Key(k::START_GAME)),
    ("credit", &["/user/hand/right/input/b/click"], Does::Key(k::ADD_CREDIT)),
    ("launch", &["/user/hand/right/input/thumbstick/click"], Does::Key(k::PLUNGER)),
    ("plunger", &["/user/hand/right/input/thumbstick"], Does::Pull(k::PLUNGER)),
    ("nudge_left", &["/user/hand/left/input/bumper/click"], Does::Key(k::LEFT_TILT)),
    ("nudge_right", &["/user/hand/right/input/bumper/click"], Does::Key(k::RIGHT_TILT)),
    ("recentre", &["/user/hand/left/input/thumbstick/click"], Does::Recentre),
    ("menu", &["/user/hand/left/input/view/click", "/user/hand/left/input/menu/click", "/user/hand/left/input/x/click"], Does::Key(menu::IN_GAME_UI)),
    ("quit", &["/user/hand/right/input/menu/click", "/user/hand/left/input/y/click"], Does::Key(menu::EXIT)),
    ("menu_up", &["/user/hand/left/input/dpad_up/click"], Does::Key(menu::UP)),
    ("menu_down", &["/user/hand/left/input/dpad_down/click"], Does::Key(menu::DOWN)),
    ("menu_left", &["/user/hand/left/input/dpad_left/click"], Does::Key(menu::LEFT)),
    ("menu_right", &["/user/hand/left/input/dpad_right/click"], Does::Key(menu::RIGHT)),
    ("left_aim", &["/user/hand/left/input/aim/pose"], Does::Aim(0)),
    ("right_aim", &["/user/hand/right/input/aim/pose"], Does::Aim(1)),
];

/// What the hands do for the panel's pointer this frame: each trigger's pull (0 to 1, left then right)
/// and the right stick (x, y; up positive).
#[derive(Clone, Copy, Debug, Default)]
pub struct Pointing {
    pub trigger: [f32; 2],
    pub stick: [f32; 2],
}

/// The profiles bindings are offered for, the Frame's first.
const PROFILES: &[&str] = &[
    "/interaction_profiles/valve/frame_controller_valve",
    "/interaction_profiles/oculus/touch_controller",
    "/interaction_profiles/valve/index_controller",
    "/interaction_profiles/khr/simple_controller",
];

/// The controllers' actions, attached to the session.
pub struct Controls {
    set: xr::ActionSet,
    actions: Vec<(xr::Action, Does)>,
    /// Each trigger's state (past its press point), by action.
    triggered: Vec<bool>,
    /// The keys held now, by code.
    held: [bool; 256],
    recentre_held: bool,
    /// The gaze's pose action and its space (`XR_EXT_eye_gaze_interaction`), for foveated rendering.
    gaze: Option<(xr::Action, xr::Space)>,
    /// Each hand's aim pose's space, for the panel's pointer.
    aims: [Option<xr::Space>; 2],
    pub pointing: Pointing,
}

impl Controls {
    /// The action set, its actions and their bindings, attached to `session`.
    pub fn new(rt: &Runtime, session: xr::Session) -> Result<Controls, String> {
        // SAFETY: OpenXR's action calls, with NUL-terminated names and arrays of the counts given
        unsafe {
            let mut info: xr::ActionSetCreateInfo = std::mem::zeroed();
            info.ty = xr::StructureType::ACTION_SET_CREATE_INFO;
            copy_name(&mut info.action_set_name, "pinball");
            copy_name(&mut info.localized_action_set_name, "Pinball");
            let mut set = xr::ActionSet::NULL;
            super::check(rt, (rt.f.create_action_set)(rt.instance, &info, &mut set), "the controllers' actions")?;
            let mut controls =
                Controls { set, actions: Vec::new(), triggered: Vec::new(), held: [false; 256], recentre_held: false, gaze: None, aims: [None; 2], pointing: Pointing::default() };
            let mut offered: Vec<(xr::Action, xr::Path, &str)> = Vec::new();
            for (name, paths, does) in ACTIONS {
                let mut ai: xr::ActionCreateInfo = std::mem::zeroed();
                ai.ty = xr::StructureType::ACTION_CREATE_INFO;
                copy_name(&mut ai.action_name, name);
                copy_name(&mut ai.localized_action_name, &name.replace('_', " "));
                ai.action_type = match does {
                    Does::Trigger { .. } => xr::ActionType::FLOAT_INPUT,
                    Does::Pull(_) => xr::ActionType::VECTOR2F_INPUT,
                    Does::Aim(_) => xr::ActionType::POSE_INPUT,
                    _ => xr::ActionType::BOOLEAN_INPUT,
                };
                let mut action = xr::Action::NULL;
                super::check(rt, (rt.f.create_action)(set, &ai, &mut action), name)?;
                controls.actions.push((action, *does));
                for p in paths.iter() {
                    offered.push((action, path(rt, p)?, p));
                }
            }
            controls.triggered = vec![false; controls.actions.len()];
            // each profile takes the bindings it has: each tried alone (a suggestion with one path a
            // profile hasn't is refused whole), then those taken suggested together
            for profile in PROFILES {
                if profile.contains("frame_controller") && !rt.has(VALVE_FRAME_CONTROLLER) {
                    continue;
                }
                let profile_path = path(rt, profile)?;
                let (taken, refused): (Vec<_>, Vec<_>) =
                    offered.iter().partition(|&&(action, binding, _)| suggest(rt, profile_path, &[xr::ActionSuggestedBinding { action, binding }]) == xr::Result::SUCCESS);
                if taken.is_empty() {
                    continue;
                }
                let taken: Vec<xr::ActionSuggestedBinding> = taken.iter().map(|&&(action, binding, _)| xr::ActionSuggestedBinding { action, binding }).collect();
                let r = suggest(rt, profile_path, &taken);
                println!(
                    "VR controls: {} of {} bindings for {profile}{}; the profile has no {}",
                    taken.len(),
                    offered.len(),
                    if r == xr::Result::SUCCESS { String::new() } else { format!(" (refused together: {})", rt.result(r)) },
                    refused.iter().map(|(_, _, p)| *p).collect::<Vec<_>>().join(", ")
                );
            }
            // the gaze, for foveated rendering: a pose, on the eye tracker's own profile
            let mut gaze = xr::Action::NULL;
            if rt.eye_gaze {
                let mut ai: xr::ActionCreateInfo = std::mem::zeroed();
                ai.ty = xr::StructureType::ACTION_CREATE_INFO;
                copy_name(&mut ai.action_name, "eye_gaze");
                copy_name(&mut ai.localized_action_name, "eye gaze");
                ai.action_type = xr::ActionType::POSE_INPUT;
                if (rt.f.create_action)(set, &ai, &mut gaze) == xr::Result::SUCCESS {
                    let binding = xr::ActionSuggestedBinding { action: gaze, binding: path(rt, "/user/eyes_ext/input/gaze_ext/pose")? };
                    let r = suggest(rt, path(rt, "/interaction_profiles/ext/eye_gaze_interaction")?, &[binding]);
                    if r != xr::Result::SUCCESS {
                        println!("VR controls: the gaze's binding was refused ({})", rt.result(r));
                        gaze = xr::Action::NULL;
                    }
                } else {
                    gaze = xr::Action::NULL;
                }
            }
            let attach = xr::SessionActionSetsAttachInfo { ty: xr::StructureType::SESSION_ACTION_SETS_ATTACH_INFO, next: null(), count_action_sets: 1, action_sets: &set };
            super::check(rt, (rt.f.attach_session_action_sets)(session, &attach), "attaching the controllers' actions")?;
            if gaze != xr::Action::NULL {
                let info = xr::ActionSpaceCreateInfo {
                    ty: xr::StructureType::ACTION_SPACE_CREATE_INFO,
                    next: null(),
                    action: gaze,
                    subaction_path: xr::Path::NULL,
                    pose_in_action_space: xr::Posef { orientation: xr::Quaternionf { x: 0.0, y: 0.0, z: 0.0, w: 1.0 }, position: xr::Vector3f { x: 0.0, y: 0.0, z: 0.0 } },
                };
                let mut space = xr::Space::NULL;
                if (rt.f.create_action_space)(session, &info, &mut space) == xr::Result::SUCCESS {
                    controls.gaze = Some((gaze, space));
                    println!("VR controls: the gaze, for foveated rendering");
                }
            }
            // where each hand points, for the panel
            for &(action, does) in &controls.actions {
                let Does::Aim(hand) = does else { continue };
                let info = xr::ActionSpaceCreateInfo {
                    ty: xr::StructureType::ACTION_SPACE_CREATE_INFO,
                    next: null(),
                    action,
                    subaction_path: xr::Path::NULL,
                    pose_in_action_space: xr::Posef { orientation: xr::Quaternionf { x: 0.0, y: 0.0, z: 0.0, w: 1.0 }, position: xr::Vector3f { x: 0.0, y: 0.0, z: 0.0 } },
                };
                let mut space = xr::Space::NULL;
                if (rt.f.create_action_space)(session, &info, &mut space) == xr::Result::SUCCESS {
                    controls.aims[hand] = Some(space);
                }
            }
            Ok(controls)
        }
    }

    /// Reads the controllers (while the session has them: focused), handing each key pressed or let
    /// go to `key`, and whether the recentre button was just pressed.
    pub fn sync(&mut self, rt: &Runtime, session: xr::Session, focused: bool, mut key: impl FnMut(u8, bool)) -> bool {
        let mut want = [false; 256];
        let mut recentre = false;
        if focused {
            // SAFETY: our attached action set and its actions
            unsafe {
                let active = xr::ActiveActionSet { action_set: self.set, subaction_path: xr::Path::NULL };
                let sync = xr::ActionsSyncInfo { ty: xr::StructureType::ACTIONS_SYNC_INFO, next: null(), count_active_action_sets: 1, active_action_sets: &active };
                if (rt.f.sync_actions)(session, &sync) == xr::Result::SUCCESS {
                    for (i, &(action, does)) in self.actions.iter().enumerate() {
                        let get = xr::ActionStateGetInfo { ty: xr::StructureType::ACTION_STATE_GET_INFO, next: null(), action, subaction_path: xr::Path::NULL };
                        match does {
                            Does::Key(code) => {
                                let mut s: xr::ActionStateBoolean = std::mem::zeroed();
                                s.ty = xr::StructureType::ACTION_STATE_BOOLEAN;
                                if (rt.f.get_action_state_boolean)(session, &get, &mut s) == xr::Result::SUCCESS && s.is_active != xr::FALSE && s.current_state != xr::FALSE {
                                    want[code as usize] = true;
                                }
                            }
                            Does::Trigger { key: code, press, release } => {
                                let mut s: xr::ActionStateFloat = std::mem::zeroed();
                                s.ty = xr::StructureType::ACTION_STATE_FLOAT;
                                let value = if (rt.f.get_action_state_float)(session, &get, &mut s) == xr::Result::SUCCESS && s.is_active != xr::FALSE { s.current_state } else { 0.0 };
                                if code == k::LEFT_FLIPPER {
                                    self.pointing.trigger[0] = value;
                                } else if code == k::RIGHT_FLIPPER {
                                    self.pointing.trigger[1] = value;
                                }
                                self.triggered[i] = if self.triggered[i] { value >= release } else { value >= press };
                                want[code as usize] |= self.triggered[i];
                            }
                            Does::Pull(code) => {
                                let mut s: xr::ActionStateVector2f = std::mem::zeroed();
                                s.ty = xr::StructureType::ACTION_STATE_VECTOR2F;
                                let (x, y) = if (rt.f.get_action_state_vector2f)(session, &get, &mut s) == xr::Result::SUCCESS && s.is_active != xr::FALSE {
                                    (s.current_state.x, s.current_state.y)
                                } else {
                                    (0.0, 0.0)
                                };
                                if code == k::PLUNGER {
                                    self.pointing.stick = [x, y];
                                }
                                self.triggered[i] = if self.triggered[i] { y < -0.3 } else { y < -0.5 };
                                want[code as usize] |= self.triggered[i];
                            }
                            Does::Recentre => {
                                let mut s: xr::ActionStateBoolean = std::mem::zeroed();
                                s.ty = xr::StructureType::ACTION_STATE_BOOLEAN;
                                recentre = (rt.f.get_action_state_boolean)(session, &get, &mut s) == xr::Result::SUCCESS && s.is_active != xr::FALSE && s.current_state != xr::FALSE;
                            }
                            // located when asked (`aim`)
                            Does::Aim(_) => {}
                        }
                    }
                }
            }
        }
        // what changed, as key presses; everything let go when the session loses the controllers
        for code in 0..256 {
            if want[code] != self.held[code] {
                self.held[code] = want[code];
                key(code as u8, want[code]);
            }
        }
        let pressed = recentre && !self.recentre_held;
        self.recentre_held = recentre;
        pressed
    }

    /// The profile each hand's controller is in now, for the log.
    /// The gaze ray at `time` in the room's space `room`, when the eye tracker gives it (its direction at
    /// least: SteamVR gives no position).
    pub fn gaze(&self, rt: &Runtime, room: xr::Space, time: xr::Time) -> Option<super::fovea::GazeRay> {
        let (_, space) = self.gaze?;
        // SAFETY: our spaces, on the session they were made for
        unsafe {
            let mut at: xr::SpaceLocation = std::mem::zeroed();
            at.ty = xr::StructureType::SPACE_LOCATION;
            if (rt.f.locate_space)(space, room, time, &mut at) != xr::Result::SUCCESS || !at.location_flags.contains(xr::SpaceLocationFlags::ORIENTATION_VALID) {
                return None;
            }
            let (o, p) = (at.pose.orientation, at.pose.position);
            Some(super::fovea::GazeRay {
                position: at.location_flags.contains(xr::SpaceLocationFlags::POSITION_VALID).then_some([p.x, p.y, p.z]),
                orientation: [o.x, o.y, o.z, o.w],
            })
        }
    }

    /// Where `hand` (0 left, 1 right) points at `time`, in the room's space `room`: its aim pose, whose
    /// -z axis is the ray.
    pub fn aim(&self, rt: &Runtime, room: xr::Space, time: xr::Time, hand: usize) -> Option<super::place::Pose> {
        let space = self.aims.get(hand).copied().flatten()?;
        // SAFETY: our spaces, on the session they were made for
        unsafe {
            let mut at: xr::SpaceLocation = std::mem::zeroed();
            at.ty = xr::StructureType::SPACE_LOCATION;
            let valid = xr::SpaceLocationFlags::ORIENTATION_VALID | xr::SpaceLocationFlags::POSITION_VALID;
            if (rt.f.locate_space)(space, room, time, &mut at) != xr::Result::SUCCESS || !at.location_flags.contains(valid) {
                return None;
            }
            Some(super::place::Pose::from_xr(&at.pose))
        }
    }

    pub fn profiles(&self, rt: &Runtime, session: xr::Session) -> String {
        ["/user/hand/left", "/user/hand/right"]
            .iter()
            .map(|hand| {
                // SAFETY: OpenXR's queries, with buffers of the sizes given
                unsafe {
                    let Ok(top) = path(rt, hand) else { return format!("{hand}: ?") };
                    let mut state = xr::InteractionProfileState { ty: xr::StructureType::INTERACTION_PROFILE_STATE, next: null_mut(), interaction_profile: xr::Path::NULL };
                    if (rt.f.get_current_interaction_profile)(session, top, &mut state) != xr::Result::SUCCESS || state.interaction_profile == xr::Path::NULL {
                        return format!("{hand}: none");
                    }
                    let mut buf = [0 as c_char; 256];
                    let mut n = 0;
                    (rt.f.path_to_string)(rt.instance, state.interaction_profile, buf.len() as u32, &mut n, buf.as_mut_ptr());
                    format!("{hand}: {}", CStr::from_ptr(buf.as_ptr()).to_string_lossy())
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn destroy(&mut self, rt: &Runtime) {
        if let Some((_, space)) = self.gaze.take() {
            // SAFETY: our space, destroyed once, before its action
            unsafe { (rt.f.destroy_space)(space) };
        }
        for space in self.aims.iter_mut().filter_map(Option::take) {
            // SAFETY: our space, destroyed once, before its action
            unsafe { (rt.f.destroy_space)(space) };
        }
        if self.set != xr::ActionSet::NULL {
            // SAFETY: our action set, destroyed once (its actions with it)
            unsafe { (rt.f.destroy_action_set)(self.set) };
            self.set = xr::ActionSet::NULL;
        }
    }
}

fn copy_name(dst: &mut [c_char], name: &str) {
    for (d, s) in dst.iter_mut().zip(name.bytes().chain(std::iter::once(0))) {
        *d = s as c_char;
    }
}

fn path(rt: &Runtime, p: &str) -> Result<xr::Path, String> {
    let c = CString::new(p).unwrap_or_default();
    let mut out = xr::Path::NULL;
    // SAFETY: a NUL-terminated path string
    let r = unsafe { (rt.f.string_to_path)(rt.instance, c.as_ptr(), &mut out) };
    if r == xr::Result::SUCCESS { Ok(out) } else { Err(format!("OpenXR's path {p}: {}", rt.result(r))) }
}

fn suggest(rt: &Runtime, profile: xr::Path, bindings: &[xr::ActionSuggestedBinding]) -> xr::Result {
    let s = xr::InteractionProfileSuggestedBinding {
        ty: xr::StructureType::INTERACTION_PROFILE_SUGGESTED_BINDING,
        next: null(),
        interaction_profile: profile,
        count_suggested_bindings: bindings.len() as u32,
        suggested_bindings: bindings.as_ptr(),
    };
    // SAFETY: the bindings' array, of the count given
    unsafe { (rt.f.suggest_interaction_profile_bindings)(rt.instance, &s) }
}
