use bevy::ecs::hierarchy::ChildOf;
use bevy::gltf::Gltf;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use core_drawable::{
    AdsNodeKind,
    CollisionShape, DrawableCollision,
    DrawableManifest, DrawableManifestRegistry, EntityDef,
    LodGroup, TextureSource,
    StandardPbrMaterial, LayeredEnvMaterial,
};

use crate::camera;
use crate::state::{
    ColliderPanel, DebugStatus, GltfHandleCache, InfoOverlay, LodPanel, LodViewerState,
    RigPanel, RigViewerState, ViewerState, VCOL_DEBUG_MODES, WEATHER_PRESETS, WeatherState,
};

pub(crate) fn update_info_overlay(
    mut done: Local<bool>,
    drawable_reg: Res<DrawableManifestRegistry>,
    gltf_cache:   Res<GltfHandleCache>,
    manifests:    Res<Assets<DrawableManifest>>,
    gltf_assets:  Res<Assets<Gltf>>,
    mut overlay:  Query<&mut Text, With<InfoOverlay>>,
) {
    if *done { return; }

    let all_ready = gltf_cache.0.values().all(|(h, _)| gltf_assets.get(h).is_some());
    if !gltf_cache.0.is_empty() && !all_ready { return; }
    if gltf_cache.0.is_empty() && drawable_reg.0.is_empty() { return; }

    let mut lines: Vec<String> = Vec::new();

    for (stem, (gltf_handle, _path)) in &gltf_cache.0 {
        let Some(gltf) = gltf_assets.get(gltf_handle) else { continue };

        let tex_count = gltf.source.as_ref().map(|s| s.textures().count()).unwrap_or(0);
        let anim_count = gltf.source.as_ref().map(|s| s.animations().count()).unwrap_or(0);
        lines.push(format!("── {} ──", stem));
        lines.push(format!(
            "GLB  meshes:{} materials:{} textures:{} nodes:{} animations:{}",
            gltf.meshes.len(),
            gltf.materials.len(),
            tex_count,
            gltf.nodes.len(),
            anim_count,
        ));

        if let Some(manifest_handle) = drawable_reg.0.get(stem) {
            if let Some(manifest) = manifests.get(manifest_handle) {
                lines.push(format!(
                    "Drawable  asset:{} v{}",
                    manifest.asset_name, manifest.version
                ));

                let mesh_count = manifest.entities.values()
                    .filter(|e| matches!(e, EntityDef::MESH { .. })).count();
                let col_count = manifest.entities.values()
                    .filter(|e| matches!(e, EntityDef::COLLISION { .. })).count();
                if !manifest.entities.is_empty() {
                    lines.push(format!("Entities  MESH:{} COLLISION:{}", mesh_count, col_count));
                }

                let mut mat_names: Vec<&String> = manifest.materials.keys().collect();
                mat_names.sort();
                for mat_name in mat_names {
                    let mat = &manifest.materials[mat_name];
                    lines.push(format!("  [{}]  template:{}", mat_name, mat.template));

                    let mut tex_names: Vec<&String> = mat.textures.keys().collect();
                    tex_names.sort();
                    for slot in tex_names {
                        let tex = &mat.textures[slot];
                        let src = match tex.source {
                            TextureSource::Embedded => "embedded",
                            TextureSource::Shared => "shared",
                        };
                        lines.push(format!("    {} → {} ({})", slot, tex.name, src));
                    }

                    let p = &mat.params;
                    let mut params: Vec<String> = Vec::new();
                    if let Some(t) = p.tiling       { params.push(format!("tiling:{:.2}", t)); }
                    if let Some(t) = p.l0_tiling     { params.push(format!("l0_tiling:{:.2}", t)); }
                    if let Some(t) = p.l1_tiling     { params.push(format!("l1_tiling:{:.2}", t)); }
                    if let Some(v) = p.snow_level    { params.push(format!("snow:{:.2}", v)); }
                    if let Some(v) = p.dirt_level    { params.push(format!("dirt:{:.2}", v)); }
                    if let Some(v) = p.wetness       { params.push(format!("wet:{:.2}", v)); }
                    if let Some(v) = p.porosity      { params.push(format!("porosity:{:.2}", v)); }
                    if let Some(c) = p.tint {
                        params.push(format!("tint:[{:.2},{:.2},{:.2},{:.2}]", c[0], c[1], c[2], c[3]));
                    }
                    if let Some(m) = &p.opacity_mode {
                        let at = p.alpha_threshold.map(|v| format!("@{:.2}", v)).unwrap_or_default();
                        params.push(format!("alpha:{}{}", m, at));
                    }
                    if !params.is_empty() {
                        lines.push(format!("    params: {}", params.join("  ")));
                    }
                }
            }
        } else {
            lines.push("Drawable  (žádný manifest)".to_string());
        }
    }

    if let Ok(mut txt) = overlay.single_mut() {
        txt.0 = lines.join("\n");
    }
    *done = true;
}

pub(crate) fn draw_grid(mut gizmos: Gizmos, state: Res<ViewerState>) {
    if !state.grid_visible {
        return;
    }
    let half: i32 = 10;
    let gray = Color::srgba(0.45, 0.45, 0.45, 0.45);
    for i in -half..=half {
        let f = i as f32;
        gizmos.line(Vec3::new(f, 0.0, -(half as f32)), Vec3::new(f, 0.0, half as f32), gray);
        gizmos.line(Vec3::new(-(half as f32), 0.0, f), Vec3::new(half as f32, 0.0, f), gray);
    }
    gizmos.line(
        Vec3::new(-(half as f32), 0.001, 0.0),
        Vec3::new(half as f32, 0.001, 0.0),
        Color::srgb(0.8, 0.2, 0.2),
    );
    gizmos.line(
        Vec3::new(0.0, 0.001, -(half as f32)),
        Vec3::new(0.0, 0.001, half as f32),
        Color::srgb(0.2, 0.2, 0.8),
    );
}

pub(crate) fn handle_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut cam: ResMut<camera::OrbitState>,
    mut state: ResMut<ViewerState>,
    mut overlay: Query<&mut Visibility, With<InfoOverlay>>,
) {
    if keys.just_pressed(KeyCode::KeyR) {
        *cam = camera::OrbitState::default();
    }
    if keys.just_pressed(KeyCode::KeyG) {
        state.grid_visible = !state.grid_visible;
    }
    if keys.just_pressed(KeyCode::KeyH) {
        state.overlay_visible = !state.overlay_visible;
        for mut vis in &mut overlay {
            *vis = if state.overlay_visible {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
        }
    }
    if keys.just_pressed(KeyCode::KeyF) {
        cam.focus    = Vec3::ZERO;
        cam.distance = 2.5;
        cam.yaw      = -0.5;
        cam.pitch    = 0.35;
    }
    if keys.just_pressed(KeyCode::KeyC) {
        state.colliders_visible = !state.colliders_visible;
    }
    if keys.just_pressed(KeyCode::KeyX) {
        state.skeleton_visible = !state.skeleton_visible;
    }
}

pub(crate) fn sync_rig_viewer_state(
    mut rig: ResMut<RigViewerState>,
    nodes: Query<(Entity, &AdsNodeKind, Option<&Name>)>,
) {
    let mut targets: Vec<(String, Entity)> = nodes.iter()
        .filter_map(|(entity, kind, name)| {
            (*kind == AdsNodeKind::IkTarget).then(|| {
                (
                    name.map(|value| value.as_str().to_string())
                        .unwrap_or_else(|| format!("IK {}", entity.index())),
                    entity,
                )
            })
        })
        .collect();
    targets.sort_by(|a, b| a.0.cmp(&b.0));

    let entities: Vec<Entity> = targets.iter().map(|(_, entity)| *entity).collect();
    if entities != rig.ik_targets {
        rig.ik_names = targets.iter().map(|(name, _)| name.clone()).collect();
        rig.ik_targets = entities;
        if rig.selected_ik >= rig.ik_targets.len() {
            rig.selected_ik = 0;
        }
    }
    if rig.move_step <= 0.0 {
        rig.move_step = 0.05;
    }
}

pub(crate) fn handle_rig_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<ViewerState>,
    mut rig: ResMut<RigViewerState>,
    mut transforms: Query<&mut Transform>,
    mut debug_status: Query<(&mut Text, &mut Visibility), With<DebugStatus>>,
) {
    if keys.just_pressed(KeyCode::KeyI) {
        rig.ik_mode = !rig.ik_mode;
        rig.dragging_ik = false;
        if rig.ik_mode {
            state.skeleton_visible = true;
        }

        if let Ok((mut txt, mut vis)) = debug_status.single_mut() {
            *vis = Visibility::Visible;
            if rig.ik_names.is_empty() {
                txt.0 = format!(
                    "IK režim: {}\nIK prvky: 0\nSeznam: (žádné IK targety)",
                    if rig.ik_mode { "ON" } else { "OFF" }
                );
            } else {
                txt.0 = format!(
                    "IK režim: {}\nIK prvky: {}\nSeznam: {}",
                    if rig.ik_mode { "ON" } else { "OFF" },
                    rig.ik_names.len(),
                    rig.ik_names.join(", ")
                );
            }
        }
    }

    if rig.ik_targets.is_empty() {
        return;
    }

    if keys.just_pressed(KeyCode::Tab) {
        let backwards = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        if backwards {
            rig.selected_ik = if rig.selected_ik == 0 {
                rig.ik_targets.len().saturating_sub(1)
            } else {
                rig.selected_ik.saturating_sub(1)
            };
        } else {
            rig.selected_ik = (rig.selected_ik + 1) % rig.ik_targets.len();
        }
    }

    if !rig.ik_mode {
        return;
    }

    let Some(&selected) = rig.ik_targets.get(rig.selected_ik) else { return };
    let Ok(mut transform) = transforms.get_mut(selected) else { return };

    let mut delta = Vec3::ZERO;
    let step = if keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
        rig.move_step * 4.0
    } else {
        rig.move_step
    };

    if keys.pressed(KeyCode::KeyJ) { delta.x -= step; }
    if keys.pressed(KeyCode::KeyL) { delta.x += step; }
    if keys.pressed(KeyCode::ArrowUp) { delta.y += step; }
    if keys.pressed(KeyCode::ArrowDown) { delta.y -= step; }
    if keys.pressed(KeyCode::KeyU) { delta.z -= step; }
    if keys.pressed(KeyCode::KeyO) { delta.z += step; }

    if delta != Vec3::ZERO {
        transform.translation += delta;
    }
}

pub(crate) fn handle_rig_mouse_drag(
    windows: Query<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut rig: ResMut<RigViewerState>,
    ik_globals: Query<(Entity, &GlobalTransform, Option<&ChildOf>)>,
    parent_globals: Query<&GlobalTransform>,
    mut ik_locals: Query<&mut Transform>,
) {
    if !rig.ik_mode || rig.ik_targets.is_empty() {
        rig.hovered_ik = None;
        rig.dragging_ik = false;
        return;
    }

    let Ok(window) = windows.single() else { return };
    let Some(cursor) = window.cursor_position() else {
        rig.hovered_ik = None;
        rig.dragging_ik = false;
        return;
    };
    let Ok((camera, camera_gt)) = camera_q.single() else { return };

    let mut hovered: Option<(usize, f32)> = None;
    for (idx, entity) in rig.ik_targets.iter().enumerate() {
        let Ok((_, target_gt, _)) = ik_globals.get(*entity) else { continue };
        let Ok(screen_pos) = camera.world_to_viewport(camera_gt, target_gt.translation()) else { continue };
        let d = screen_pos.distance(cursor);
        if d <= 24.0 {
            match hovered {
                Some((_, best_d)) if d >= best_d => {}
                _ => hovered = Some((idx, d)),
            }
        }
    }
    rig.hovered_ik = hovered.map(|(idx, _)| idx);

    if mouse.just_pressed(MouseButton::Left) {
        if let Some(idx) = rig.hovered_ik {
            rig.selected_ik = idx;
            if let Some(&entity) = rig.ik_targets.get(idx) {
                if let Ok((_, target_gt, _)) = ik_globals.get(entity) {
                    rig.drag_distance = (target_gt.translation() - camera_gt.translation()).length().max(0.05);
                    rig.dragging_ik = true;
                }
            }
        } else {
            rig.dragging_ik = false;
        }
    }

    if mouse.just_released(MouseButton::Left) {
        rig.dragging_ik = false;
    }

    if !rig.dragging_ik || !mouse.pressed(MouseButton::Left) {
        return;
    }

    let Some(&selected) = rig.ik_targets.get(rig.selected_ik) else { return };
    let Ok(ray) = camera.viewport_to_world(camera_gt, cursor) else { return };
    let target_world = ray.origin + *ray.direction * rig.drag_distance;

    let Ok((_, _, child_of)) = ik_globals.get(selected) else { return };
    let local_target = if let Some(parent) = child_of.and_then(|c| parent_globals.get(c.parent()).ok()) {
        parent.to_matrix().inverse().transform_point3(target_world)
    } else {
        target_world
    };

    if let Ok(mut local_t) = ik_locals.get_mut(selected) {
        local_t.translation = local_target;
    }
}

pub(crate) fn update_rig_overlay(
    state: Res<ViewerState>,
    rig: Res<RigViewerState>,
    nodes: Query<&AdsNodeKind>,
    mut panel: Query<(&mut Text, &mut Visibility), With<RigPanel>>,
) {
    let Ok((mut txt, mut vis)) = panel.single_mut() else { return };

    if !state.skeleton_visible && !rig.ik_mode {
        *vis = Visibility::Hidden;
        txt.0 = String::new();
        return;
    }

    *vis = Visibility::Visible;
    let selected_name = rig.ik_names.get(rig.selected_ik).map(String::as_str).unwrap_or("-");
    let hovered_name = rig
        .hovered_ik
        .and_then(|idx| rig.ik_names.get(idx))
        .map(String::as_str)
        .unwrap_or("-");
    let rig_entity_count = nodes.iter().count();
    let lines = vec![
        format!("── Rig ──  Skeleton:{}  IK edit:{}", state.skeleton_visible, rig.ik_mode),
        format!("IK targets: {}  Selected: {}", rig.ik_targets.len(), selected_name),
        format!("Hover: {}  Drag: {}", hovered_name, if rig.dragging_ik { "ON" } else { "OFF" }),
        format!("Move step: {:.2}", rig.move_step),
        "X show bones | I IK mode | LMB pick+drag | Tab next target".to_string(),
        "ArrowUp/ArrowDown Y+/- | J/L X-/+ | U/O Z-/+ | Shift = faster move".to_string(),
        format!("Rig entities: {}", rig_entity_count),
    ];
    txt.0 = lines.join("\n");
}

pub(crate) fn draw_skeleton_overlay(
    mut gizmos: Gizmos,
    state: Res<ViewerState>,
    rig: Res<RigViewerState>,
    nodes: Query<(Entity, &AdsNodeKind, &GlobalTransform, Option<&ChildOf>)>,
) {
    if !state.skeleton_visible {
        return;
    }

    let selected = rig.ik_targets.get(rig.selected_ik).copied();
    let hovered = rig.hovered_ik.and_then(|idx| rig.ik_targets.get(idx)).copied();

    for (entity, kind, gt, parent) in &nodes {
        let pos = gt.translation();
        let color = match kind {
            AdsNodeKind::DeformationBone => Color::srgb(0.2, 0.9, 1.0),
            AdsNodeKind::IkTarget => {
                if Some(entity) == selected {
                    Color::srgb(1.0, 0.95, 0.2)
                } else if Some(entity) == hovered {
                    Color::srgb(1.0, 0.7, 0.25)
                } else {
                    Color::srgb(1.0, 0.45, 0.2)
                }
            }
            AdsNodeKind::Mechanical => Color::srgb(0.9, 0.5, 1.0),
            AdsNodeKind::Socket => Color::srgb(0.2, 1.0, 0.4),
            AdsNodeKind::Standard => continue,
        };

        let size = if *kind == AdsNodeKind::IkTarget { 0.055 } else { 0.025 };
        draw_cross_gizmo(&mut gizmos, pos, size, color);

        if let Some(parent) = parent {
            if let Ok((_, parent_kind, parent_gt, _)) = nodes.get(parent.parent()) {
                if *kind == AdsNodeKind::DeformationBone
                    || *kind == AdsNodeKind::IkTarget
                    || *parent_kind == AdsNodeKind::DeformationBone
                {
                    gizmos.line(parent_gt.translation(), pos, color.with_alpha(0.8));
                }
            }
        }
    }
}

pub(crate) fn sync_mesh_visibility_for_collider_mode(
    state: Res<ViewerState>,
    mut last: Local<bool>,
    mut meshes: Query<(&mut Visibility, Option<&Name>), (With<Mesh3d>, Without<DrawableCollision>)>,
) {
    if *last == state.colliders_visible { return; }
    *last = state.colliders_visible;

    if state.colliders_visible {
        for (mut vis, _) in &mut meshes {
            *vis = Visibility::Hidden;
        }
    } else {
        for (mut vis, name) in &mut meshes {
            let is_col = name.map(|n| n.as_str().starts_with("COL_")).unwrap_or(false);
            if !is_col {
                *vis = Visibility::Inherited;
            }
        }
    }
}

pub(crate) fn draw_colliders(
    mut gizmos: Gizmos,
    state: Res<ViewerState>,
    query: Query<(&DrawableCollision, &GlobalTransform, Option<&Mesh3d>, Option<&SkinnedMesh>)>,
    joint_globals: Query<&GlobalTransform>,
    mesh_assets: Res<Assets<Mesh>>,
    inverse_bindposes_assets: Res<Assets<SkinnedMeshInverseBindposes>>,
) {
    if !state.colliders_visible { return; }
    for (dc, gt, mesh3d, skinned) in &query {
        let (scale, rot, center) = gt.to_scale_rotation_translation();
        let color = collider_color(dc);
        match &dc.shape {
            CollisionShape::Box => {
                let he = dc.half_extents.unwrap_or(Vec3::splat(0.5)) * scale;
                draw_box_gizmo(&mut gizmos, center, rot, he, color);
            }
            CollisionShape::Mesh | CollisionShape::Convex => {
                if !draw_mesh_wireframe(
                    &mut gizmos,
                    mesh3d,
                    skinned,
                    &mesh_assets,
                    &inverse_bindposes_assets,
                    &joint_globals,
                    gt,
                    color,
                ) {
                    let he = dc.half_extents.unwrap_or(Vec3::splat(0.5)) * scale;
                    draw_box_gizmo(&mut gizmos, center, rot, he, color);
                }
            }
            CollisionShape::Navmesh => {
                if !draw_mesh_wireframe(
                    &mut gizmos,
                    mesh3d,
                    skinned,
                    &mesh_assets,
                    &inverse_bindposes_assets,
                    &joint_globals,
                    gt,
                    Color::srgb(0.95, 0.15, 0.8),
                ) {
                    let he = dc.half_extents.unwrap_or(Vec3::splat(0.5)) * scale;
                    draw_box_gizmo(&mut gizmos, center, rot, he, Color::srgb(0.95, 0.15, 0.8));
                }
            }
            CollisionShape::Sphere => {
                let r = dc.radius.unwrap_or(0.5) * scale.max_element();
                draw_sphere_gizmo(&mut gizmos, center, rot, r, color);
            }
            CollisionShape::Capsule => {
                let radial_scale = scale.x.max(scale.z);
                let r = dc.radius.unwrap_or(0.3) * radial_scale;
                let half_seg = dc.height
                    .map(|h| (h * 0.5 * scale.y - r).max(0.0))
                    .unwrap_or(0.5);
                draw_capsule_gizmo(&mut gizmos, center, rot, r, half_seg, color);
            }
            CollisionShape::Cylinder => {
                let radial_scale = scale.x.max(scale.z);
                let r = dc.radius.unwrap_or(0.5) * radial_scale;
                let half_h = dc.height.map(|h| h * 0.5 * scale.y).unwrap_or(0.5);
                draw_cylinder_gizmo(&mut gizmos, center, rot, r, half_h, color);
            }
        }
    }
}

fn draw_mesh_wireframe(
    gizmos: &mut Gizmos,
    mesh3d: Option<&Mesh3d>,
    skinned: Option<&SkinnedMesh>,
    mesh_assets: &Assets<Mesh>,
    inverse_bindposes_assets: &Assets<SkinnedMeshInverseBindposes>,
    joint_globals: &Query<&GlobalTransform>,
    gt: &GlobalTransform,
    color: Color,
) -> bool {
    let Some(mesh_h) = mesh3d else { return false };
    let Some(mesh) = mesh_assets.get(mesh_h.id()) else { return false };

    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { return false };

    let indices: Vec<u32> = match mesh.indices() {
        Some(Indices::U32(idx)) => idx.clone(),
        Some(Indices::U16(idx)) => idx.iter().map(|&i| i as u32).collect(),
        None => return false,
    };

    let skinned_world_positions = if let Some(skin) = skinned {
        let joint_indices = match mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX) {
            Some(VertexAttributeValues::Uint16x4(values)) => {
                Some(values.iter().map(|joint| [joint[0] as usize, joint[1] as usize, joint[2] as usize, joint[3] as usize]).collect::<Vec<_>>())
            }
            Some(VertexAttributeValues::Uint32x4(values)) => {
                Some(values.iter().map(|joint| [joint[0] as usize, joint[1] as usize, joint[2] as usize, joint[3] as usize]).collect::<Vec<_>>())
            }
            _ => None,
        };
        let joint_weights = match mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT) {
            Some(VertexAttributeValues::Float32x4(values)) => Some(values),
            _ => None,
        };
        let inverse_bindposes = inverse_bindposes_assets.get(&skin.inverse_bindposes);

        match (joint_indices, joint_weights, inverse_bindposes) {
            (Some(joint_indices), Some(joint_weights), Some(inverse_bindposes)) => {
                let mut out = Vec::with_capacity(positions.len());
                for (index, pos) in positions.iter().enumerate() {
                    let local = Vec4::new(pos[0], pos[1], pos[2], 1.0);
                    let mut world = Vec3::ZERO;
                    let mut total_weight = 0.0f32;
                    let joints = joint_indices.get(index).copied().unwrap_or([0, 0, 0, 0]);
                    let weights = joint_weights.get(index).copied().unwrap_or([0.0, 0.0, 0.0, 0.0]);

                    for i in 0..4 {
                        let weight = weights[i];
                        if weight <= 0.0 {
                            continue;
                        }
                        let joint_idx = joints[i];
                        let Some(&joint_entity) = skin.joints.get(joint_idx) else { continue };
                        let Some(inverse_bind) = inverse_bindposes.get(joint_idx) else { continue };
                        let Ok(joint_gt) = joint_globals.get(joint_entity) else { continue };
                        let skinned_point = (joint_gt.to_matrix() * *inverse_bind) * local;
                        world += skinned_point.truncate() * weight;
                        total_weight += weight;
                    }

                    if total_weight > 0.0 {
                        out.push(world);
                    } else {
                        out.push(gt.transform_point(Vec3::from(*pos)));
                    }
                }
                Some(out)
            }
            _ => None,
        }
    } else {
        None
    };

    use std::collections::HashSet;
    let mut drawn: HashSet<(u32, u32)> = HashSet::new();

    for tri in indices.chunks_exact(3) {
        let [a, b, c] = [tri[0], tri[1], tri[2]];
        for &(i, j) in &[(a, b), (b, c), (c, a)] {
            let edge = if i < j { (i, j) } else { (j, i) };
            if drawn.insert(edge) {
                let pa = if let Some(skinned_positions) = &skinned_world_positions {
                    skinned_positions[i as usize]
                } else {
                    gt.transform_point(Vec3::from(positions[i as usize]))
                };
                let pb = if let Some(skinned_positions) = &skinned_world_positions {
                    skinned_positions[j as usize]
                } else {
                    gt.transform_point(Vec3::from(positions[j as usize]))
                };
                gizmos.line(pa, pb, color);
            }
        }
    }

    !drawn.is_empty()
}

fn collider_color(dc: &DrawableCollision) -> Color {
    if dc.ladder         { Color::srgb(1.00, 0.85, 0.00) }
    else if dc.climbable { Color::srgb(0.00, 1.00, 0.40) }
    else if !dc.is_static { Color::srgb(1.00, 0.40, 0.10) }
    else                 { Color::srgb(0.10, 0.80, 1.00) }
}

pub(crate) fn update_collider_panel(
    state: Res<ViewerState>,
    colliders: Query<(&DrawableCollision, &GlobalTransform)>,
    mut panel: Query<(&mut Text, &mut Visibility), With<ColliderPanel>>,
) {
    let Ok((mut txt, mut vis)) = panel.single_mut() else { return };
    if !state.colliders_visible {
        *vis = Visibility::Hidden;
        return;
    }
    *vis = Visibility::Visible;

    let mut lines: Vec<String> = vec![
        "── Collidery (cyan=static  orange=dynamic  green=climbable  yellow=ladder) ──".to_string(),
    ];

    let items: Vec<_> = colliders.iter()
        .map(|(dc, gt)| (dc, gt.translation()))
        .collect();

    if items.is_empty() {
        lines.push("(žádné DrawableCollision entity)".to_string());
        lines.push("Model vyžaduje .drawable nebo .adm manifest.".to_string());
    } else {
        for (i, (dc, pos)) in items.iter().enumerate() {
            let size_str = match &dc.shape {
                CollisionShape::Box | CollisionShape::Convex | CollisionShape::Mesh | CollisionShape::Navmesh => {
                    let he = dc.half_extents.unwrap_or(Vec3::splat(0.5));
                    format!("he:[{:.2},{:.2},{:.2}]", he.x, he.y, he.z)
                }
                CollisionShape::Sphere => format!("r:{:.2}", dc.radius.unwrap_or(0.5)),
                CollisionShape::Capsule | CollisionShape::Cylinder => format!(
                    "r:{:.2}  h:{:.2}", dc.radius.unwrap_or(0.3), dc.height.unwrap_or(1.0)
                ),
            };
            let mut flags = vec![if dc.is_static { "static" } else { "dynamic" }];
            if dc.climbable { flags.push("climbable"); }
            if dc.ladder    { flags.push("ladder"); }

            lines.push(format!(
                "[{}] {:?}  {:?}  {}",
                i + 1, dc.shape, dc.material, flags.join("+")
            ));
            lines.push(format!(
                "    {}  @ [{:.2},{:.2},{:.2}]",
                size_str, pos.x, pos.y, pos.z
            ));
            if dc.friction != 0.0 || dc.restitution != 0.0 || dc.mass != 0.0 {
                lines.push(format!(
                    "    frict:{:.2}  rest:{:.2}  mass:{:.1}",
                    dc.friction, dc.restitution, dc.mass
                ));
            }
            if !dc.tags.is_empty() {
                lines.push(format!("    tags: {}", dc.tags.join(", ")));
            }
        }
    }
    txt.0 = lines.join("\n");
}

fn draw_box_gizmo(gizmos: &mut Gizmos, center: Vec3, rot: Quat, half: Vec3, color: Color) {
    let c = [
        center + rot * Vec3::new(-half.x, -half.y, -half.z),
        center + rot * Vec3::new( half.x, -half.y, -half.z),
        center + rot * Vec3::new( half.x,  half.y, -half.z),
        center + rot * Vec3::new(-half.x,  half.y, -half.z),
        center + rot * Vec3::new(-half.x, -half.y,  half.z),
        center + rot * Vec3::new( half.x, -half.y,  half.z),
        center + rot * Vec3::new( half.x,  half.y,  half.z),
        center + rot * Vec3::new(-half.x,  half.y,  half.z),
    ];
    gizmos.line(c[0], c[1], color); gizmos.line(c[1], c[2], color);
    gizmos.line(c[2], c[3], color); gizmos.line(c[3], c[0], color);
    gizmos.line(c[4], c[5], color); gizmos.line(c[5], c[6], color);
    gizmos.line(c[6], c[7], color); gizmos.line(c[7], c[4], color);
    gizmos.line(c[0], c[4], color); gizmos.line(c[1], c[5], color);
    gizmos.line(c[2], c[6], color); gizmos.line(c[3], c[7], color);
}

fn draw_cross_gizmo(gizmos: &mut Gizmos, center: Vec3, half: f32, color: Color) {
    gizmos.line(center - Vec3::X * half, center + Vec3::X * half, color);
    gizmos.line(center - Vec3::Y * half, center + Vec3::Y * half, color);
    gizmos.line(center - Vec3::Z * half, center + Vec3::Z * half, color);
}

fn draw_sphere_gizmo(gizmos: &mut Gizmos, center: Vec3, rot: Quat, radius: f32, color: Color) {
    let (rx, ry, rz) = (rot * Vec3::X, rot * Vec3::Y, rot * Vec3::Z);
    draw_arc_gizmo(gizmos, center, rx, ry, radius, 0.0, std::f32::consts::TAU, 24, color);
    draw_arc_gizmo(gizmos, center, rx, rz, radius, 0.0, std::f32::consts::TAU, 24, color);
    draw_arc_gizmo(gizmos, center, ry, rz, radius, 0.0, std::f32::consts::TAU, 24, color);
}

fn draw_cylinder_gizmo(
    gizmos: &mut Gizmos, center: Vec3, rot: Quat,
    radius: f32, half_h: f32, color: Color,
) {
    let (up, right, fwd) = (rot * Vec3::Y, rot * Vec3::X, rot * Vec3::Z);
    let (bot, top) = (center - up * half_h, center + up * half_h);
    draw_arc_gizmo(gizmos, bot, right, fwd, radius, 0.0, std::f32::consts::TAU, 24, color);
    draw_arc_gizmo(gizmos, top, right, fwd, radius, 0.0, std::f32::consts::TAU, 24, color);
    gizmos.line(bot + right * radius, top + right * radius, color);
    gizmos.line(bot - right * radius, top - right * radius, color);
    gizmos.line(bot + fwd   * radius, top + fwd   * radius, color);
    gizmos.line(bot - fwd   * radius, top - fwd   * radius, color);
}

fn draw_capsule_gizmo(
    gizmos: &mut Gizmos, center: Vec3, rot: Quat,
    radius: f32, half_seg: f32, color: Color,
) {
    let (up, right, fwd) = (rot * Vec3::Y, rot * Vec3::X, rot * Vec3::Z);
    let (bot, top) = (center - up * half_seg, center + up * half_seg);
    draw_arc_gizmo(gizmos, bot, right, fwd, radius, 0.0, std::f32::consts::TAU, 24, color);
    draw_arc_gizmo(gizmos, top, right, fwd, radius, 0.0, std::f32::consts::TAU, 24, color);
    gizmos.line(bot + right * radius, top + right * radius, color);
    gizmos.line(bot - right * radius, top - right * radius, color);
    gizmos.line(bot + fwd   * radius, top + fwd   * radius, color);
    gizmos.line(bot - fwd   * radius, top - fwd   * radius, color);
    draw_arc_gizmo(gizmos, top, right,  up, radius, 0.0, std::f32::consts::PI, 12, color);
    draw_arc_gizmo(gizmos, top, fwd,    up, radius, 0.0, std::f32::consts::PI, 12, color);
    draw_arc_gizmo(gizmos, bot, right, -up, radius, 0.0, std::f32::consts::PI, 12, color);
    draw_arc_gizmo(gizmos, bot, fwd,   -up, radius, 0.0, std::f32::consts::PI, 12, color);
}

fn draw_arc_gizmo(
    gizmos: &mut Gizmos,
    center: Vec3, t1: Vec3, t2: Vec3,
    radius: f32, start: f32, end: f32, segments: u32,
    color: Color,
) {
    let step = (end - start) / segments as f32;
    let mut prev = center + (t1 * start.cos() + t2 * start.sin()) * radius;
    for i in 1..=segments {
        let a = start + i as f32 * step;
        let next = center + (t1 * a.cos() + t2 * a.sin()) * radius;
        gizmos.line(prev, next, color);
        prev = next;
    }
}

pub(crate) fn handle_lod_key(
    keys: Res<ButtonInput<KeyCode>>,
    rig: Res<RigViewerState>,
    mut lod_state: ResMut<LodViewerState>,
    lod_groups: Query<&LodGroup>,
) {
    if rig.ik_mode {
        return;
    }
    if !keys.just_pressed(KeyCode::KeyL) { return; }

    lod_state.panel_active = true;

    let max_levels = lod_groups.iter()
        .map(|g| g.lod_entities.len())
        .max()
        .unwrap_or(0);

    if max_levels == 0 { return; }

    lod_state.forced = match lod_state.forced {
        None => Some(0),
        Some(n) => {
            let next = n as usize + 1;
            if next >= max_levels { None } else { Some(next as u8) }
        }
    };
}

pub(crate) fn apply_forced_lod(
    lod_state: Res<LodViewerState>,
    mut lod_groups: Query<&mut LodGroup>,
    mut visibility_q: Query<&mut Visibility>,
) {
    let Some(forced) = lod_state.forced else { return };

    for mut lod in &mut lod_groups {
        lod.active_lod = u8::MAX;
        for (level_idx, entities) in lod.lod_entities.iter().enumerate() {
            let vis = if level_idx as u8 == forced {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            for &entity in entities {
                if let Ok(mut v) = visibility_q.get_mut(entity) {
                    *v = vis;
                }
            }
        }
    }
}

pub(crate) fn update_lod_panel(
    mut lod_state: ResMut<LodViewerState>,
    lod_groups: Query<&LodGroup>,
    mut panel: Query<(&mut Text, &mut Visibility), With<LodPanel>>,
) {
    let Ok((mut txt, mut vis)) = panel.single_mut() else { return };

    let groups: Vec<&LodGroup> = lod_groups.iter().collect();

    if !groups.is_empty() {
        lod_state.panel_active = true;
    }

    if !lod_state.panel_active {
        *vis = Visibility::Hidden;
        return;
    }
    *vis = Visibility::Visible;

    if groups.is_empty() {
        txt.0 = "── LOD ──\nŽádné LOD skupiny (model nemá _LOD1+ uzly)".to_string();
        return;
    }

    let total_levels: usize = groups.iter().map(|g| g.lod_entities.len()).sum();
    let mode_str = match lod_state.forced {
        None => "auto".to_string(),
        Some(n) => format!("LOD{} vynuceno", n),
    };
    let mut lines = vec![
        format!("── LOD ({}) ──", mode_str),
        format!("Skupiny: {}  celkem úrovní: {}", groups.len(), total_levels),
    ];

    for (i, group) in groups.iter().enumerate() {
        let num_levels = group.lod_entities.len();
        let dists: Vec<String> = group.lod_dist_sq.iter()
            .map(|&d| format!("{:.0}m", d.sqrt()))
            .collect();
        let active_str = if group.active_lod == u8::MAX {
            "—".to_string()
        } else {
            format!("LOD{}", group.active_lod)
        };
        lines.push(format!(
            "  [{i}] {num_levels} úrovní  vzdálenosti:[{}]  aktivní:{active_str}{}",
            if dists.is_empty() { "výchozí".to_string() } else { dists.join(", ") },
            if group.cull_beyond_last { "  cull" } else { "" },
        ));
    }

    txt.0 = lines.join("\n");
}

pub(crate) fn draw_lod_circles(
    mut gizmos: Gizmos,
    lod_groups: Query<(&GlobalTransform, &LodGroup)>,
) {
    const LOD_COLORS: [Color; 3] = [
        Color::srgb(0.2, 1.0, 0.2),
        Color::srgb(1.0, 1.0, 0.2),
        Color::srgb(1.0, 0.4, 0.2),
    ];

    for (gt, lod) in &lod_groups {
        let center = gt.translation();
        for (i, &dist_sq) in lod.lod_dist_sq.iter().enumerate() {
            let radius = dist_sq.sqrt();
            let color = LOD_COLORS.get(i).copied()
                .unwrap_or(Color::srgb(0.8, 0.2, 0.8));
            draw_arc_gizmo(
                &mut gizmos,
                Vec3::new(center.x, 0.001, center.z),
                Vec3::X, Vec3::Z,
                radius, 0.0, std::f32::consts::TAU, 64,
                color,
            );
        }
    }
}

pub(crate) fn handle_material_debug(
    keys:          Res<ButtonInput<KeyCode>>,
    mut weather:   ResMut<WeatherState>,
    mut std_mats:  ResMut<Assets<StandardPbrMaterial>>,
    mut env_mats:  ResMut<Assets<LayeredEnvMaterial>>,
    mut status:    Query<&mut Text, With<DebugStatus>>,
) {
    let w_pressed = keys.just_pressed(KeyCode::KeyW);
    let v_pressed = keys.just_pressed(KeyCode::KeyV);
    if !w_pressed && !v_pressed {
        return;
    }

    if w_pressed {
        weather.preset_idx = (weather.preset_idx + 1) % WEATHER_PRESETS.len();
    }
    if v_pressed {
        weather.vcol_idx = (weather.vcol_idx + 1) % VCOL_DEBUG_MODES.len();
    }

    let (snow, dirt, wet, por, weather_name) = WEATHER_PRESETS[weather.preset_idx];
    let (tiling_y, vcol_name)               = VCOL_DEBUG_MODES[weather.vcol_idx];
    let weather_vec = Vec4::new(snow, dirt, wet, por);

    for (_, mat) in std_mats.iter_mut() {
        mat.extension.params.weather  = weather_vec;
        mat.extension.params.tiling.y = tiling_y;
    }
    for (_, mat) in env_mats.iter_mut() {
        mat.extension.params.weather  = weather_vec;
    }

    if let Ok(mut txt) = status.single_mut() {
        txt.0 = format!("Weather: {}  |  VCol: {}", weather_name, vcol_name);
    }
}
