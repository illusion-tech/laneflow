//! Bevy 官方 externally-driven offscreen 模式；渲染不改变交通输入。
use std::{error::Error, path::Path, time::Duration};

use bevy::{
    camera::{RenderTarget, ScalingMode},
    prelude::*,
    render::{
        RenderApp, RenderPlugin,
        pipelined_rendering::PipelinedRenderingPlugin,
        render_resource::{PollType, TextureFormat},
        renderer::{RenderAdapterInfo, RenderDevice},
        view::screenshot::{Screenshot, save_to_disk},
    },
    window::ExitCondition,
    winit::WinitPlugin,
};
use laneflow_junction_generator::GridCatalog;
use serde_json::{Value, json};

#[derive(Resource)]
struct Target(Handle<Image>);

pub fn plugins(app: &mut App) {
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .set(RenderPlugin {
                synchronous_pipeline_compilation: true,
                ..default()
            })
            .disable::<WinitPlugin>()
            .disable::<PipelinedRenderingPlugin>(),
    );
}

pub fn setup(app: &mut App, entities: &[Entity], grid: &GridCatalog) -> Value {
    let image = Image::new_target_texture(1600, 1000, TextureFormat::Rgba8UnormSrgb, None);
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    let mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(2.0, 1.5, 4.5));
    let material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb(0.2, 0.8, 0.75),
            unlit: true,
            ..default()
        });
    for &entity in entities {
        app.world_mut()
            .entity_mut(entity)
            .insert((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone())));
    }
    let extent =
        grid.columns.max(grid.cells.len().div_ceil(grid.columns)) as f32 * grid.pitch_meters as f32;
    app.world_mut().spawn((
        Camera3d::default(),
        Camera {
            clear_color: Color::srgb(0.025, 0.04, 0.07).into(),
            ..default()
        },
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: extent,
            },
            far: 100_000.0,
            ..OrthographicProjection::default_3d()
        }),
        RenderTarget::Image(target.clone().into()),
        Transform::from_xyz(0.0, extent, 0.0).looking_at(Vec3::ZERO, Vec3::Z),
    ));
    app.insert_resource(Target(target));
    json!({"kind":"offscreen-unlit-cuboid-proxies","width":1600,"height":1000,"proxy_count":entities.len(),
        "camera":"whole-centered-grid-orthographic","camera_extent_meters":extent,"pipelined_rendering":false,
        "measurement":"render extraction, submission and synchronous GPU completion; not GPU timestamp duration"})
}

pub fn render(app: &mut App) -> Result<(), Box<dyn Error>> {
    app.update_sub_app_by_label(RenderApp);
    app.world()
        .resource::<RenderDevice>()
        .poll(PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(30)),
        })
        .map_err(|error| format!("GPU completion wait: {error:?}"))?;
    Ok(())
}

pub fn adapter_info(app: &App) -> Value {
    let adapter = app.world().resource::<RenderAdapterInfo>();
    json!({"name":adapter.name,"driver":adapter.driver,"driver_info":adapter.driver_info,
        "backend":format!("{:?}",adapter.backend),"device_type":format!("{:?}",adapter.device_type)})
}

pub fn visible_count(app: &App, entities: &[Entity]) -> usize {
    entities
        .iter()
        .filter(|&&entity| {
            app.world()
                .get::<ViewVisibility>(entity)
                .is_some_and(|visibility| visibility.get())
        })
        .count()
}

pub fn preview(app: &mut App, output: &Path) -> Result<(), Box<dyn Error>> {
    let target = app.world().resource::<Target>().0.clone();
    let destination = output.to_path_buf();
    app.world_mut()
        .spawn(Screenshot::image(target))
        .observe(save_to_disk(destination));
    for _ in 0..8 {
        app.sub_apps_mut().main.run_default_schedule();
        render(app)?;
        app.world_mut().clear_trackers();
    }
    if !output.is_file() {
        return Err("renderer preview was not written".into());
    }
    Ok(())
}
