use std::collections::{HashMap, HashSet};

use avian3d::prelude::{LinearVelocity, ShapeHits};
use bevy::prelude::*;
use bevy_gltf::GltfAssetLabel;
use core_resources::{
    AnimationState, EntityHandle, LocalEventBus, ModelAnimationRegistry, ModelName,
    ModelRegistry,
};
use core_shared::PlayerMarker;

use super::{LocalClientId, resolve_ped_profile_for_model};
use super::movement::has_ground_contact_with_thresholds;
use crate::config::ClientConfigResource;
use crate::drawable::{AdmSceneRoot, GltfHandleCache, PedPhysicsDef, PedPhysicsRegistry};
use crate::native_assets::{AdmHandleCache, PedAdsAnimHandleCache, PedAdsAnimIndex};

const ANIM_GROUND_CONTACT_DIST_ENTER: f32 = 0.07;
const ANIM_GROUND_CONTACT_DIST_EXIT: f32 = 0.14;
const ANIM_GROUNDED_GRACE_SECS: f32 = 0.18;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocomotionState {
    Idle,
    Walk,
    Run,
    Sprint,
}

impl Default for LocomotionState {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlayerAnimState {
    Idle,
    Walk,
    Run,
    Sprint,
    Jump,
    Fall,
    Land,
}

impl PlayerAnimState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Walk => "walk",
            Self::Run => "run",
            Self::Sprint => "sprint",
            Self::Jump => "jump",
            Self::Fall => "fall",
            Self::Land => "land",
        }
    }
}

impl Default for PlayerAnimState {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Component, Default)]
pub(super) struct AutoPlayerAnimMemory {
    was_grounded: bool,
    land_until: f32,
    airborne_since: Option<f32>,
    move_intent_until: f32,
    filtered_horiz_speed: f32,
    locomotion: LocomotionState,
    anim_state: PlayerAnimState,
}

#[derive(Resource, Default)]
pub(super) struct LuaAnimationGraphCache {
    map: HashMap<(String, u32), (Handle<AnimationGraph>, AnimationNodeIndex)>,
}

#[derive(Resource, Default)]
pub(super) struct ModelAnimationDiscoveryCache {
    adm: HashMap<String, Handle<crate::drawable::AdmScene>>,
    gltf: HashMap<String, Handle<bevy::gltf::Gltf>>,
}

pub(super) fn update_player_state_driven_animations(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    cfg: Res<ClientConfigResource>,
    ped_reg: Res<PedPhysicsRegistry>,
    ped_assets: Res<Assets<PedPhysicsDef>>,
    local_bus: Res<LocalEventBus>,
    local_client_id: Option<Res<LocalClientId>>,
    mut commands: Commands,
    mut model_roots: Query<
        (
            Entity,
            &AdmSceneRoot,
            &ModelName,
            &bevy::ecs::hierarchy::ChildOf,
            Option<&mut AnimationState>,
        ),
        With<AdmSceneRoot>,
    >,
    mut players: Query<
        (
            &PlayerMarker,
            Option<&EntityHandle>,
            &LinearVelocity,
            Option<&ShapeHits>,
            &mut AutoPlayerAnimMemory,
        ),
    >,
) {
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    let bindings = &cfg.0.input.keys;
    let has_move_input = keys.pressed(bindings.move_forward)
        || keys.pressed(bindings.move_backward)
        || keys.pressed(bindings.move_left)
        || keys.pressed(bindings.move_right);

    for (model_entity, _adm_root, model_name, child_of, anim_state) in &mut model_roots {
        let parent = child_of.parent();
        let Ok((marker, handle, vel, shape_hits, mut memory)) = players.get_mut(parent) else {
            continue;
        };

        let Some(ped) = resolve_ped_profile_for_model(&model_name.0, &ped_reg, &ped_assets) else {
            continue;
        };

        let raw_grounded = has_ground_contact_with_thresholds(
            shape_hits,
            memory.was_grounded,
            ANIM_GROUND_CONTACT_DIST_ENTER,
            ANIM_GROUND_CONTACT_DIST_EXIT,
        );
        if raw_grounded {
            memory.airborne_since = None;
        } else if memory.airborne_since.is_none() {
            memory.airborne_since = Some(now);
        }

        let grounded = raw_grounded
            || memory
                .airborne_since
                .map(|t| now - t <= ANIM_GROUNDED_GRACE_SECS)
                .unwrap_or(false);

        if grounded && !memory.was_grounded && vel.y < -0.10 {
            memory.land_until = now + 0.18;
        }
        memory.was_grounded = grounded;

        let horiz_speed = Vec2::new(vel.x, vel.z).length();
        let speed_alpha = (1.0 - (-10.0 * dt).exp()).clamp(0.0, 1.0);
        memory.filtered_horiz_speed += (horiz_speed - memory.filtered_horiz_speed) * speed_alpha;

        let idle_threshold = ped.movement.idle_threshold.max(0.01);
        let walk_threshold = (ped.movement.run_speed * 0.60).max(idle_threshold + 0.05);
        let sprint_threshold = (ped.movement.run_speed + ped.movement.sprint_speed) * 0.5;

        let is_local_player = local_client_id
            .as_ref()
            .map(|id| id.0 == marker.client_id)
            .unwrap_or(false);
        if is_local_player && has_move_input {
            memory.move_intent_until = now + 0.22;
        } else if is_local_player {
            memory.move_intent_until = now;
        }
        let movement_intent_active = is_local_player && now < memory.move_intent_until;

        let idle_exit = idle_threshold + 0.06;
        let walk_up = walk_threshold + 0.16;
        let walk_down = (walk_threshold - 0.16).max(idle_exit + 0.02);
        let sprint_up = sprint_threshold + 0.20;
        let sprint_down = (sprint_threshold - 0.20).max(walk_up + 0.02);

        if is_local_player && grounded && !has_move_input {
            memory.locomotion = LocomotionState::Idle;
            memory.filtered_horiz_speed = 0.0;
        } else {
            memory.locomotion = match memory.locomotion {
                LocomotionState::Idle => {
                    if memory.filtered_horiz_speed > idle_exit || (movement_intent_active && grounded)
                    {
                        LocomotionState::Walk
                    } else {
                        LocomotionState::Idle
                    }
                }
                LocomotionState::Walk => {
                    if memory.filtered_horiz_speed <= idle_threshold && !movement_intent_active {
                        LocomotionState::Idle
                    } else if memory.filtered_horiz_speed >= walk_up {
                        LocomotionState::Run
                    } else {
                        LocomotionState::Walk
                    }
                }
                LocomotionState::Run => {
                    if memory.filtered_horiz_speed <= walk_down {
                        LocomotionState::Walk
                    } else if memory.filtered_horiz_speed >= sprint_up {
                        LocomotionState::Sprint
                    } else {
                        LocomotionState::Run
                    }
                }
                LocomotionState::Sprint => {
                    if memory.filtered_horiz_speed <= sprint_down {
                        LocomotionState::Run
                    } else {
                        LocomotionState::Sprint
                    }
                }
            };
        }

        let (desired_clip, looping, next_anim_state) = if !grounded {
            if vel.y < -0.2 {
                (&ped.animations.fall_loop, true, PlayerAnimState::Fall)
            } else {
                (&ped.animations.jump_loop, true, PlayerAnimState::Jump)
            }
        } else if now < memory.land_until {
            (&ped.animations.land, false, PlayerAnimState::Land)
        } else {
            match memory.locomotion {
                LocomotionState::Idle if movement_intent_active => {
                    (&ped.animations.walk, true, PlayerAnimState::Walk)
                }
                LocomotionState::Idle => (&ped.animations.idle, true, PlayerAnimState::Idle),
                LocomotionState::Walk => (&ped.animations.walk, true, PlayerAnimState::Walk),
                LocomotionState::Run => (&ped.animations.run, true, PlayerAnimState::Run),
                LocomotionState::Sprint => (&ped.animations.sprint, true, PlayerAnimState::Sprint),
            }
        };

        if desired_clip.is_empty() {
            continue;
        }
        let clip = desired_clip.clone();

        let blend_time = match next_anim_state {
            PlayerAnimState::Land => 0.08,
            PlayerAnimState::Jump | PlayerAnimState::Fall => 0.10,
            _ => 0.16,
        };

        debug!(
            "[animation] player blend weights: locomotion={:?} speed={:.2} grounded={} move_intent={}",
            memory.locomotion, memory.filtered_horiz_speed, grounded, movement_intent_active
        );

        if memory.anim_state != next_anim_state {
            info!(
                "[animation] player state: {:?} -> {:?}",
                memory.anim_state, next_anim_state
            );
            let payload = serde_json::to_vec(&serde_json::json!({
                "client_id": marker.client_id,
                "handle": handle.map(|entity_handle: &EntityHandle| entity_handle.0),
                "state": next_anim_state.as_str(),
                "clip": clip,
                "grounded": grounded,
                "speed": memory.filtered_horiz_speed,
                "is_local": is_local_player,
                "move_intent": movement_intent_active,
            }))
            .unwrap_or_default();
            local_bus.push("player:anim_state".to_string(), payload);
            memory.anim_state = next_anim_state;
        }

        match anim_state {
            Some(mut state) => {
                if state.current.as_deref() != Some(clip.as_str()) || state.looping != looping {
                    state.current = Some(clip.clone());
                    state.looping = looping;
                    state.paused = false;
                    state.speed = 1.0;
                    state.blend_time = blend_time;
                    state.flags = 1;
                }
            }
            None => {
                debug!("[animation] npc entity={:?} clip={:?}", model_entity, clip);
                commands.entity(model_entity).insert(AnimationState {
                    current: Some(clip),
                    speed: 1.0,
                    looping,
                    paused: false,
                    blend_time,
                    flags: 1,
                });
            }
        }
    }
}

fn parse_animation_index(selector: &str) -> Option<u32> {
    if let Ok(idx) = selector.parse::<u32>() {
        return Some(idx);
    }

    if let Some(rest) = selector.strip_prefix("clip:") {
        return rest.parse::<u32>().ok();
    }
    if let Some(rest) = selector.strip_prefix("anim:") {
        return rest.parse::<u32>().ok();
    }

    None
}

fn extract_gltf_clip_names(gltf: &bevy::gltf::Gltf) -> Vec<String> {
    let Some(source) = gltf.source.as_ref() else {
        return Vec::new();
    };

    let mut names = Vec::new();
    for anim in source.animations() {
        if let Some(name) = anim.name() {
            names.push(name.to_string());
        } else {
            names.push(format!("clip:{}", anim.index()));
        }
    }

    names
}

pub(super) fn update_model_animation_registry(
    model_registry: Res<ModelRegistry>,
    anim_registry: Res<ModelAnimationRegistry>,
    asset_server: Res<AssetServer>,
    adm_cache: Res<AdmHandleCache>,
    gltf_cache: Res<GltfHandleCache>,
    adm_assets: Res<Assets<crate::drawable::AdmScene>>,
    gltf_assets: Res<Assets<bevy::gltf::Gltf>>,
    anim_set_assets: Res<Assets<crate::drawable::AnimationSet>>,
    ped_anim_index: Res<PedAdsAnimIndex>,
    ped_anim_handles: Res<PedAdsAnimHandleCache>,
    mut discovery_cache: ResMut<ModelAnimationDiscoveryCache>,
) {
    let mut keep = HashSet::new();

    for (model_name, model_path) in model_registry.entries() {
        keep.insert(model_name.clone());
        let ext = model_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();

        if ext == "adm" {
            let handle = if let Some(handle) = adm_cache.0.get(&model_name) {
                handle.clone()
            } else if let Some(handle) = discovery_cache.adm.get(&model_name) {
                handle.clone()
            } else {
                let bevy_path = model_path.to_string_lossy().replace('\\', "/");
                let handle: Handle<crate::drawable::AdmScene> = asset_server.load(bevy_path);
                discovery_cache.adm.insert(model_name.clone(), handle.clone());
                handle
            };

            if adm_assets.get(&handle).is_some() {
                let mut all_clip_names = Vec::new();
                let mut all_dicts: Vec<core_resources::ModelAnimationDictionary> = Vec::new();

                if let Some(anim_paths) = ped_anim_index.0.get(&model_name) {
                    for path in anim_paths {
                        if let Some(handle) = ped_anim_handles.0.get(path) {
                            if let Some(anim_set) = anim_set_assets.get(handle) {
                                for clip in &anim_set.clips {
                                    if !all_clip_names.contains(&clip.name) {
                                        all_clip_names.push(clip.name.clone());
                                    }
                                }

                                for dict_set in &anim_set.dictionaries {
                                    let mut found = false;
                                    for existing in &mut all_dicts {
                                        if existing.name == dict_set.name {
                                            for clip_name in &dict_set.clip_names {
                                                if !existing.clip_names.contains(clip_name) {
                                                    existing.clip_names.push(clip_name.clone());
                                                }
                                            }
                                            found = true;
                                            break;
                                        }
                                    }
                                    if !found {
                                        all_dicts.push(core_resources::ModelAnimationDictionary {
                                            name: dict_set.name.clone(),
                                            clip_names: dict_set.clip_names.clone(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }

                anim_registry.set_clip_names(&model_name, all_clip_names);
                anim_registry.set_animation_dictionaries(&model_name, all_dicts);
            }
            continue;
        }

        if ext == "glb" || ext == "gltf" {
            let handle: Handle<bevy::gltf::Gltf> = if let Some((handle, _)) = gltf_cache.0.get(&model_name) {
                handle.clone()
            } else if let Some(handle) = discovery_cache.gltf.get(&model_name) {
                handle.clone()
            } else {
                let bevy_path = model_path.to_string_lossy().replace('\\', "/");
                let handle = asset_server.load_with_settings(
                    bevy_path,
                    |settings: &mut bevy::gltf::GltfLoaderSettings| {
                        settings.include_source = true;
                    },
                );
                discovery_cache.gltf.insert(model_name.clone(), handle.clone());
                handle
            };

            if let Some(gltf) = gltf_assets.get(&handle) {
                anim_registry.set_clip_names(&model_name, extract_gltf_clip_names(gltf));
            }
        }
    }

    discovery_cache.adm.retain(|name, _| keep.contains(name));
    discovery_cache.gltf.retain(|name, _| keep.contains(name));
    anim_registry.retain_models(&keep);
}

fn resolve_animation_graph(
    model_name: &str,
    animation_selector: &str,
    model_registry: &ModelRegistry,
    asset_server: &AssetServer,
    graphs: &mut Assets<AnimationGraph>,
    cache: &mut LuaAnimationGraphCache,
) -> Option<(Handle<AnimationGraph>, AnimationNodeIndex)> {
    let clip_index = parse_animation_index(animation_selector)?;

    let key = (model_name.to_string(), clip_index);
    if let Some(cached) = cache.map.get(&key) {
        return Some(cached.clone());
    }

    let model_path = model_registry.path(model_name)?;
    let ext = model_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    if ext != "glb" && ext != "gltf" {
        return None;
    }

    let model_asset = model_path.to_string_lossy().replace('\\', "/");
    let clip = asset_server.load(GltfAssetLabel::Animation(clip_index as usize).from_asset(model_asset));
    let (graph, node) = AnimationGraph::from_clip(clip);
    let graph_handle = graphs.add(graph);
    cache.map.insert(key, (graph_handle.clone(), node));
    Some((graph_handle, node))
}

pub(super) fn apply_lua_animation_state(
    mut commands: Commands,
    model_registry: Res<ModelRegistry>,
    asset_server: Res<AssetServer>,
    mut graph_assets: ResMut<Assets<AnimationGraph>>,
    mut graph_cache: ResMut<LuaAnimationGraphCache>,
    anim_roots: Query<(Entity, &AnimationState, &ModelName)>,
    children_q: Query<&Children>,
    mut players: Query<(Entity, &mut AnimationPlayer, Option<&AnimationGraphHandle>)>,
) {
    for (root, state, model_name) in &anim_roots {
        let Some(clip_name) = state.current.as_ref() else {
            continue;
        };

        let Some((graph_handle, node)) = resolve_animation_graph(
            &model_name.0,
            clip_name,
            &model_registry,
            &asset_server,
            &mut graph_assets,
            &mut graph_cache,
        ) else {
            continue;
        };

        debug!(
            "[animation] lua anim entity={:?} clip={:?} blend={:.2}",
            root, clip_name, state.blend_time
        );

        for entity in children_q.iter_descendants(root) {
            let Ok((player_entity, mut player, graph_handle_comp)) = players.get_mut(entity) else {
                continue;
            };

            if graph_handle_comp.map(|handle| handle.0 != graph_handle).unwrap_or(true) {
                info!("[animation] lua override applied: {:?}", clip_name);
                commands
                    .entity(player_entity)
                    .insert(AnimationGraphHandle(graph_handle.clone()));
            }

            let playing = player.play(node);
            if state.looping {
                playing.repeat();
            }

            if let Some(active) = player.animation_mut(node) {
                active.set_speed(state.speed);
                if state.paused {
                    active.pause();
                } else {
                    active.resume();
                }
            }

        }
    }
}