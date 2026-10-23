use bevy_ecs::component::Component;
use bevy_math::{Vec2, Vec3};
use bevy_transform::components::Transform;

use super::corridor_scene::Geometry;

#[derive(Component, Clone, Debug)]
pub(super) struct Orbit {
    pub focus: Vec3,
    pub distance: f32,
    pub yaw: f32,
    pub elevation: f32,
    extent: f32,
}

impl Orbit {
    pub fn new(geometry: &Geometry) -> Self {
        let extent = (geometry.max - geometry.min).max_element().max(10.0);
        Self {
            focus: (geometry.min + geometry.max) * 0.5,
            distance: extent * 0.9,
            yaw: 0.0,
            elevation: 1.03,
            extent,
        }
    }

    // 鼠标量已经是帧内累计位移，不能再乘 delta_time。
    pub fn apply(&mut self, pan: Vec2, rotate: Vec2, scroll_lines: f32) {
        if !pan.is_finite() || !rotate.is_finite() || !scroll_lines.is_finite() {
            return;
        }
        self.yaw = (self.yaw - rotate.x * 0.004).rem_euclid(std::f32::consts::TAU);
        self.elevation = (self.elevation + rotate.y * 0.004).clamp(0.18, 1.48);
        self.distance =
            (self.distance * (-scroll_lines * 0.12).exp()).clamp(12.0, self.extent * 4.0);
        let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        let forward = Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos());
        self.focus += (-pan.x * right + pan.y * forward) * (self.distance * 0.0012);
        self.focus = self.focus.clamp(
            Vec3::splat(-self.extent * 4.0),
            Vec3::splat(self.extent * 4.0),
        );
        self.focus.y = 0.0;
    }

    pub fn transform(&self) -> Transform {
        let planar = self.distance * self.elevation.cos();
        Transform::from_translation(
            self.focus
                + Vec3::new(
                    planar * self.yaw.sin(),
                    self.distance * self.elevation.sin(),
                    planar * self.yaw.cos(),
                ),
        )
        .looking_at(self.focus, Vec3::Y)
    }
}
