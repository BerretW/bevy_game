use std::fs;
use std::io::{Cursor, Read};
use std::path::PathBuf;

use serde::Deserialize;

use bevy::gltf::GltfLoaderSettings;
use bevy::prelude::*;
use rfd::FileDialog;

use core_drawable::{AdmScene, AdmSceneRoot, AnimationSet, DrawableManifestRegistry};
use core_resources::{AnimationState, AttachedAnimSets, ModelName, ModelRegistry};

use crate::state::{ActiveViewerModelRoot, AnimDebugState, GltfHandleCache, ModelPaths, ModelSourcePaths, UiDiagAnimButton, UiLoadAdmButton, UiLoadAnimButton, ViewerIkChainDef, ViewerIkChains, ViewerSessionAnimHandles};

// ─── IK sidecar parser ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct IkTomlFile {
    #[serde(default)]
    chains: Vec<IkTomlChain>,
}

#[derive(Deserialize)]
struct IkTomlChain {
    #[serde(default)]
    name: String,
    #[serde(default = "ik_default_enabled")]
    enabled: bool,
    #[serde(default)]
    parent_bone: String,
    #[serde(default)]
    ik_target: String,
    #[serde(default)]
    effector_bone: String,
    #[serde(default)]
    pole_bone: Option<String>,
}

fn ik_default_enabled() -> bool { true }

fn load_ik_sidecar(path: &PathBuf, ik_chains: &mut ViewerIkChains) {
    let ik_path = path.with_extension("ik.toml");
    if !ik_path.exists() { return; }
    let content = match fs::read_to_string(&ik_path) {
        Ok(s) => s,
        Err(e) => { warn!("[viewer] IK toml read error {}: {}", ik_path.display(), e); return; }
    };
    let parsed: IkTomlFile = match toml::from_str(&content) {
        Ok(v) => v,
        Err(e) => { warn!("[viewer] IK toml parse error {}: {}", ik_path.display(), e); return; }
    };
    let mut added = 0usize;
    for c in parsed.chains {
        if c.parent_bone.is_empty() || c.ik_target.is_empty() || c.effector_bone.is_empty() {
            continue;
        }
        let pole = c.pole_bone.filter(|s| !s.is_empty());
        ik_chains.chains.push(ViewerIkChainDef {
            name: c.name,
            enabled: c.enabled,
            parent_bone: c.parent_bone,
            ik_target: c.ik_target,
            effector_bone: c.effector_bone,
            pole_bone: pole,
        });
        added += 1;
    }
    if added > 0 {
        info!("[viewer] IK sidecar: {} chain(s) from {}", added, ik_path.file_name().unwrap_or_default().to_string_lossy());
    }
}

// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn handle_loader_buttons(
    adm_q: Query<&Interaction, (Changed<Interaction>, With<UiLoadAdmButton>)>,
    anim_q: Query<&Interaction, (Changed<Interaction>, With<UiLoadAnimButton>)>,
    diag_q: Query<&Interaction, (Changed<Interaction>, With<UiDiagAnimButton>)>,
    asset_server: Res<AssetServer>,
    mut drawable_reg: ResMut<DrawableManifestRegistry>,
    mut gltf_cache: ResMut<GltfHandleCache>,
    mut model_reg: ResMut<ModelRegistry>,
    mut anim_handles: ResMut<ViewerSessionAnimHandles>,
    mut model_source_paths: ResMut<ModelSourcePaths>,
    mut ik_chains: ResMut<ViewerIkChains>,
    mut commands: Commands,
    roots: Query<Entity, With<AdmSceneRoot>>,
    mut query_anim_roots: Query<&mut AttachedAnimSets>,
    mut anim_debug: ResMut<AnimDebugState>,
    mut active_root: ResMut<ActiveViewerModelRoot>,
) {
    for interaction in &adm_q {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let Some(path) = FileDialog::new()
            .add_filter("Model", &["adm", "glb", "gltf"])
            .pick_file()
        else {
            continue;
        };

        let mut model_paths = Vec::new();
        model_paths.push(path);
        for path in model_paths {
            load_one_model(
                &path,
                &asset_server,
                &mut drawable_reg,
                &mut gltf_cache,
                &mut model_reg,
                &mut anim_handles,
                &mut model_source_paths,
                &mut ik_chains,
                &mut active_root,
                &mut commands,
                &[],
            );
        }
    }

    for interaction in &anim_q {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let Some(files) = FileDialog::new()
            .add_filter("ADS Anim", &["ads_anim"])
            .pick_files()
        else {
            continue;
        };

        let mut added_paths: Vec<String> = Vec::new();
        for path in files {
            if let Some(anim_path) = canonical_bevy_path(&path) {
                added_paths.push(anim_path.clone());
                let handle: Handle<AnimationSet> = asset_server.load(anim_path.clone());
                anim_handles.handles.insert(anim_path, handle);
            }
        }

        if added_paths.is_empty() {
            continue;
        }

        for entity in &roots {
            if let Ok(mut attached) = query_anim_roots.get_mut(entity) {
                for path in &added_paths {
                    if !attached.sets.iter().any(|p| p == path) {
                        attached.sets.push(path.clone());
                    }
                }
            } else {
                commands.entity(entity).insert(AttachedAnimSets {
                    sets: added_paths.clone(),
                });
            }
        }
    }

    for interaction in &diag_q {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let Some(path) = FileDialog::new()
            .add_filter("ADS Anim", &["ads_anim"])
            .pick_file()
        else {
            continue;
        };

        anim_debug.text = diagnose_anim_set_file(&path);
        if let Some(first) = anim_debug.text.lines().next() {
            info!("[viewer] {}", first);
        }
    }
}

pub(crate) fn load_models(
    paths: Res<ModelPaths>,
    asset_server: Res<AssetServer>,
    mut drawable_reg: ResMut<DrawableManifestRegistry>,
    mut gltf_cache: ResMut<GltfHandleCache>,
    mut model_reg: ResMut<ModelRegistry>,
    mut anim_handles: ResMut<ViewerSessionAnimHandles>,
    mut model_source_paths: ResMut<ModelSourcePaths>,
    mut ik_chains: ResMut<ViewerIkChains>,
    mut active_root: ResMut<ActiveViewerModelRoot>,
    mut commands: Commands,
    mut overlay: Query<&mut Text, With<crate::state::InfoOverlay>>,
) {
    let mut model_paths: Vec<PathBuf> = Vec::new();
    let mut explicit_anim_sets: Vec<String> = Vec::new();

    for path in &paths.0 {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        if ext == "ads_anim" {
            if let Some(bevy_path) = canonical_bevy_path(path) {
                if !explicit_anim_sets.iter().any(|p| p == &bevy_path) {
                    let handle: Handle<AnimationSet> = asset_server.load(bevy_path.clone());
                    anim_handles.handles.insert(bevy_path.clone(), handle);
                    explicit_anim_sets.push(bevy_path);
                }
            }
            continue;
        }
        model_paths.push(path.clone());
    }

    if model_paths.is_empty() {
        if let Ok(mut txt) = overlay.single_mut() {
            txt.0 = "Žádný model.\nSpusť: model_viewer cesta/k/modelu.adm [cesta/k/setu.ads_anim]".to_string();
        }
        return;
    }

    for path in &model_paths {
        load_one_model(
            path,
            &asset_server,
            &mut drawable_reg,
            &mut gltf_cache,
            &mut model_reg,
            &mut anim_handles,
            &mut model_source_paths,
            &mut ik_chains,
            &mut active_root,
            &mut commands,
            &explicit_anim_sets,
        );
    }

    if let Ok(mut txt) = overlay.single_mut() {
        let names: Vec<_> = model_paths
            .iter()
            .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(|s| s.to_string()))
            .collect();
        if explicit_anim_sets.is_empty() {
            txt.0 = format!("Modely: {}", names.join(", "));
        } else {
            txt.0 = format!("Modely: {}\nAnimSety: {}", names.join(", "), explicit_anim_sets.len());
        }
    }
}

pub(crate) fn load_one_model(
    path: &PathBuf,
    asset_server: &AssetServer,
    drawable_reg: &mut DrawableManifestRegistry,
    gltf_cache: &mut GltfHandleCache,
    model_reg: &mut ModelRegistry,
    anim_handles: &mut ViewerSessionAnimHandles,
    model_source_paths: &mut ModelSourcePaths,
    ik_chains: &mut ViewerIkChains,
    active_root: &mut ActiveViewerModelRoot,
    commands: &mut Commands,
    explicit_anim_sets: &[String],
) {
    let path = match path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            warn!("[viewer] nelze zpřesnit cestu {:?}: {}", path, e);
            return;
        }
    };

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("model")
        .to_string();

    let bevy_path = path.to_string_lossy().replace('\\', "/");

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();

    if ext == "adm" {
        let adm_handle: Handle<AdmScene> = asset_server.load(bevy_path.clone());
        model_reg.register_native(stem.clone(), bevy_path.clone());
        model_source_paths.0.insert(stem.clone(), path.clone());

        let mut attached_sets: Vec<String> = explicit_anim_sets.to_vec();

        let sibling_anim = path.with_extension("ads_anim");
        if sibling_anim.exists() {
            if let Some(anim_path) = canonical_bevy_path(&sibling_anim) {
                if !attached_sets.iter().any(|p| p == &anim_path) {
                    attached_sets.push(anim_path);
                }
            }
        }

        for set_path in &attached_sets {
            let handle: Handle<AnimationSet> = asset_server.load(set_path.clone());
            anim_handles.handles.insert(set_path.clone(), handle);
        }

        let drawable_path = path.with_extension("drawable");
        if drawable_path.exists() {
            let dp = drawable_path.to_string_lossy().replace('\\', "/");
            let handle = asset_server.load(dp.clone());
            drawable_reg.0.insert(stem.clone(), handle);
            info!("[viewer] drawable manifest: '{}'", dp);
        } else {
            info!("[viewer] žádný .drawable pro '{}', použijí se výchozí materiály", stem);
        }

        if attached_sets.is_empty() {
            let entity = commands
                .spawn((AdmSceneRoot(adm_handle), Transform::default(), ModelName(stem.clone()), AnimationState::default()))
                .id();
            active_root.root = Some(entity);
            active_root.model_name = Some(stem);
        } else {
            let entity = commands
                .spawn((
                    AdmSceneRoot(adm_handle),
                    Transform::default(),
                    ModelName(stem.clone()),
                    AnimationState::default(),
                    AttachedAnimSets { sets: attached_sets },
                ))
                .id();
            active_root.root = Some(entity);
            active_root.model_name = Some(stem);
        }
        load_ik_sidecar(&path, ik_chains);
    } else {
        let gltf_handle: Handle<bevy::gltf::Gltf> = asset_server.load_with_settings(
            bevy_path.clone(),
            |s: &mut GltfLoaderSettings| {
                s.include_source = true;
            },
        );
        gltf_cache.0.insert(stem.clone(), (gltf_handle, bevy_path.clone()));
        model_reg.register_native(stem.clone(), bevy_path.clone());

        let drawable_path = path.with_extension("drawable");
        if drawable_path.exists() {
            let dp = drawable_path.to_string_lossy().replace('\\', "/");
            let handle = asset_server.load(dp.clone());
            drawable_reg.0.insert(stem.clone(), handle);
            info!("[viewer] drawable manifest: '{}'", dp);
        } else {
            info!("[viewer] žádný .drawable pro '{}', použijí se výchozí materiály", stem);
        }

        let scene_path = format!("{}#Scene0", bevy_path);
        let scene_handle: Handle<Scene> = asset_server.load(scene_path);

        let entity = commands
            .spawn((SceneRoot(scene_handle), Transform::default(), ModelName(stem.clone())))
            .id();
        active_root.root = Some(entity);
        active_root.model_name = Some(stem);
        load_ik_sidecar(&path, ik_chains);
    }
}

pub(crate) fn canonical_bevy_path(path: &PathBuf) -> Option<String> {
    path.canonicalize()
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
}

fn diagnose_anim_set_file(path: &PathBuf) -> String {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            return format!("ADS diag: {}\n  read error: {}", path.display(), err);
        }
    };

    let mut cur = Cursor::new(bytes.as_slice());
    let magic = match read_u8_vec_local(&mut cur, 4) {
        Ok(magic) => magic,
        Err(err) => return format!("ADS diag: {}\n  read error: {:?}", path.display(), err),
    };

    let version = match read_u32_local(&mut cur) {
        Ok(v) => v,
        Err(err) => return format!("ADS diag: {}\n  bad header: {:?}", path.display(), err),
    };

    let mut lines = vec![format!("ADS diag: {}", path.display())];
    lines.push(format!("  size: {} bytes", bytes.len()));
    lines.push(format!("  magic: {:?}", String::from_utf8_lossy(&magic)));
    lines.push(format!("  version: {}", version));

    if magic.as_slice() != b"ADS\0" {
        lines.push("  status: invalid magic, expected ADS\0".to_string());
        return lines.join("\n");
    }

    if version != 1 {
        lines.push("  status: unsupported version, expected 1".to_string());
        return lines.join("\n");
    }

    match parse_anim_set_debug(&mut cur, bytes.len()) {
        Ok(report) => {
            lines.extend(report);
        }
        Err(err) => {
            lines.push(format!("  parse error: {:?}", err));
        }
    }

    lines.join("\n")
}

fn parse_anim_set_debug(cur: &mut Cursor<&[u8]>, total_len: usize) -> Result<Vec<String>, AnimDiagError> {
    let clip_count = read_u32_local(cur)? as usize;
    let mut clip_lines = vec![format!("  clips: {}", clip_count)];
    let mut clip_names = Vec::with_capacity(clip_count);

    for clip_idx in 0..clip_count {
        let name = read_string_local(cur)?;
        let duration = read_f32_local(cur)?;
        let track_count = read_u32_local(cur)? as usize;
        clip_names.push(name.clone());
        clip_lines.push(format!("    [{}] {}  duration:{:.3}s  tracks:{}", clip_idx, name, duration, track_count));

        for _ in 0..track_count {
            let _node_name = read_string_local(cur)?;
            let key_count = read_u32_local(cur)? as usize;

            for _ in 0..key_count {
                skip_f32_local(cur)?;
                skip_f32_local(cur)?;
                skip_f32_local(cur)?;
                skip_f32_local(cur)?;
                skip_f32_local(cur)?;
                skip_f32_local(cur)?;
                skip_f32_local(cur)?;
                skip_f32_local(cur)?;
                skip_f32_local(cur)?;
                skip_f32_local(cur)?;
            }
        }
    }

    let remaining = total_len.saturating_sub(cur.position() as usize);
    clip_lines.push(format!("  remaining bytes after clips: {}", remaining));
    if !clip_names.is_empty() {
        clip_lines.push(format!("  clip names: {}", clip_names.join(", ")));
    }

    Ok(clip_lines)
}

#[derive(Debug)]
enum AnimDiagError {
    Utf8,
    UnexpectedEof,
}

fn read_u32_local(cur: &mut Cursor<&[u8]>) -> Result<u32, AnimDiagError> {
    let mut buf = [0u8; 4];
    cur.read_exact(&mut buf).map_err(|_| AnimDiagError::UnexpectedEof)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_f32_local(cur: &mut Cursor<&[u8]>) -> Result<f32, AnimDiagError> {
    let mut buf = [0u8; 4];
    cur.read_exact(&mut buf).map_err(|_| AnimDiagError::UnexpectedEof)?;
    Ok(f32::from_le_bytes(buf))
}

fn skip_f32_local(cur: &mut Cursor<&[u8]>) -> Result<(), AnimDiagError> {
    let mut buf = [0u8; 4];
    cur.read_exact(&mut buf).map_err(|_| AnimDiagError::UnexpectedEof)?;
    Ok(())
}

fn read_u16_local(cur: &mut Cursor<&[u8]>) -> Result<u16, AnimDiagError> {
    let mut buf = [0u8; 2];
    cur.read_exact(&mut buf).map_err(|_| AnimDiagError::UnexpectedEof)?;
    Ok(u16::from_le_bytes(buf))
}

fn read_string_local(cur: &mut Cursor<&[u8]>) -> Result<String, AnimDiagError> {
    let len = read_u16_local(cur)? as usize;
    let mut buf = vec![0u8; len];
    cur.read_exact(&mut buf).map_err(|_| AnimDiagError::UnexpectedEof)?;
    String::from_utf8(buf).map_err(|_| AnimDiagError::Utf8)
}

fn read_u8_vec_local(cur: &mut Cursor<&[u8]>, len: usize) -> Result<Vec<u8>, AnimDiagError> {
    let mut buf = vec![0u8; len];
    cur.read_exact(&mut buf).map_err(|_| AnimDiagError::UnexpectedEof)?;
    Ok(buf)
}
