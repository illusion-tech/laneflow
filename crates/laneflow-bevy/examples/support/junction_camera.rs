//! 相机预设、天空大气与光照。
use bevy::{
    camera::{Exposure, ScalingMode},
    light::{Atmosphere, GlobalAmbientLight, atmosphere::ScatteringMedium},
    pbr::{AtmosphereSettings, DistanceFog, FogFalloff},
    prelude::*,
};

use super::junction_cli::CameraPreset;

#[derive(Resource)]
pub(super) struct CameraPresetChoice(pub CameraPreset);

const SKY_LIGHT: Color = Color::srgb(0.85, 0.9, 1.0);
const AMBIENT_BRIGHTNESS: f32 = 2500.0;

/// 天空大气（地球散射介质 + 相机设置）、太阳（平行光，投影，约 40° 俯角）与按
/// 预设放置的相机；`view_height` 是正交俯视的画面高度（米）。
pub(super) fn spawn(
    commands: &mut Commands,
    scattering_mediums: &mut Assets<ScatteringMedium>,
    preset: CameraPreset,
    view_height: f32,
) {
    let medium = scattering_mediums.add(ScatteringMedium::earth(256, 256));
    commands.spawn(Atmosphere::earth(medium));
    match preset {
        CameraPreset::Persp => {
            commands.spawn((
                Camera3d::default(),
                Exposure { ev100: 13.0 },
                // fov 62° + ~26° 俯角：画面上方露出地平线/天空，路口核心占主体。
                Projection::Perspective(PerspectiveProjection {
                    fov: 1.0821,
                    ..default()
                }),
                AtmosphereSettings::default(),
                // 0.19 起 AmbientLight / DistanceFog 是相机组件；全局环境光用
                // GlobalAmbientLight 资源做弱补光。
                ambient_light(),
                fog(),
                Transform::from_xyz(55.0, 38.0, 55.0).looking_at(Vec3::ZERO, Vec3::Y),
            ));
        }
        CameraPreset::Topdown => {
            // 正交垂直向下：按当前环路范围覆盖全场景，方像素便于量几何。
            commands.spawn((
                Camera3d::default(),
                Exposure { ev100: 13.0 },
                Projection::Orthographic(OrthographicProjection {
                    scaling_mode: ScalingMode::Fixed {
                        width: view_height * 1.6,
                        height: view_height,
                    },
                    ..OrthographicProjection::default_3d()
                }),
                Transform::from_xyz(0.0, 300.0, 0.0).looking_at(Vec3::ZERO, Vec3::Z),
            ));
        }
        CameraPreset::Close
        | CameraPreset::Merge
        | CameraPreset::Waiting
        | CameraPreset::Signals => {
            // 低位近景：看路口东南一角，读标线线型与停止线。
            commands.spawn((
                Camera3d::default(),
                Exposure { ev100: 13.0 },
                AtmosphereSettings::default(),
                ambient_light(),
                fog(),
                match preset {
                    CameraPreset::Merge => Transform::from_xyz(58.0, 70.0, -160.0)
                        .looking_at(Vec3::new(0.0, 0.0, -176.0), Vec3::Y),
                    CameraPreset::Waiting => Transform::from_xyz(-48.0, 19.0, 14.0)
                        .looking_at(Vec3::new(-4.0, 0.0, 0.0), Vec3::Y),
                    CameraPreset::Signals => Transform::from_xyz(9.0, 8.0, 17.0)
                        .looking_at(Vec3::new(34.5, 7.25, 6.0), Vec3::Y),
                    _ => Transform::from_xyz(46.0, 17.0, 26.0)
                        .looking_at(Vec3::new(18.0, 0.0, 3.0), Vec3::Y),
                },
            ));
        }
    }
    commands.spawn((
        DirectionalLight {
            illuminance: 100_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, -0.8, -0.7, 0.0)),
    ));
    commands.insert_resource(GlobalAmbientLight {
        color: SKY_LIGHT,
        brightness: AMBIENT_BRIGHTNESS,
        ..default()
    });
}

fn ambient_light() -> AmbientLight {
    AmbientLight {
        color: SKY_LIGHT,
        brightness: AMBIENT_BRIGHTNESS,
        ..default()
    }
}

fn fog() -> DistanceFog {
    DistanceFog {
        color: Color::srgb(0.82, 0.88, 0.97),
        falloff: FogFalloff::Linear {
            start: 400.0,
            end: 1_200.0,
        },
        ..default()
    }
}
