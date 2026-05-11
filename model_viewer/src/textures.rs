use std::path::Path;

use bevy::gltf::{Gltf, GltfAssetLabel};
use bevy::prelude::*;
use core_drawable::{AdmScene, AdmSceneRoot, DrawableManifest, DrawableManifestRegistry};

use crate::GltfHandleCache;

// ─── Constants ────────────────────────────────────────────────────────────────

const THUMB: f32 = 96.0;
const COLS: usize = 2;
pub const PANEL_W: f32 = THUMB * COLS as f32 + 32.0; // 224
const ROWS: usize = 4;

// ─── Data ─────────────────────────────────────────────────────────────────────

pub struct TextureEntry {
    #[allow(dead_code)]
    pub name: String,
    pub short: String,
    pub handle: Handle<Image>,
    pub slot: Option<String>,
}

#[derive(Resource, Default)]
pub struct TextureBrowser {
    pub entries:        Vec<TextureEntry>,
    pub visible:        bool,
    pub scroll:         usize,
    pub loaded:         bool,
    pub extract_status: Option<String>,
    /// Cesty ADM souborů pro E-export (jméno modelu → cesta na disku).
    pub adm_paths:      std::collections::HashMap<String, String>,
}

#[derive(Component)]
pub struct TextureBrowserRoot;

#[derive(Component)]
pub struct ExtractStatusText;

// ─── Init ─────────────────────────────────────────────────────────────────────

pub fn init_texture_browser(
    mut done:         Local<bool>,
    gltf_cache:       Res<GltfHandleCache>,
    gltf_assets:      Res<Assets<Gltf>>,
    adm_roots:        Query<(&AdmSceneRoot, &core_resources::ModelName)>,
    adm_assets:       Res<Assets<AdmScene>>,
    drawable_reg:     Res<DrawableManifestRegistry>,
    manifests:        Res<Assets<DrawableManifest>>,
    asset_server:     Res<AssetServer>,
    mut browser:      ResMut<TextureBrowser>,
) {
    if *done { return; }

    let has_glb = !gltf_cache.0.is_empty();
    let has_adm = adm_roots.iter().next().is_some();

    if !has_glb && !has_adm { return; }

    // GLB: čekáme dokud se všechny načtou
    if has_glb && !gltf_cache.0.values().all(|(h, _)| gltf_assets.get(h).is_some()) { return; }

    // ADM: čekáme dokud se všechny načtou
    if has_adm && adm_roots.iter().any(|(r, _)| adm_assets.get(&r.0).is_none()) { return; }

    // ── Slot mapa z drawable manifestů ───────────────────────────────────────
    let mut slot_map: std::collections::HashMap<String, String> = Default::default();
    for (stem, mh) in &drawable_reg.0 {
        if let Some(manifest) = manifests.get(mh) {
            for (mat, mat_def) in &manifest.materials {
                for (slot, tex) in &mat_def.textures {
                    let key = tex.name
                        .trim_end_matches(|c: char| c == '.' || c.is_ascii_alphabetic())
                        .to_string();
                    let label = format!("{}/{}/{}", stem, mat, slot);
                    slot_map.entry(key).or_insert_with(|| label.clone());
                    slot_map.entry(tex.name.clone()).or_insert_with(|| label);
                }
            }
        }
    }

    // ── GLB textury ───────────────────────────────────────────────────────────
    for (_stem, (gltf_handle, bevy_path)) in &gltf_cache.0 {
        let Some(gltf) = gltf_assets.get(gltf_handle) else { continue };
        let img_count = gltf.source.as_ref().map(|s| s.images().count()).unwrap_or(0);
        let path_owned = bevy_path.clone();

        for i in 0..img_count {
            let raw = gltf.source.as_ref()
                .and_then(|s| s.images().nth(i))
                .and_then(|img| img.name().map(|n| n.to_string()))
                .unwrap_or_else(|| format!("image_{}", i));

            let handle: Handle<Image> = asset_server
                .load(GltfAssetLabel::Texture(i).from_asset(path_owned.clone()));

            let stem_only = Path::new(&raw)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(&raw);
            let slot = slot_map.get(stem_only).or_else(|| slot_map.get(&raw)).cloned();

            browser.entries.push(TextureEntry {
                short: shorten(stem_only),
                name:  raw,
                handle,
                slot,
            });
        }
    }

    // ── ADM embedded textury ──────────────────────────────────────────────────
    for (adm_root, model_name) in &adm_roots {
        let Some(adm) = adm_assets.get(&adm_root.0) else { continue };

        // Ulož cestu pro E-export
        browser.adm_paths.insert(model_name.0.clone(), adm.source_path.clone());

        let mut names: Vec<&String> = adm.embedded.keys().collect();
        names.sort();

        for name in names {
            let handle = adm.embedded[name].clone();
            let stem_only = Path::new(name)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(name);
            let slot = slot_map.get(stem_only).or_else(|| slot_map.get(name.as_str())).cloned();

            browser.entries.push(TextureEntry {
                short: shorten(stem_only),
                name:  name.clone(),
                handle,
                slot,
            });
        }
    }

    browser.loaded = true;
    *done = true;
}

fn shorten(s: &str) -> String {
    if s.len() > 16 { format!("{}…", &s[..15]) } else { s.to_string() }
}

// ─── Keyboard ─────────────────────────────────────────────────────────────────

pub fn handle_texture_keys(
    keys:        Res<ButtonInput<KeyCode>>,
    gltf_cache:  Res<GltfHandleCache>,
    mut browser: ResMut<TextureBrowser>,
) {
    if keys.just_pressed(KeyCode::KeyT) && browser.loaded {
        browser.visible = !browser.visible;
    }

    if browser.visible {
        let total_rows = (browser.entries.len() + COLS - 1) / COLS;
        let max_scroll = total_rows.saturating_sub(ROWS);
        if keys.just_pressed(KeyCode::ArrowUp) {
            browser.scroll = browser.scroll.saturating_sub(1);
        }
        if keys.just_pressed(KeyCode::ArrowDown) && browser.scroll < max_scroll {
            browser.scroll += 1;
        }
    }

    if keys.just_pressed(KeyCode::KeyE) && browser.loaded {
        let msg = export_all(&gltf_cache, &browser.adm_paths);
        browser.extract_status = Some(msg);
    }
}

// ─── Panel UI ─────────────────────────────────────────────────────────────────

pub fn rebuild_panel(
    mut last_vis:    Local<Option<bool>>,
    mut last_scroll: Local<usize>,
    browser:         Res<TextureBrowser>,
    roots:           Query<Entity, With<TextureBrowserRoot>>,
    mut commands:    Commands,
) {
    let vis_changed    = Some(browser.visible) != *last_vis;
    let scroll_changed = browser.scroll != *last_scroll;
    if !vis_changed && !scroll_changed { return; }
    *last_vis    = Some(browser.visible);
    *last_scroll = browser.scroll;

    for e in &roots { commands.entity(e).despawn(); }
    if !browser.visible { return; }

    let start = browser.scroll * COLS;
    let end   = (start + ROWS * COLS).min(browser.entries.len());

    commands
        .spawn((
            TextureBrowserRoot,
            Node {
                position_type:  PositionType::Absolute,
                right:          Val::Px(0.0),
                top:            Val::Px(0.0),
                bottom:         Val::Px(0.0),
                width:          Val::Px(PANEL_W),
                flex_direction: FlexDirection::Column,
                padding:        UiRect::all(Val::Px(8.0)),
                row_gap:        Val::Px(5.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.06, 0.06, 0.10, 0.92)),
        ))
        .with_children(|p| {
            p.spawn((
                Text::new(format!(
                    "Textury ({})   ↑↓   T zavřít",
                    browser.entries.len()
                )),
                TextFont { font_size: 11.0, ..default() },
                TextColor(Color::srgba(0.75, 0.75, 1.0, 0.9)),
            ));

            if browser.entries.is_empty() {
                p.spawn((
                    Text::new("(žádné embedded textury)"),
                    TextFont { font_size: 11.0, ..default() },
                    TextColor(Color::srgba(0.6, 0.6, 0.6, 0.8)),
                ));
            }

            for row_start in (start..end).step_by(COLS) {
                p.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap:     Val::Px(8.0),
                    ..default()
                })
                .with_children(|row| {
                    for col in 0..COLS {
                        let idx = row_start + col;
                        if idx >= browser.entries.len() { break; }
                        let e = &browser.entries[idx];

                        row.spawn(Node {
                            flex_direction: FlexDirection::Column,
                            width:          Val::Px(THUMB),
                            row_gap:        Val::Px(2.0),
                            ..default()
                        })
                        .with_children(|cell| {
                            cell.spawn((
                                ImageNode { image: e.handle.clone(), ..default() },
                                Node {
                                    width:  Val::Px(THUMB),
                                    height: Val::Px(THUMB),
                                    ..default()
                                },
                            ));
                            cell.spawn((
                                Text::new(e.short.clone()),
                                TextFont  { font_size: 9.0, ..default() },
                                TextColor(Color::srgba(0.9, 0.9, 0.9, 0.85)),
                            ));
                            if let Some(slot) = &e.slot {
                                cell.spawn((
                                    Text::new(slot.clone()),
                                    TextFont  { font_size: 8.0, ..default() },
                                    TextColor(Color::srgba(0.5, 0.9, 0.5, 0.8)),
                                ));
                            }
                        });
                    }
                });
            }

            let total_rows = (browser.entries.len() + COLS - 1) / COLS;
            if total_rows > ROWS {
                p.spawn((
                    Text::new(format!(
                        "řada {}/{}",
                        browser.scroll + 1,
                        total_rows.saturating_sub(ROWS - 1)
                    )),
                    TextFont  { font_size: 9.0, ..default() },
                    TextColor(Color::srgba(0.5, 0.5, 0.5, 0.7)),
                ));
            }

            p.spawn((
                Text::new("E: exportovat textury na disk"),
                TextFont  { font_size: 10.0, ..default() },
                TextColor(Color::srgba(0.6, 0.85, 0.6, 0.8)),
            ));
        });
}

pub fn show_extract_status(
    mut last_status: Local<Option<String>>,
    browser:         Res<TextureBrowser>,
    status_q:        Query<Entity, With<ExtractStatusText>>,
    mut commands:    Commands,
) {
    if browser.extract_status == *last_status { return; }
    *last_status = browser.extract_status.clone();

    for e in &status_q { commands.entity(e).despawn(); }

    if let Some(status) = &browser.extract_status {
        commands.spawn((
            ExtractStatusText,
            Text::new(status.clone()),
            TextFont { font_size: 14.0, ..default() },
            TextColor(Color::srgba(0.35, 1.0, 0.45, 0.95)),
            Node {
                position_type: PositionType::Absolute,
                bottom:        Val::Px(30.0),
                left:          Val::Px(8.0),
                ..default()
            },
        ));
    }
}

// ─── Export ───────────────────────────────────────────────────────────────────

fn export_all(
    gltf_cache: &GltfHandleCache,
    adm_paths:  &std::collections::HashMap<String, String>,
) -> String {
    let mut total = 0usize;
    let mut errors: Vec<String> = Vec::new();

    for (_stem, (_handle, bevy_path)) in &gltf_cache.0 {
        let path = std::path::PathBuf::from(bevy_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("model");
        let out_dir = path.parent().unwrap_or(Path::new(".")).join(format!("{}_textures", stem));
        match extract_glb(&path, &out_dir) {
            Ok(n)  => total += n,
            Err(e) => errors.push(format!("{stem}: {e}")),
        }
    }

    for (stem, source_path) in adm_paths {
        let path = std::path::PathBuf::from(source_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let out_dir = path.parent().unwrap_or(Path::new(".")).join(format!("{}_textures", stem));
        match extract_adm(&path, &out_dir) {
            Ok(n)  => total += n,
            Err(e) => errors.push(format!("{stem}: {e}")),
        }
    }

    if errors.is_empty() {
        format!("Exportováno {} textur", total)
    } else if total > 0 {
        format!("{} textur, chyby: {}", total, errors.join("; "))
    } else {
        format!("Chyba: {}", errors.join("; "))
    }
}

/// Extrahuje embedded textury z GLB souboru.
fn extract_glb(glb_path: &Path, out_dir: &Path) -> Result<usize, String> {
    let bytes = std::fs::read(glb_path).map_err(|e| format!("čtení: {e}"))?;
    if bytes.len() < 12 {
        return Err("soubor příliš krátký".into());
    }
    let magic = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    if magic != 0x46546C67 {
        return Err("není GLB soubor".into());
    }

    let mut json_slice: Option<&[u8]> = None;
    let mut bin_slice:  Option<&[u8]> = None;
    let mut pos = 12usize;
    while pos + 8 <= bytes.len() {
        let chunk_len  = u32::from_le_bytes(bytes[pos..pos+4].try_into().unwrap()) as usize;
        let chunk_type = u32::from_le_bytes(bytes[pos+4..pos+8].try_into().unwrap());
        let end = pos + 8 + chunk_len;
        if end > bytes.len() { break; }
        match chunk_type {
            0x4E4F534A => json_slice = Some(&bytes[pos+8..end]),
            0x004E4942 => bin_slice  = Some(&bytes[pos+8..end]),
            _ => {}
        }
        pos = end;
    }

    let json = json_slice.ok_or("chybí JSON chunk")?;
    let doc: serde_json::Value = serde_json::from_slice(json).map_err(|e| format!("JSON: {e}"))?;
    let Some(images) = doc.get("images").and_then(|v| v.as_array()) else { return Ok(0); };
    let bvs = doc.get("bufferViews").and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&[]);
    let bin = bin_slice.unwrap_or(&[]);

    std::fs::create_dir_all(out_dir).map_err(|e| format!("mkdir: {e}"))?;
    let mut count = 0usize;

    for (idx, img) in images.iter().enumerate() {
        let name = img.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let mime = img.get("mimeType").and_then(|v| v.as_str()).unwrap_or("image/png");
        let ext  = if mime.contains("jpeg") || mime.contains("jpg") { "jpg" } else { "png" };

        let raw_stem = if name.is_empty() {
            format!("image_{idx}")
        } else {
            Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or(name).to_string()
        };

        let file_name = format!("{raw_stem}.{ext}");
        let Some(bv_idx) = img.get("bufferView").and_then(|v| v.as_u64()) else { continue };
        let Some(bv) = bvs.get(bv_idx as usize) else { continue };
        let off = bv.get("byteOffset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let len = bv.get("byteLength").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        if off + len > bin.len() { continue; }

        let out_path = out_dir.join(&file_name);
        std::fs::write(&out_path, &bin[off..off + len]).map_err(|e| format!("{file_name}: {e}"))?;
        info!("[viewer] textura → {:?}", out_path);
        count += 1;
    }
    Ok(count)
}

/// Extrahuje embedded textury z ADM souboru.
/// ADM formát: magic(4) + version(4) + mesh_count(4) + node_count(4) + has_textures(4)
/// + [mesh data] + [node data] + [texture count(4) + (name_len(2) + name + fmt(1) + srgb(1) + size(4) + data)*]
fn extract_adm(adm_path: &Path, out_dir: &Path) -> Result<usize, String> {
    use std::io::{Cursor, Read};

    let bytes = std::fs::read(adm_path).map_err(|e| format!("čtení: {e}"))?;
    let mut cur = Cursor::new(bytes.as_slice());

    // Header
    let mut magic = [0u8; 4];
    cur.read_exact(&mut magic).map_err(|_| "čtení magic")?;
    if &magic != b"ADM\0" {
        return Err("není ADM soubor".into());
    }
    let version = read_u32_cur(&mut cur)?;
    if version != 1 && version != 2 && version != 3 {
        return Err(format!("nepodporovaná verze ADM: {}", version));
    }
    let mesh_count   = read_u32_cur(&mut cur)? as usize;
    let node_count   = read_u32_cur(&mut cur)? as usize;
    let has_textures = read_u32_cur(&mut cur)?;

    if has_textures == 0 {
        return Ok(0);
    }

    // Přeskočíme mesh data
    for _ in 0..mesh_count {
        skip_adm_mesh(&mut cur)?;
    }
    // Přeskočíme node data
    for _ in 0..node_count {
        skip_adm_node(&mut cur)?;
    }

    // Textury
    let tex_count = read_u32_cur(&mut cur)? as usize;
    std::fs::create_dir_all(out_dir).map_err(|e| format!("mkdir: {e}"))?;
    let mut count = 0usize;

    for _ in 0..tex_count {
        let name   = read_str_cur(&mut cur)?;
        let fmt    = read_u8_cur(&mut cur)?;    // 1=PNG, 2=DDS
        let _srgb  = read_u8_cur(&mut cur)?;
        let size   = read_u32_cur(&mut cur)? as usize;
        let mut data = vec![0u8; size];
        cur.read_exact(&mut data).map_err(|_| format!("čtení textury '{}'", name))?;

        let ext = if fmt == 2 { "dds" } else { "png" };
        let stem = Path::new(&name).file_stem().and_then(|s| s.to_str()).unwrap_or(&name);
        let file_name = format!("{}.{}", stem, ext);
        let out_path = out_dir.join(&file_name);
        std::fs::write(&out_path, &data).map_err(|e| format!("{file_name}: {e}"))?;
        info!("[viewer] textura → {:?}", out_path);
        count += 1;
    }
    Ok(count)
}

// ─── ADM binary helpers ───────────────────────────────────────────────────────

fn read_u8_cur(cur: &mut std::io::Cursor<&[u8]>) -> Result<u8, String> {
    use std::io::Read;
    let mut b = [0u8; 1];
    cur.read_exact(&mut b).map_err(|_| "čtení u8")?;
    Ok(b[0])
}

fn read_u16_cur(cur: &mut std::io::Cursor<&[u8]>) -> Result<u16, String> {
    use std::io::Read;
    let mut b = [0u8; 2];
    cur.read_exact(&mut b).map_err(|_| "čtení u16")?;
    Ok(u16::from_le_bytes(b))
}

fn read_u32_cur(cur: &mut std::io::Cursor<&[u8]>) -> Result<u32, String> {
    use std::io::Read;
    let mut b = [0u8; 4];
    cur.read_exact(&mut b).map_err(|_| "čtení u32")?;
    Ok(u32::from_le_bytes(b))
}

fn read_str_cur(cur: &mut std::io::Cursor<&[u8]>) -> Result<String, String> {
    use std::io::Read;
    let len = read_u16_cur(cur)? as usize;
    let mut s = vec![0u8; len];
    cur.read_exact(&mut s).map_err(|_| "čtení string")?;
    String::from_utf8(s).map_err(|_| "invalid UTF-8".into())
}

fn skip_bytes(cur: &mut std::io::Cursor<&[u8]>, n: usize) -> Result<(), String> {
    use std::io::Seek;
    cur.seek(std::io::SeekFrom::Current(n as i64)).map_err(|_| "seek")?;
    Ok(())
}

/// Přeskočí jeden ADM mesh blok.
/// Formát: name(str) + vert_count(u32) + index_count(u32) + attr_flags(u32)
///         + [atributy dle flags] + [indices: u32 × index_count]
fn skip_adm_mesh(cur: &mut std::io::Cursor<&[u8]>) -> Result<(), String> {
    let _name        = read_str_cur(cur)?;
    let vert_count   = read_u32_cur(cur)? as usize;
    let index_count  = read_u32_cur(cur)? as usize;
    let attr_flags   = read_u32_cur(cur)?;

    const ATTR_POS:    u32 = 1 << 0;
    const ATTR_NRM:    u32 = 1 << 1;
    const ATTR_TAN:    u32 = 1 << 2;
    const ATTR_UV0:    u32 = 1 << 3;
    const ATTR_UV1:    u32 = 1 << 4;
    const ATTR_MASKS0: u32 = 1 << 5;
    const ATTR_MASKS1: u32 = 1 << 6;

    if attr_flags & ATTR_POS    != 0 { skip_bytes(cur, vert_count * 3 * 4)?; }
    if attr_flags & ATTR_NRM    != 0 { skip_bytes(cur, vert_count * 3 * 4)?; }
    if attr_flags & ATTR_TAN    != 0 { skip_bytes(cur, vert_count * 4 * 4)?; }
    if attr_flags & ATTR_UV0    != 0 { skip_bytes(cur, vert_count * 2 * 4)?; }
    if attr_flags & ATTR_UV1    != 0 { skip_bytes(cur, vert_count * 2 * 4)?; }
    if attr_flags & ATTR_MASKS0 != 0 { skip_bytes(cur, vert_count * 4)?; }
    if attr_flags & ATTR_MASKS1 != 0 { skip_bytes(cur, vert_count * 4)?; }

    skip_bytes(cur, index_count * 4)?;
    Ok(())
}

/// Přeskočí jeden ADM node blok.
/// Formát: name(str) + node_type(u8) + mesh_index(i32) + parent_index(i32)
///         + material_name(str) + mat4(16×f32)
fn skip_adm_node(cur: &mut std::io::Cursor<&[u8]>) -> Result<(), String> {
    let _name          = read_str_cur(cur)?;
    let _node_type     = read_u8_cur(cur)?;
    skip_bytes(cur, 4)?; // mesh_index i32
    skip_bytes(cur, 4)?; // parent_index i32
    let _material_name = read_str_cur(cur)?;
    skip_bytes(cur, 16 * 4)?; // mat4
    Ok(())
}
