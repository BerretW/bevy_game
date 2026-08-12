use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use core_drawable::{AdmSceneRoot, AnimationSet};
use core_resources::{AnimationState, AttachedAnimSets, ModelName};

use crate::state::{ADM_ANIM_MASK_ALL, ADM_ANIM_MASK_LEFT_UPPER_LIMB, ADM_ANIM_MASK_LOWER_BODY, ADM_ANIM_MASK_RIGHT_UPPER_LIMB, ADM_ANIM_MASK_UPPER_BODY, AdmAnimationBrowser, AnimDebugPanel, AnimDebugState, AnimPanel, ViewerSessionAnimHandles};

pub(crate) fn handle_animation_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut browser: ResMut<AdmAnimationBrowser>,
) {
    if browser.root.is_none() {
        return;
    }

    if !browser.dictionaries.is_empty() {
        if keys.just_pressed(KeyCode::PageUp) {
            browser.selected_dict_idx = if browser.selected_dict_idx == 0 {
                browser.dictionaries.len().saturating_sub(1)
            } else {
                browser.selected_dict_idx.saturating_sub(1)
            };
            browser.selected_idx = 0;
        }
        if keys.just_pressed(KeyCode::PageDown) {
            browser.selected_dict_idx = (browser.selected_dict_idx + 1) % browser.dictionaries.len();
            browser.selected_idx = 0;
        }
    }

    if browser.clips.is_empty() {
        return;
    }

    if keys.just_pressed(KeyCode::Space) {
        browser.paused = !browser.paused;
    }
    if keys.just_pressed(KeyCode::KeyN) || keys.just_pressed(KeyCode::ArrowRight) {
        browser.selected_idx = (browser.selected_idx + 1) % browser.clips.len();
    }
    if keys.just_pressed(KeyCode::KeyB) || keys.just_pressed(KeyCode::ArrowLeft) {
        browser.selected_idx = if browser.selected_idx == 0 {
            browser.clips.len().saturating_sub(1)
        } else {
            browser.selected_idx.saturating_sub(1)
        };
    }

    if keys.just_pressed(KeyCode::Digit1) {
        browser.flags = ADM_ANIM_MASK_ALL;
    } else if keys.just_pressed(KeyCode::Digit2) {
        browser.flags = ADM_ANIM_MASK_LOWER_BODY;
    } else if keys.just_pressed(KeyCode::Digit3) {
        browser.flags = ADM_ANIM_MASK_UPPER_BODY;
    } else if keys.just_pressed(KeyCode::Digit4) {
        browser.flags = ADM_ANIM_MASK_RIGHT_UPPER_LIMB;
    } else if keys.just_pressed(KeyCode::Digit5) {
        browser.flags = ADM_ANIM_MASK_LEFT_UPPER_LIMB;
    } else if keys.just_pressed(KeyCode::Digit6) {
        browser.flags = ADM_ANIM_MASK_LOWER_BODY | ADM_ANIM_MASK_RIGHT_UPPER_LIMB;
    }
}

fn selected_dict_name(browser: &AdmAnimationBrowser) -> Option<&str> {
    browser.dictionaries.get(browser.selected_dict_idx).map(String::as_str)
}

fn normalize_anim_set_path(path: &str) -> String {
    if path.ends_with(".ads_anim") {
        path.to_string()
    } else {
        format!("{path}.ads_anim")
    }
}

fn collect_anim_set_data(
    attached_sets: Option<&AttachedAnimSets>,
    asset_server: &AssetServer,
    anim_set_assets: &Assets<AnimationSet>,
    anim_handles: &ViewerSessionAnimHandles,
) -> (
    Vec<String>,
    Vec<String>,
    HashMap<String, Vec<String>>,
    HashMap<String, Vec<(f32, String)>>,
) {
    let mut all_clips: Vec<String> = Vec::new();
    let mut dictionaries: Vec<String> = Vec::new();
    let mut dict_to_clips: HashMap<String, Vec<String>> = HashMap::new();
    let mut notifies_by_clip: HashMap<String, Vec<(f32, String)>> = HashMap::new();
    let mut seen_clips: HashSet<String> = HashSet::new();

    let Some(attached_sets) = attached_sets else {
        return (all_clips, dictionaries, dict_to_clips, notifies_by_clip);
    };

    for set_path in &attached_sets.sets {
        let normalized = normalize_anim_set_path(set_path);
        let handle: Handle<AnimationSet> = anim_handles
            .handles
            .get(&normalized)
            .cloned()
            .unwrap_or_else(|| asset_server.load(normalized));
        let Some(anim_set) = anim_set_assets.get(&handle) else { continue };

        for clip in &anim_set.clips {
            if seen_clips.insert(clip.name.clone()) {
                all_clips.push(clip.name.clone());
                notifies_by_clip.insert(
                    clip.name.clone(),
                    clip.notifies
                        .iter()
                        .map(|n| (n.time, n.name.clone()))
                        .collect(),
                );
            }
        }

        for dict in &anim_set.dictionaries {
            if !dictionaries.iter().any(|name| name == &dict.name) {
                dictionaries.push(dict.name.clone());
            }
            let entry = dict_to_clips.entry(dict.name.clone()).or_default();
            for clip_name in &dict.clip_names {
                if !entry.iter().any(|name| name == clip_name) {
                    entry.push(clip_name.clone());
                }
            }
        }
    }

    (all_clips, dictionaries, dict_to_clips, notifies_by_clip)
}

pub(crate) fn sync_adm_animation_browser(
    mut browser: ResMut<AdmAnimationBrowser>,
    roots: Query<(Entity, &ModelName, Option<&AttachedAnimSets>), With<AnimationState>>,
    asset_server: Res<AssetServer>,
    anim_set_assets: Res<Assets<AnimationSet>>,
    anim_handles: Res<ViewerSessionAnimHandles>,
) {
    if let Some(root) = browser.root {
        if roots.get(root).is_err() {
            browser.root = None;
            browser.clips.clear();
        }
    }

    if browser.root.is_none() {
        for (entity, model_name, attached_sets) in &roots {
            let (all_clips, dictionaries, dict_to_clips, notifies_by_clip) = collect_anim_set_data(
                attached_sets,
                &asset_server,
                &anim_set_assets,
                &anim_handles,
            );
            if all_clips.is_empty() {
                continue;
            }

            browser.root = Some(entity);
            browser.model_name = Some(model_name.0.clone());
            browser.dictionaries = dictionaries;
            if browser.selected_dict_idx >= browser.dictionaries.len() {
                browser.selected_dict_idx = 0;
            }
            browser.clips = if let Some(dict_name) = selected_dict_name(&browser) {
                dict_to_clips.get(dict_name).cloned().unwrap_or_else(|| all_clips.clone())
            } else {
                all_clips
            };
            browser.selected_idx = 0;
            browser.flags = ADM_ANIM_MASK_ALL;
            browser.speed = 1.0;
            browser.looping = true;
            browser.paused = false;
            browser.notifies = browser
                .clips
                .get(browser.selected_idx)
                .and_then(|name| notifies_by_clip.get(name))
                .cloned()
                .unwrap_or_default();
            break;
        }
    }

    if let Some(root) = browser.root {
        if let Ok((_, model_name, attached_sets)) = roots.get(root) {
            browser.model_name = Some(model_name.0.clone());

            let (all_clips, dictionaries, dict_to_clips, notifies_by_clip) = collect_anim_set_data(
                attached_sets,
                &asset_server,
                &anim_set_assets,
                &anim_handles,
            );

            if dictionaries != browser.dictionaries {
                browser.dictionaries = dictionaries;
                if browser.selected_dict_idx >= browser.dictionaries.len() {
                    browser.selected_dict_idx = 0;
                }
                browser.selected_idx = 0;
            }

            let clips = if let Some(dict_name) = selected_dict_name(&browser) {
                dict_to_clips.get(dict_name).cloned().unwrap_or_else(|| all_clips.clone())
            } else {
                all_clips
            };

            if clips != browser.clips {
                browser.clips = clips;
                if browser.selected_idx >= browser.clips.len() {
                    browser.selected_idx = 0;
                }
            }

            browser.notifies = browser
                .clips
                .get(browser.selected_idx)
                .and_then(|name| notifies_by_clip.get(name))
                .cloned()
                .unwrap_or_default();
        }
    }
}

pub(crate) fn apply_animation_browser_state(
    browser: Res<AdmAnimationBrowser>,
    mut roots: Query<&mut AnimationState, With<AdmSceneRoot>>,
) {
    let Some(root) = browser.root else { return };
    let Some(clip_name) = browser.clips.get(browser.selected_idx).cloned() else { return };
    let Ok(mut anim_state) = roots.get_mut(root) else { return };

    anim_state.current = Some(clip_name);
    anim_state.flags = browser.flags;
    anim_state.speed = browser.speed;
    anim_state.looping = browser.looping;
    anim_state.paused = browser.paused;
}

pub(crate) fn update_animation_overlay(
    browser: Res<AdmAnimationBrowser>,
    roots: Query<(Entity, &ModelName, Option<&AttachedAnimSets>), With<AnimationState>>,
    asset_server: Res<AssetServer>,
    anim_set_assets: Res<Assets<AnimationSet>>,
    anim_handles: Res<ViewerSessionAnimHandles>,
    mut panel: Query<(&mut Text, &mut Visibility), With<AnimPanel>>,
) {
    let Ok((mut txt, mut vis)) = panel.single_mut() else { return };

    let mut runtime_lines: Vec<String> = Vec::new();
    runtime_lines.push(format!("Runtime roots with AnimationState: {}", roots.iter().count()));

    let mut roots_with_sets = 0usize;
    let mut roots_with_clips = 0usize;
    for (entity, model_name, attached_sets) in &roots {
        if let Some(attached_sets) = attached_sets {
            roots_with_sets += 1;
            let (clips, dicts, _, _) = collect_anim_set_data(
                Some(attached_sets),
                &asset_server,
                &anim_set_assets,
                &anim_handles,
            );
            if !clips.is_empty() {
                roots_with_clips += 1;
                runtime_lines.push(format!(
                    "  root {:?} model:{} sets:{} clips:{} dicts:{}",
                    entity,
                    model_name.0,
                    attached_sets.sets.len(),
                    clips.len(),
                    dicts.len()
                ));
            } else {
                runtime_lines.push(format!(
                    "  root {:?} model:{} sets:{} clips:0 (asset not ready or parse failed)",
                    entity,
                    model_name.0,
                    attached_sets.sets.len()
                ));
            }
        } else {
            runtime_lines.push(format!("  root {:?} model:{} sets:0", entity, model_name.0));
        }
    }

    if browser.root.is_none() {
        *vis = Visibility::Visible;
        txt.0 = [
            "── Animace ──",
            "Žádné aktivní animace.",
            "Nahraj .adm a .ads_anim (s klipy).",
            "Ovládání: N/→ další clip, B/← předchozí, Space pauza.",
            "PageUp/PageDown přepíná dictionary.",
            &format!("Roots s AttachedAnimSets: {}", roots_with_sets),
            &format!("Roots s načtenými klipy: {}", roots_with_clips),
        ]
        .join("\n");
        return;
    }

    *vis = Visibility::Visible;
    let model_name = browser.model_name.as_deref().unwrap_or("ADM");
    let dict_name = selected_dict_name(&browser).unwrap_or("ALL");
    let dict_total = browser.dictionaries.len().max(1);
    let clip_total = browser.clips.len().max(1);
    let clip_name = browser.clips.get(browser.selected_idx).map(|s| s.as_str()).unwrap_or("-");
    let mut lines = vec![
        format!("── Animace ({}) ──", model_name),
        format!("Dict: {}  [{}/{}]", dict_name, browser.selected_dict_idx + 1, dict_total),
        format!("Clip: {}  [{}/{}]", clip_name, browser.selected_idx + 1, clip_total),
        format!("Flags: {}  Speed: {:.2}  Loop: {}  Paused: {}", browser.flags, browser.speed, browser.looping, browser.paused),
        "PageUp/PageDown dict | N/→ next clip | B/← prev clip | Space pause".to_string(),
        "1 all | 2 lower body | 3 upper body | 4 right arm | 5 left arm | 6 ride+smoke".to_string(),
    ];

    if !browser.dictionaries.is_empty() {
        lines.push("Dicty:".to_string());
        for (idx, name) in browser.dictionaries.iter().enumerate() {
            let marker = if idx == browser.selected_dict_idx { ">" } else { " " };
            lines.push(format!("{} {}", marker, name));
        }
    }

    if browser.notifies.is_empty() {
        lines.push("Notifies: žádné (ADS_ANIM bez markerů)".to_string());
    } else {
        lines.push(format!("Notifies: {}x", browser.notifies.len()));
        for (t, name) in &browser.notifies {
            lines.push(format!("  {:.3}s  {}", t, name));
        }
    }

    if !browser.clips.is_empty() {
        lines.push("Clipy:".to_string());
        for (idx, name) in browser.clips.iter().enumerate() {
            let notify_badge = {
                if idx == browser.selected_idx && !browser.notifies.is_empty() {
                    format!(" [{}✱]", browser.notifies.len())
                } else {
                    String::new()
                }
            };
            let marker = if idx == browser.selected_idx { ">" } else { " " };
            lines.push(format!("{} {}{}", marker, name, notify_badge));
        }
    }

    lines.push(String::new());
    lines.push("Runtime debug:".to_string());
    lines.extend(runtime_lines);

    txt.0 = lines.join("\n");
}

pub(crate) fn update_anim_debug_overlay(
    debug: Res<AnimDebugState>,
    mut panel: Query<(&mut Text, &mut Visibility), With<AnimDebugPanel>>,
) {
    let Ok((mut txt, mut vis)) = panel.single_mut() else { return };

    if debug.text.trim().is_empty() {
        *vis = Visibility::Hidden;
        txt.0.clear();
        return;
    }

    *vis = Visibility::Visible;
    txt.0 = debug.text.clone();
}
