// Ported from Visual Pinball X: src/renderer/VRDevice.cpp, VRDevice::RenderFrame's table placement (the
// playfield's space reference) and its eye matrices, XrPosef_ToMatrix3D, and Matrix3D::SetPerspectiveFovRH
// (src/math/matrix.h) (license:GPLv3+), as SPD13/vpinball-steam-frame has them at bb04b2a41 (10.8.1).
// Copyright (C) 2000-2026 Visual Pinball development team and contributors.
// This port is also GPL-3.0-or-later.

//! Where the table stands in the room in VR, as VPX places it: scaled so its lockbar is as wide as the
//! player's (`[Player]` `LockbarWidth`), the lockbar's top at the player's height for it (`LockbarHeight`),
//! the playfield at its slope, turned and moved by `[PlayerVR]` `Orientation`, `TableX`, `TableY` and
//! `TableZ`; and each eye's view and projection of it, in the renderer's convention (VPX's: row vectors,
//! VPU). The room is OpenXR's: metres, Y up, the floor at 0 in a stage space. Recentring sets the
//! orientation and offsets from where the head is, as VPX's "headset view centering" does.

use crate::camera::{cm_to_vpu, vpu_to_cm};
use crate::math::{Mat4, Vec3};
use crate::settings::Settings;

/// VPU to metres (VPX's `vpuToWorldScale`).
pub const VPU_TO_METRES: f32 = (0.0254 * 1.0625 / 50.0) as f32;
/// The near plane, 10 cm before the eyes.
pub const NEAR_METRES: f32 = 0.1;
/// The playfield's bottom edge is 5 cm from the lockbar (VPX's fixed `lockbarToPlayfield`).
const LOCKBAR_TO_PLAYFIELD_CM: f32 = 5.0;

pub use crate::vr::TableMeasures;

/// Where the player puts the table: VPX's `[Player]` and `[PlayerVR]` settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// The lockbar's width and height above the floor, cm.
    pub lockbar_width_cm: f32,
    pub lockbar_height_cm: f32,
    /// The table turned about the vertical, degrees, and moved, cm (VPX's `m_orientation`, `m_tablePos`).
    pub orientation_deg: f32,
    pub table_cm: [f32; 3],
}

impl Placement {
    /// From the settings, with VPX's ranges and defaults (`Settings_properties.inl`).
    pub fn from_settings(s: &Settings) -> Placement {
        Placement {
            lockbar_width_cm: s.float_or("Player", "LockbarWidth", 70.0, 10.0, 150.0, 0.0),
            lockbar_height_cm: s.float_or("Player", "LockbarHeight", 85.0, 0.0, 250.0, 0.0),
            orientation_deg: s.float_or("PlayerVR", "Orientation", 0.0, -180.0, 180.0, 0.0),
            table_cm: [
                s.float_or("PlayerVR", "TableX", 0.0, -100.0, 100.0, 0.0),
                s.float_or("PlayerVR", "TableY", 0.0, -100.0, 100.0, 0.0),
                s.float_or("PlayerVR", "TableZ", 0.0, -100.0, 100.0, 0.0),
            ],
        }
    }

    /// The table's scale in the room: its lockbar as wide as the player's, the lockbar being the
    /// playfield's width and 2¼" (VPX's `m_scale`, held to 0.1-2).
    pub fn scale(&self, t: &TableMeasures) -> f32 {
        let table_width_cm = vpu_to_cm(t.right - t.left) + 2.25 * 2.54;
        (self.lockbar_width_cm / table_width_cm).clamp(0.1, 2.0)
    }

    /// The playfield's own coordinates (VPU) into the room's, still in VPU (VPX's `m_pfWorld.m_toWorld`).
    pub fn playfield_to_room(&self, t: &TableMeasures) -> Mat4 {
        let scale = self.scale(t);
        let scene_scale = Mat4::scale(scale, scale, scale);
        // VPX's left-handed table into OpenXR's right-handed room, the table's Z up becoming Y up
        let coords = Mat4::scale(1.0, -1.0, 1.0);
        let view_orientation = Mat4::rotate_z(self.orientation_deg.to_radians()) * Mat4::rotate_x((-90.0f32).to_radians());
        let scaled_glass_height_cm = scale * vpu_to_cm(t.glass_bottom);
        let playfield_slope = Mat4::rotate_x(t.slope_deg.to_radians());
        let table_coords = Mat4::translate(-scale * (t.right - t.left) * 0.5, scale * (t.bottom - t.top), 0.0);
        let [x, y, z] = self.table_cm;
        let playfield_pos =
            Mat4::translate(-cm_to_vpu(x), cm_to_vpu(y + LOCKBAR_TO_PLAYFIELD_CM), cm_to_vpu(z + self.lockbar_height_cm - scaled_glass_height_cm));
        scene_scale * coords * table_coords * playfield_slope * playfield_pos * view_orientation
    }

    /// VPX's headset view centering: the table turned to face where the head looks and moved to where
    /// it stands (`[Player]` `ScreenPlayerX`/`Y` from the lockbar, as VPX adds them), from the middle of
    /// the eyes' poses in the room.
    pub fn recentred(&self, eyes_middle: Pose, player_cm: [f32; 2]) -> Placement {
        // the view of that middle pose, in VPU (VPX's `medianView`)
        let pos = eyes_middle.position;
        let middle = Pose { position: [pos[0] / VPU_TO_METRES, pos[1] / VPU_TO_METRES, pos[2] / VPU_TO_METRES], orientation: eyes_middle.orientation };
        let view = middle.view_matrix();
        let angle = view.m[0][2].atan2(view.m[0][0]);
        let (s, c) = angle.sin_cos();
        let dx = -vpu_to_cm(middle.position[0]);
        let dy = -vpu_to_cm(middle.position[2]);
        Placement {
            orientation_deg: angle.to_degrees(),
            table_cm: [dx * c - dy * s + player_cm[0], dx * s + dy * c + player_cm[1], 0.0],
            ..*self
        }
    }
}

/// An OpenXR pose: where something is in the room (metres) and how it's turned (x, y, z, w).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub position: [f32; 3],
    pub orientation: [f32; 4],
}

impl Pose {
    pub fn from_xr(p: &openxr_sys::Posef) -> Pose {
        Pose { position: [p.position.x, p.position.y, p.position.z], orientation: [p.orientation.x, p.orientation.y, p.orientation.z, p.orientation.w] }
    }

    /// The pose as OpenXR's.
    pub fn to_xr(&self) -> openxr_sys::Posef {
        let ([x, y, z, w], [px, py, pz]) = (self.orientation, self.position);
        openxr_sys::Posef { orientation: openxr_sys::Quaternionf { x, y, z, w }, position: openxr_sys::Vector3f { x: px, y: py, z: pz } }
    }

    /// The view from this pose: the room into the eye's own space, the pose undone (VPX's
    /// `XrPosef_ToMatrix3D`), in the pose's units.
    pub fn view_matrix(&self) -> Mat4 {
        let [x, y, z, w] = self.orientation;
        // the turn undone: the conjugate's rotation, which in the row-vector convention is the
        // quaternion's own matrix transposed (bx::mtxFromQuaternion, then Transpose)
        let mut m = Mat4::from_quaternion([-x, -y, -z, w]);
        // then the position undone: -(p) turned by the inverse rotation
        let p = Vec3::new(-self.position[0], -self.position[1], -self.position[2]);
        let r = m;
        m.m[3][0] = p.x * r.m[0][0] + p.y * r.m[1][0] + p.z * r.m[2][0];
        m.m[3][1] = p.x * r.m[0][1] + p.y * r.m[1][1] + p.z * r.m[2][1];
        m.m[3][2] = p.x * r.m[0][2] + p.y * r.m[1][2] + p.z * r.m[2][2];
        m
    }

    /// The middle of two poses (VPX lerps the positions and normalises the lerped quaternions).
    pub fn middle(a: &Pose, b: &Pose) -> Pose {
        let position = [0, 1, 2].map(|i| (a.position[i] + b.position[i]) * 0.5);
        let mut q = [0, 1, 2, 3].map(|i| (a.orientation[i] + b.orientation[i]) * 0.5);
        let len = q.iter().map(|v| v * v).sum::<f32>().sqrt().max(f32::MIN_POSITIVE);
        q.iter_mut().for_each(|v| *v /= len);
        Pose { position, orientation: q }
    }
}

/// An eye's field of view, the four angles (radians) OpenXR gives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fov {
    pub left: f32,
    pub right: f32,
    pub up: f32,
    pub down: f32,
}

/// VPX's `SetPerspectiveFovRH` with finite planes (`SetPerspectiveOffCenterRH`): depth 0 at the near
/// plane, 1 at the far one.
pub fn projection(fov: Fov, near: f32, far: f32) -> Mat4 {
    let (l, r, b, t) = (fov.left.tan() * near, fov.right.tan() * near, fov.down.tan() * near, fov.up.tan() * near);
    let mut m = Mat4 { m: [[0.0; 4]; 4] };
    m.m[0][0] = 2.0 * near / (r - l);
    m.m[1][1] = 2.0 * near / (t - b);
    m.m[2][0] = (r + l) / (r - l);
    m.m[2][1] = (t + b) / (t - b);
    m.m[2][2] = far / (near - far);
    m.m[2][3] = -1.0;
    m.m[3][2] = near * m.m[2][2];
    m
}

/// One eye's matrices for the renderer: its view of the playfield (VPX's `m_pfWorld.m_view[eye]`,
/// orthonormal, for the shading), its projection (`m_sceneProj[eye]`, the table's scale and VPU to metres
/// folded in), and the eye's place in the playfield's coordinates.
#[derive(Clone, Copy, Debug)]
pub struct EyeMatrices {
    pub view: Mat4,
    pub projection: Mat4,
    pub eye: Vec3,
}

/// An eye's matrices, from its pose and field of view in the room, the playfield placed by
/// `to_room` (`Placement::playfield_to_room`) at `scale`; `far` in metres.
pub fn eye_matrices(pose: &Pose, fov: Fov, to_room: &Mat4, scale: f32, far: f32) -> EyeMatrices {
    let vpu = Mat4::scale(VPU_TO_METRES, VPU_TO_METRES, VPU_TO_METRES);
    let inv_vpu = Mat4::scale(1.0 / VPU_TO_METRES, 1.0 / VPU_TO_METRES, 1.0 / VPU_TO_METRES);
    // the pose's view, in VPU
    let view = vpu * pose.view_matrix() * inv_vpu;
    let inv_scene_scale = Mat4::scale(1.0 / scale, 1.0 / scale, 1.0 / scale);
    let scene_scale = Mat4::scale(scale, scale, scale);
    let pf_view = *to_room * view * inv_scene_scale;
    let projection = scene_scale * vpu * projection(fov, NEAR_METRES, far);
    let eye = pf_view.inverse().position();
    EyeMatrices { view: pf_view, projection, eye }
}

/// The far plane: the scene's size, at least 5 m (VPX's `max(5, m_sceneSize * vpuToWorldScale)`), the
/// scene's size being its bounding box's diagonal in VPU.
pub fn far_plane(bounds_min: Vec3, bounds_max: Vec3) -> f32 {
    let d = Vec3::new(bounds_max.x - bounds_min.x, bounds_max.y - bounds_min.y, bounds_max.z - bounds_min.z);
    ((d.x * d.x + d.y * d.y + d.z * d.z).sqrt() * VPU_TO_METRES).max(5.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A standard 20.25" x 42" playfield (VPX's 952 x 2162 VPU table, 6.5 degrees).
    fn table() -> TableMeasures {
        TableMeasures { left: 0.0, top: 0.0, right: 952.0, bottom: 2162.0, glass_bottom: 210.0, slope_deg: 6.5 }
    }

    fn placement() -> Placement {
        Placement { lockbar_width_cm: 70.0, lockbar_height_cm: 85.0, orientation_deg: 0.0, table_cm: [0.0, 0.0, 0.0] }
    }

    fn room_metres(m: &Mat4, p: Vec3) -> Vec3 {
        let r = m.transform(p);
        Vec3::new(r.x * VPU_TO_METRES, r.y * VPU_TO_METRES, r.z * VPU_TO_METRES)
    }

    #[test]
    fn the_lockbar_is_as_wide_as_the_players() {
        let t = table();
        let s = placement().scale(&t);
        let width_cm = vpu_to_cm(952.0) * s + 2.25 * 2.54 * s;
        assert!((width_cm - 70.0).abs() < 0.01, "{width_cm}");
    }

    #[test]
    fn the_playfield_stands_before_the_player_at_lockbar_height() {
        let t = table();
        let p = placement();
        let m = p.playfield_to_room(&t);
        // the playfield's bottom middle: just behind the lockbar, its top below the lockbar by the glass
        let bottom = room_metres(&m, Vec3::new(476.0, 2162.0, 0.0));
        assert!(bottom.x.abs() < 1e-4, "centred: {bottom:?}");
        let glass_m = vpu_to_cm(210.0) * p.scale(&t) / 100.0;
        assert!((bottom.y - (0.85 - glass_m)).abs() < 0.002, "height {bottom:?}");
        assert!((bottom.z + 0.05).abs() < 0.002, "5 cm ahead (-Z): {bottom:?}");
        // the far end is further away (-Z) and higher (the slope)
        let top = room_metres(&m, Vec3::new(476.0, 0.0, 0.0));
        assert!(top.z < bottom.z - 0.8, "{top:?}");
        assert!(top.y > bottom.y, "{top:?}");
        // the playfield's left edge is on the player's left (-X)
        let left = room_metres(&m, Vec3::new(0.0, 2162.0, 0.0));
        assert!(left.x < -0.2, "{left:?}");
        // the playfield's up (+Z in VPX, towards the glass) is up in the room
        let above = room_metres(&m, Vec3::new(476.0, 2162.0, 100.0));
        assert!(above.y > bottom.y, "{above:?}");
    }

    #[test]
    fn an_eye_above_the_lockbar_sees_the_playfield_ahead() {
        let t = table();
        let p = placement();
        let to_room = p.playfield_to_room(&t);
        let pose = Pose { position: [0.0, 1.2, 0.3], orientation: [0.0, 0.0, 0.0, 1.0] };
        let fov = Fov { left: -0.8, right: 0.8, up: 0.8, down: -0.8 };
        let e = eye_matrices(&pose, fov, &to_room, p.scale(&t), 5.0);
        let clip = |v: Vec3| (e.view * e.projection).transform(v);
        // the playfield's middle is in view, below the middle of the picture (the eye looks ahead)
        let mid = clip(Vec3::new(476.0, 1081.0, 0.0));
        assert!(mid.x.abs() < 0.05 && mid.y < 0.0 && mid.y > -1.0 && mid.z > 0.0 && mid.z < 1.0, "{mid:?}");
        // the left of the playfield is on the left of the picture
        let left = clip(Vec3::new(0.0, 1081.0, 0.0));
        assert!(left.x < mid.x, "{left:?} {mid:?}");
        // the eye, in the playfield's coordinates, is above it (+Z) and before its bottom (+Y)
        assert!(e.eye.z > 0.0 && e.eye.y > 2162.0, "{:?}", e.eye);
    }

    #[test]
    fn a_pose_turned_left_sees_the_left() {
        // turned 90 degrees to the left about Y: the -X axis of the room is ahead (-Z of the eye)
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let pose = Pose { position: [0.0, 0.0, 0.0], orientation: [0.0, h, 0.0, h] };
        let v = pose.view_matrix();
        let ahead = v.transform(Vec3::new(-1.0, 0.0, 0.0));
        assert!((ahead.z + 1.0).abs() < 1e-5 && ahead.x.abs() < 1e-5, "{ahead:?}");
        // and a moved pose sees its own position at the origin
        let moved = Pose { position: [1.0, 2.0, 3.0], orientation: [0.0, h, 0.0, h] };
        let o = moved.view_matrix().transform(Vec3::new(1.0, 2.0, 3.0));
        assert!(o.x.abs() < 1e-5 && o.y.abs() < 1e-5 && o.z.abs() < 1e-5, "{o:?}");
    }

    #[test]
    fn recentring_faces_the_table_where_the_head_looks() {
        let p = placement();
        // the head 1 m to the right, turned 90 degrees to the right (looking down +X... -Z turned to +X)
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let head = Pose { position: [1.0, 1.6, 0.0], orientation: [0.0, -h, 0.0, h] };
        let r = p.recentred(head, [0.0, 0.0]);
        let to_room = r.playfield_to_room(&table());
        // the playfield's bottom middle is now ahead of the head: to its right in the room (+X)
        let bottom = room_metres(&to_room, Vec3::new(476.0, 2162.0, 0.0));
        assert!(bottom.x > 1.0 && bottom.z.abs() < 0.1, "{bottom:?}");
    }
}
