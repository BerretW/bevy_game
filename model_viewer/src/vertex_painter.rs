//! Jednoduchý CPU vertex color painter pro ADS Model Viewer.
//!
//! Ovládání:
//!   P           — zapnout / vypnout paint mode
//!   L           — přepnout vrstvu (bevy_masks ↔ bevy_masks2)
//!   1–4         — vybrat kanál v aktivní vrstvě
//!   ↑ / ↓       — hodnota barvy (0.0 – 1.0)
//!   [ / ]       — poloměr štětce
//!   LMB drag    — malovat
//!   Shift+LMB   — erase (vrátí kanál na neutrální hodnotu)
//!   Del         — vymazat celou vrstvu na meshu pod kurzorem

use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

// ---------------------------------------------------------------------------
// Vrstvy a jejich kanály
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum PaintLayer {
    #[default]
    Masks0,  // ATTRIBUTE_COLOR   — bevy_masks  (RGBA)
    Masks1,  // ATTRIBUTE_UV_1    — bevy_masks2 (AO, Emissive)
}

impl PaintLayer {
    fn next(self) -> Self {
        match self { PaintLayer::Masks0 => PaintLayer::Masks1, PaintLayer::Masks1 => PaintLayer::Masks0 }
    }
    fn name(self) -> &'static str {
        match self { PaintLayer::Masks0 => "bevy_masks", PaintLayer::Masks1 => "bevy_masks2" }
    }
    fn channels(self) -> &'static [&'static str] {
        match self {
            PaintLayer::Masks0 => &["R  normal_suppress", "G  dirt", "B  wet", "A  palette"],
            PaintLayer::Masks1 => &["x  AO (1=none)", "y  emissive"],
        }
    }
    /// Neutrální hodnota pro erase — "žádný efekt" pro daný kanál.
    fn erase_value(self, channel: usize) -> f32 {
        match (self, channel) {
            (PaintLayer::Masks1, 0) => 1.0,  // AO: 1.0 = bez okluzí
            _ => 0.0,
        }
    }
}

fn ch_color(layer: PaintLayer, ch: usize) -> Color {
    match layer {
        PaintLayer::Masks0 => match ch {
            0 => Color::srgb(1.0, 0.3, 0.3),
            1 => Color::srgb(0.3, 1.0, 0.3),
            2 => Color::srgb(0.3, 0.5, 1.0),
            _ => Color::srgb(1.0, 1.0, 0.3),
        },
        PaintLayer::Masks1 => match ch {
            0 => Color::srgb(0.8, 0.6, 1.0),  // AO — fialová
            _ => Color::srgb(1.0, 0.6, 0.2),  // emissive — oranžová
        },
    }
}

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

#[derive(Resource)]
pub struct VertexPainter {
    pub active:  bool,
    pub layer:   PaintLayer,
    pub channel: usize,    // index do layer.channels()
    pub value:   f32,      // 0..1
    pub radius:  f32,      // world-space poloměr štětce
    hover:       Option<(Vec3, Vec3)>,  // (world_pos, world_nrm)
}

impl Default for VertexPainter {
    fn default() -> Self {
        Self { active: false, layer: PaintLayer::Masks0, channel: 1, value: 1.0, radius: 0.5, hover: None }
    }
}

#[derive(Component)]
pub struct PaintStatusText;

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

pub struct VertexPainterPlugin;

impl Plugin for VertexPainterPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VertexPainter>()
            .add_systems(Startup, spawn_paint_ui)
            .add_systems(Update, (
                painter_tick,
                draw_paint_cursor,
                update_paint_status,
            ).chain());
    }
}

fn spawn_paint_ui(mut commands: Commands) {
    commands.spawn((
        Text::new("P = vertex paint mode"),
        TextFont { font_size: 13.0, ..default() },
        TextColor(Color::srgba(1.0, 0.85, 0.3, 0.85)),
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(52.0),
            right:  Val::Px(8.0),
            ..default()
        },
        PaintStatusText,
    ));
}

// ---------------------------------------------------------------------------
// Möller–Trumbore ray-triangle intersection
// ---------------------------------------------------------------------------

fn ray_tri(o: Vec3, d: Vec3, v0: Vec3, v1: Vec3, v2: Vec3) -> Option<f32> {
    const EPS: f32 = 1e-7;
    let e1 = v1 - v0;
    let e2 = v2 - v0;
    let h  = d.cross(e2);
    let a  = e1.dot(h);
    if a.abs() < EPS { return None; }
    let f  = 1.0 / a;
    let s  = o - v0;
    let u  = f * s.dot(h);
    if !(0.0..=1.0).contains(&u) { return None; }
    let q  = s.cross(e1);
    let v  = f * d.dot(q);
    if v < 0.0 || u + v > 1.0 { return None; }
    let t  = f * e2.dot(q);
    if t > EPS { Some(t) } else { None }
}

fn cast_ray_mesh(mesh: &Mesh, lo: Vec3, ld: Vec3) -> Option<(f32, Vec3)> {
    let Some(VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else { return None };

    let mut best_t = f32::INFINITY;
    let mut best_n = Vec3::Y;

    macro_rules! test_tri {
        ($i0:expr, $i1:expr, $i2:expr) => {{
            let v0 = Vec3::from(pos[$i0]);
            let v1 = Vec3::from(pos[$i1]);
            let v2 = Vec3::from(pos[$i2]);
            if let Some(t) = ray_tri(lo, ld, v0, v1, v2) {
                if t < best_t {
                    best_t = t;
                    best_n = (v1 - v0).cross(v2 - v0).normalize_or_zero();
                }
            }
        }};
    }

    match mesh.indices() {
        Some(Indices::U32(idx)) => {
            for c in idx.chunks_exact(3) {
                test_tri!(c[0] as usize, c[1] as usize, c[2] as usize);
            }
        }
        Some(Indices::U16(idx)) => {
            for c in idx.chunks_exact(3) {
                test_tri!(c[0] as usize, c[1] as usize, c[2] as usize);
            }
        }
        None => {
            for i in (0..pos.len().saturating_sub(2)).step_by(3) {
                test_tri!(i, i + 1, i + 2);
            }
        }
    }

    if best_t.is_finite() { Some((best_t, best_n)) } else { None }
}

// ---------------------------------------------------------------------------
// Čtení / zápis obou vrstev
// ---------------------------------------------------------------------------

fn read_masks0(mesh: &Mesh, n: usize) -> Vec<[f32; 4]> {
    match mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
        Some(VertexAttributeValues::Float32x4(c)) => c.clone(),
        Some(VertexAttributeValues::Uint8x4(c)) => c.iter()
            .map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, p[3] as f32 / 255.0])
            .collect(),
        _ => vec![[0.0; 4]; n],
    }
}

fn read_masks1(mesh: &Mesh, n: usize) -> Vec<[f32; 2]> {
    match mesh.attribute(Mesh::ATTRIBUTE_UV_1) {
        Some(VertexAttributeValues::Float32x2(c)) => c.clone(),
        _ => vec![[1.0, 0.0]; n],  // AO=1 (žádný), emissive=0
    }
}

// ---------------------------------------------------------------------------
// Malování
// ---------------------------------------------------------------------------

fn paint_vertices(
    mesh:      &mut Mesh,
    world_hit: Vec3,
    world_mat: &Mat4,
    radius:    f32,
    layer:     PaintLayer,
    channel:   usize,
    value:     f32,
) {
    let Some(VertexAttributeValues::Float32x3(pos_raw)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else { return };
    let n = pos_raw.len();
    let positions: Vec<Vec3> = pos_raw.iter().map(|p| Vec3::from(*p)).collect();
    let r2 = radius * radius;

    let blend = |old: f32, val: f32, falloff: f32| -> f32 { old + (val - old) * falloff };

    match layer {
        PaintLayer::Masks0 => {
            let mut colors = read_masks0(mesh, n);
            for (i, lp) in positions.iter().enumerate() {
                let dist2 = (world_mat.transform_point3(*lp) - world_hit).length_squared();
                if dist2 < r2 {
                    let falloff = 1.0 - (dist2 / r2).sqrt();
                    colors[i][channel] = blend(colors[i][channel], value, falloff);
                }
            }
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
        }
        PaintLayer::Masks1 => {
            let mut uvs = read_masks1(mesh, n);
            for (i, lp) in positions.iter().enumerate() {
                let dist2 = (world_mat.transform_point3(*lp) - world_hit).length_squared();
                if dist2 < r2 {
                    let falloff = 1.0 - (dist2 / r2).sqrt();
                    uvs[i][channel] = blend(uvs[i][channel], value, falloff);
                }
            }
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uvs);
        }
    }
}

fn clear_layer(mesh: &mut Mesh, layer: PaintLayer) {
    let n = mesh.count_vertices();
    match layer {
        PaintLayer::Masks0 => mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0_f32; 4]; n]),
        PaintLayer::Masks1 => mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[1.0_f32, 0.0]; n]),
    }
}

// ---------------------------------------------------------------------------
// Hlavní systém: vstup + raycast + painting
// ---------------------------------------------------------------------------

pub fn painter_tick(
    mut painter:     ResMut<VertexPainter>,
    keys:            Res<ButtonInput<KeyCode>>,
    mouse:           Res<ButtonInput<MouseButton>>,
    windows:         Query<&Window, With<PrimaryWindow>>,
    camera_q:        Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mesh_q:          Query<(&Mesh3d, &GlobalTransform)>,
    mut meshes:      ResMut<Assets<Mesh>>,
    mut dbg_tick:    Local<u32>,
) {
    // Toggle
    if keys.just_pressed(KeyCode::KeyP) {
        painter.active = !painter.active;
        painter.hover  = None;
        let mesh_count = mesh_q.iter().count();
        let cam_ok     = camera_q.single().is_ok();
        let cursor     = windows.single().ok().and_then(|w| w.cursor_position());
        info!("[painter] paint mode: {}  cam_ok={}  meshes={}  cursor={:?}",
            if painter.active { "ON" } else { "OFF" },
            cam_ok, mesh_count, cursor);
    }

    // Diagnostics každou sekundu (60 ticků) pokud je aktivní
    if painter.active {
        *dbg_tick += 1;
        if *dbg_tick % 60 == 1 {
            let mesh_count = mesh_q.iter().count();
            let cam_ok     = camera_q.single().is_ok();
            let cursor     = windows.single().ok().and_then(|w| w.cursor_position());
            info!("[painter] diag  cam_ok={}  meshes={}  cursor={:?}  hover={:?}",
                cam_ok, mesh_count, cursor, painter.hover.is_some());
        }
    }

    // Přepnutí vrstvy (L)
    if keys.just_pressed(KeyCode::KeyL) {
        painter.layer   = painter.layer.next();
        painter.channel = 0;  // reset kanálu při změně vrstvy
    }

    // Kanál 1–N (max 4)
    let max_ch = painter.layer.channels().len();
    for (i, kc) in [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4]
        .iter().enumerate().take(max_ch)
    {
        if keys.just_pressed(*kc) { painter.channel = i; }
    }

    // Poloměr
    if keys.pressed(KeyCode::ArrowLeft)  { painter.radius = (painter.radius - 0.02).max(0.05); }
    if keys.pressed(KeyCode::ArrowRight) { painter.radius = (painter.radius + 0.02).min(5.0);  }

    // Hodnota
    if keys.just_pressed(KeyCode::ArrowUp)   { painter.value = (painter.value + 0.1).min(1.0); }
    if keys.just_pressed(KeyCode::ArrowDown) { painter.value = (painter.value - 0.1).max(0.0); }

    if !painter.active { painter.hover = None; return; }

    // --- Raycast ---
    let Ok(window)        = windows.single()  else { painter.hover = None; return; };
    let Some(cursor)      = window.cursor_position() else { painter.hover = None; return; };
    let Ok((cam, cam_gt)) = camera_q.single() else { painter.hover = None; return; };
    let Ok(ray)           = cam.viewport_to_world(cam_gt, cursor) else { painter.hover = None; return; };
    let ray_o = ray.origin;
    let ray_d = *ray.direction;

    // Fáze 1: najdi nejbližší hit (imut. borrows uvolněny po smyčce)
    struct HitData { mesh_id: AssetId<Mesh>, world_pos: Vec3, world_nrm: Vec3, world_mat: Mat4 }
    let mut best: Option<HitData> = None;
    let mut best_dist = f32::INFINITY;

    for (mesh_h, gt) in &mesh_q {
        let Some(mesh) = meshes.get(mesh_h.id()) else { continue };
        let wm  = gt.to_matrix();
        let inv = wm.inverse();
        let lo  = inv.transform_point3(ray_o);
        let ld  = inv.transform_vector3(ray_d).normalize_or_zero();

        let Some((lt, ln)) = cast_ray_mesh(mesh, lo, ld) else { continue };

        let world_hit  = wm.transform_point3(lo + ld * lt);
        let world_dist = (world_hit - ray_o).length();

        if world_dist < best_dist {
            best_dist = world_dist;
            best = Some(HitData {
                mesh_id:   mesh_h.id(),
                world_pos: world_hit,
                world_nrm: wm.transform_vector3(ln).normalize_or_zero(),
                world_mat: wm,
            });
        }
    }

    if let Some(hit) = best {
        let was_none = painter.hover.is_none();
        painter.hover = Some((hit.world_pos, hit.world_nrm));
        if was_none {
            info!("[painter] mesh hit at {:?}, nrm {:?}", hit.world_pos, hit.world_nrm);
        }

        let erase   = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        let paint_v = if erase { painter.layer.erase_value(painter.channel) } else { painter.value };

        if mouse.pressed(MouseButton::Left) {
            if let Some(mesh) = meshes.get_mut(hit.mesh_id) {
                paint_vertices(
                    mesh, hit.world_pos, &hit.world_mat,
                    painter.radius, painter.layer, painter.channel, paint_v,
                );
            }
        }

        if keys.just_pressed(KeyCode::Delete) {
            if let Some(mesh) = meshes.get_mut(hit.mesh_id) {
                clear_layer(mesh, painter.layer);
            }
        }
    } else {
        if painter.hover.is_some() {
            info!("[painter] no mesh hit (cursor off model)");
        }
        painter.hover = None;
    }
}

// ---------------------------------------------------------------------------
// Gizmo: kruhový kurzor štětce
// ---------------------------------------------------------------------------

fn draw_circle_gizmo(gizmos: &mut Gizmos, center: Vec3, normal: Vec3, radius: f32, color: Color) {
    const SEG: u32 = 32;
    let n = normal.normalize_or_zero();
    if n == Vec3::ZERO { return; }
    let t = if n.dot(Vec3::Y).abs() < 0.95 {
        n.cross(Vec3::Y).normalize_or_zero()
    } else {
        n.cross(Vec3::X).normalize_or_zero()
    };
    if t == Vec3::ZERO { return; }
    let b = n.cross(t);
    let step = std::f32::consts::TAU / SEG as f32;
    let mut prev = center + t * radius;
    for i in 1..=SEG {
        let a    = i as f32 * step;
        let next = center + (t * a.cos() + b * a.sin()) * radius;
        gizmos.line(prev, next, color);
        prev = next;
    }
    // Kříž uprostřed pro přesné zaměření
    let s = radius * 0.12;
    gizmos.line(center - t * s, center + t * s, color);
    gizmos.line(center - b * s, center + b * s, color);
}

pub fn draw_paint_cursor(
    painter:  Res<VertexPainter>,
    mut gizmos: Gizmos,
    cam_q:    Query<&GlobalTransform, With<Camera3d>>,
) {
    if !painter.active { return; }

    if let Some((pos, nrm)) = painter.hover {
        // Offset větší než dřív — bezpečně vyhne z-fightingu s povrchem meshe
        let draw_pos = pos + nrm * 0.05;
        let color = ch_color(painter.layer, painter.channel);
        draw_circle_gizmo(&mut gizmos, draw_pos, nrm, painter.radius, color);
        // Druhý tenčí kruh jako aura — výraznější kontrast
        draw_circle_gizmo(&mut gizmos, draw_pos, nrm, painter.radius * 1.05, Color::srgba(0.0, 0.0, 0.0, 0.5));
    } else {
        // Fallback: kursor není nad meshem — nakreslí malý kruh přímo před kamerou
        // Díky tomu uvidíš, že paint mode je aktivní i bez modelu pod myší.
        let Ok(cam_gt) = cam_q.single() else { return };
        // Kamera v Bevy/OpenGL kouká podél -Z v lokálním prostoru
        let mat = cam_gt.to_matrix();
        let fwd: Vec3 = -mat.col(2).truncate().normalize_or_zero();
        let center = cam_gt.translation() + fwd * 1.5;
        // Normal pointing back at camera → kruh vypadá jako kruh, ne elipsa
        draw_circle_gizmo(&mut gizmos, center, -fwd, 0.05, Color::srgba(1.0, 1.0, 0.0, 0.8));
    }
}

// ---------------------------------------------------------------------------
// Status text
// ---------------------------------------------------------------------------

pub fn update_paint_status(
    painter: Res<VertexPainter>,
    mut q:   Query<&mut Text, With<PaintStatusText>>,
) {
    if !painter.is_changed() { return; }
    let Ok(mut txt) = q.single_mut() else { return };
    let new = if painter.active {
        let channels = painter.layer.channels();
        let ch_name  = channels.get(painter.channel).copied().unwrap_or("?");
        format!(
            "PAINT  {layer}  ch:{ch}  val:{val:.1}  r:{r:.2}    L=vrstva  1-{n}=kanál  Left/Right=radius  UP/Down↓=hodnota  Shift=erase  Del=clear  P=off",
            layer = painter.layer.name(),
            ch    = ch_name,
            val   = painter.value,
            r     = painter.radius,
            n     = channels.len(),
        )
    } else {
        "P = vertex paint mode".to_string()
    };
    if txt.0 != new { txt.0 = new; }
}
