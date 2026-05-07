//! Klientský GUI render plugin.
//!
//! Přečte `GuiDrawBuffer` každý PostUpdate frame a vykreslí příkazy
//! přes pre-alokovaný pool Sprite + Text2d entit na dedikované 2D overlay kameře.
//!
//! Architektura:
//! - `Camera2d` (order 10, no clear) — GUI vrstva nad 3D scénou.
//! - RenderLayers::layer(GUI_LAYER_ID) odděluje GUI entity od 3D kamery.
//! - Pool 128 rect slotů + 32 text slotů + 16 image slotů.
//! - `GuiHighWater` sleduje kolik slotů bylo viditelných minulý frame —
//!   skrýváme pouze ty, ne celý pool (výrazně méně ECS change-detection mutací).

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::sprite::Anchor;
use bevy::window::PrimaryWindow;

use core_resources::{DrawCommand, FontLoadQueue, GuiDrawBuffer, ImageLoadQueue};

const GUI_LAYER_ID: usize = 31;

const RECT_POOL_SIZE: usize = 128;
const TEXT_POOL_SIZE: usize = 32;
const IMAGE_POOL_SIZE: usize = 16;
const CIRCLE_SEGMENTS: usize = 24;

#[derive(Component)]
struct GuiRectSlot;

#[derive(Component)]
struct GuiTextSlot;

#[derive(Component)]
struct GuiImageSlot;

#[derive(Component)]
struct GuiCamera;

/// Registry nahranych fontu (font_id -> Handle<Font>).
#[derive(Resource, Default)]
pub struct GuiFontRegistry(pub HashMap<String, Handle<Font>>);

/// Registry nahranych obrazku (image_id -> Handle<Image>).
#[derive(Resource, Default)]
pub struct GuiImageRegistry(pub HashMap<String, Handle<Image>>);

#[derive(Resource)]
pub struct GuiPool {
    rects: Vec<Entity>,
    texts: Vec<Entity>,
    images: Vec<Entity>,
    #[allow(dead_code)] // udržuje texturu naživu po dobu existence poolu
    white_pixel: Handle<Image>,
}

/// Kolik slotů bylo viditelných v předchozím frame.
/// Skrýváme pouze 0..hw.*, ne celý pool — šetříme ECS change-detection mutace.
#[derive(Resource, Default)]
struct GuiHighWater {
    rects: usize,
    texts: usize,
    images: usize,
}

pub struct GuiRenderPlugin;

impl Plugin for GuiRenderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GuiFontRegistry>();
        app.init_resource::<GuiImageRegistry>();
        app.init_resource::<GuiHighWater>();
        app.add_systems(Startup, setup_gui);
        app.add_systems(Update, process_asset_queues);
        app.add_systems(PostUpdate, render_gui);
    }
}

fn setup_gui(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let white_pixel = images.add(Image::new_fill(
        Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[255u8, 255, 255, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    ));

    commands.spawn((
        Camera2d,
        Camera { order: 10, clear_color: ClearColorConfig::None, ..default() },
        RenderLayers::layer(GUI_LAYER_ID),
        GuiCamera,
    ));

    let rects: Vec<Entity> = (0..RECT_POOL_SIZE)
        .map(|i| {
            commands.spawn((
                Sprite {
                    image: white_pixel.clone(),
                    color: Color::WHITE,
                    custom_size: Some(Vec2::ONE),
                    ..default()
                },
                Transform::from_xyz(0.0, 0.0, 10.0 + i as f32 * 0.001),
                Visibility::Hidden,
                RenderLayers::layer(GUI_LAYER_ID),
                GuiRectSlot,
            )).id()
        })
        .collect();

    let texts: Vec<Entity> = (0..TEXT_POOL_SIZE)
        .map(|i| {
            commands.spawn((
                Text2d::new(""),
                TextFont { font_size: 16.0, ..default() },
                TextColor(Color::WHITE),
                Anchor::TOP_LEFT,
                Transform::from_xyz(0.0, 0.0, 20.0 + i as f32 * 0.001),
                Visibility::Hidden,
                RenderLayers::layer(GUI_LAYER_ID),
                GuiTextSlot,
            )).id()
        })
        .collect();

    let images_pool: Vec<Entity> = (0..IMAGE_POOL_SIZE)
        .map(|i| {
            commands.spawn((
                Sprite {
                    image: white_pixel.clone(),
                    color: Color::WHITE,
                    custom_size: Some(Vec2::ONE),
                    ..default()
                },
                Transform::from_xyz(0.0, 0.0, 15.0 + i as f32 * 0.001),
                Visibility::Hidden,
                RenderLayers::layer(GUI_LAYER_ID),
                GuiImageSlot,
            )).id()
        })
        .collect();

    commands.insert_resource(GuiPool { rects, texts, images: images_pool, white_pixel });
}

fn process_asset_queues(
    asset_server: Res<AssetServer>,
    font_queue: Res<FontLoadQueue>,
    image_queue: Res<ImageLoadQueue>,
    mut font_reg: ResMut<GuiFontRegistry>,
    mut image_reg: ResMut<GuiImageRegistry>,
) {
    for req in font_queue.drain() {
        let handle: Handle<Font> = asset_server.load(req.abs_path.clone());
        font_reg.0.insert(req.font_id, handle);
    }
    for req in image_queue.drain() {
        let handle: Handle<Image> = asset_server.load(req.abs_path.clone());
        image_reg.0.insert(req.image_id, handle);
    }
}

fn render_gui(
    draw_buffer: Res<GuiDrawBuffer>,
    pool: Res<GuiPool>,
    font_reg: Res<GuiFontRegistry>,
    image_reg: Res<GuiImageRegistry>,
    mut hw: ResMut<GuiHighWater>,
    window_q: Query<&Window, With<PrimaryWindow>>,
    mut rect_q: Query<
        (&mut Sprite, &mut Transform, &mut Visibility),
        (With<GuiRectSlot>, Without<GuiTextSlot>, Without<GuiImageSlot>),
    >,
    mut text_q: Query<
        (&mut Text2d, &mut TextFont, &mut TextColor, &mut Transform, &mut Visibility),
        (With<GuiTextSlot>, Without<GuiRectSlot>, Without<GuiImageSlot>),
    >,
    mut image_q: Query<
        (&mut Sprite, &mut Transform, &mut Visibility),
        (With<GuiImageSlot>, Without<GuiRectSlot>, Without<GuiTextSlot>),
    >,
) {
    let (win_w, win_h) = window_q
        .single()
        .map(|w| (w.width(), w.height()))
        .unwrap_or((1920.0, 1080.0));

    let cmds = draw_buffer.drain();

    // Skrýváme pouze sloty co byly viditelné minulý frame.
    hide_used(&pool, &hw, &mut rect_q, &mut text_q, &mut image_q);

    if cmds.is_empty() {
        hw.rects = 0;
        hw.texts = 0;
        hw.images = 0;
        return;
    }

    let mut ri = 0usize;
    let mut ti = 0usize;
    let mut ii = 0usize;

    let to_world = |nx: f32, ny: f32| Vec2::new((nx - 0.5) * win_w, (0.5 - ny) * win_h);

    for cmd in cmds {
        match cmd {
            DrawCommand::Rect { x, y, w, h, color } => {
                write_rect(
                    &pool.rects, &mut rect_q, &mut ri,
                    to_world(x, y),
                    Vec2::new(w * win_w, h * win_h),
                    0.0, color,
                );
            }

            DrawCommand::Line { x1, y1, x2, y2, color } => {
                let p1 = to_world(x1, y1);
                let p2 = to_world(x2, y2);
                let diff = p2 - p1;
                let len = diff.length();
                if len < 0.5 { continue; }
                let angle = diff.y.atan2(diff.x);
                write_rect(
                    &pool.rects, &mut rect_q, &mut ri,
                    (p1 + p2) * 0.5,
                    Vec2::new(len, 2.0),
                    angle, color,
                );
            }

            DrawCommand::Circle { x, y, radius, color } => {
                let cx = (x - 0.5) * win_w;
                let cy = (0.5 - y) * win_h;
                let r = radius * win_w.min(win_h);
                let step = std::f32::consts::TAU / CIRCLE_SEGMENTS as f32;
                for seg in 0..CIRCLE_SEGMENTS {
                    let a1 = seg as f32 * step;
                    let a2 = a1 + step;
                    let p1 = Vec2::new(cx + a1.cos() * r, cy + a1.sin() * r);
                    let p2 = Vec2::new(cx + a2.cos() * r, cy + a2.sin() * r);
                    let diff = p2 - p1;
                    let len = diff.length();
                    let angle = diff.y.atan2(diff.x);
                    write_rect(
                        &pool.rects, &mut rect_q, &mut ri,
                        (p1 + p2) * 0.5,
                        Vec2::new(len + 1.0, 2.0),
                        angle, color,
                    );
                }
            }

            DrawCommand::Text { text, x, y, scale, color, font_id } => {
                if ti >= pool.texts.len() { continue; }
                let world = to_world(x, y);
                let e = pool.texts[ti];
                if let Ok((mut t2d, mut font, mut tc, mut tr, mut vis)) = text_q.get_mut(e) {
                    t2d.0 = text;
                    font.font_size = (scale * win_h * 0.04).clamp(8.0, 256.0);
                    if let Some(fid) = font_id.as_deref() {
                        if let Some(h) = font_reg.0.get(fid) {
                            font.font = h.clone();
                        }
                    } else {
                        font.font = Handle::default();
                    }
                    *tc = TextColor(Color::srgba_u8(color[0], color[1], color[2], color[3]));
                    tr.translation.x = world.x;
                    tr.translation.y = world.y;
                    vis.set_if_neq(Visibility::Visible);
                }
                ti += 1;
            }

            DrawCommand::Sprite { image_id, x, y, w, h, color } => {
                if ii >= pool.images.len() { continue; }
                let Some(img_handle) = image_reg.0.get(&image_id) else { continue };
                let e = pool.images[ii];
                if let Ok((mut spr, mut tr, mut vis)) = image_q.get_mut(e) {
                    spr.image = img_handle.clone();
                    spr.color = Color::srgba_u8(color[0], color[1], color[2], color[3]);
                    spr.custom_size = Some(Vec2::new(w * win_w, h * win_h));
                    let world = to_world(x, y);
                    tr.translation.x = world.x;
                    tr.translation.y = world.y;
                    tr.rotation = Quat::IDENTITY;
                    vis.set_if_neq(Visibility::Visible);
                }
                ii += 1;
            }
        }
    }

    hw.rects = ri;
    hw.texts = ti;
    hw.images = ii;
}

/// Skryje pouze sloty 0..hw.* — ne celý pool.
fn hide_used(
    pool: &GuiPool,
    hw: &GuiHighWater,
    rect_q: &mut Query<
        (&mut Sprite, &mut Transform, &mut Visibility),
        (With<GuiRectSlot>, Without<GuiTextSlot>, Without<GuiImageSlot>),
    >,
    text_q: &mut Query<
        (&mut Text2d, &mut TextFont, &mut TextColor, &mut Transform, &mut Visibility),
        (With<GuiTextSlot>, Without<GuiRectSlot>, Without<GuiImageSlot>),
    >,
    image_q: &mut Query<
        (&mut Sprite, &mut Transform, &mut Visibility),
        (With<GuiImageSlot>, Without<GuiRectSlot>, Without<GuiTextSlot>),
    >,
) {
    for &e in &pool.rects[..hw.rects] {
        if let Ok((_, _, mut vis)) = rect_q.get_mut(e) {
            vis.set_if_neq(Visibility::Hidden);
        }
    }
    for &e in &pool.texts[..hw.texts] {
        if let Ok((_, _, _, _, mut vis)) = text_q.get_mut(e) {
            vis.set_if_neq(Visibility::Hidden);
        }
    }
    for &e in &pool.images[..hw.images] {
        if let Ok((_, _, mut vis)) = image_q.get_mut(e) {
            vis.set_if_neq(Visibility::Hidden);
        }
    }
}

fn write_rect(
    pool: &[Entity],
    rect_q: &mut Query<
        (&mut Sprite, &mut Transform, &mut Visibility),
        (With<GuiRectSlot>, Without<GuiTextSlot>, Without<GuiImageSlot>),
    >,
    idx: &mut usize,
    center: Vec2,
    size: Vec2,
    rotation_z: f32,
    color: [u8; 4],
) {
    if *idx >= pool.len() { return; }
    let e = pool[*idx];
    if let Ok((mut spr, mut tr, mut vis)) = rect_q.get_mut(e) {
        spr.color = Color::srgba_u8(color[0], color[1], color[2], color[3]);
        spr.custom_size = Some(size);
        tr.translation.x = center.x;
        tr.translation.y = center.y;
        tr.rotation = Quat::from_rotation_z(rotation_z);
        vis.set_if_neq(Visibility::Visible);
    }
    *idx += 1;
}
