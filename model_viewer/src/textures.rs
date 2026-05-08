use std::path::Path;

use bevy::gltf::{Gltf, GltfAssetLabel};
use bevy::prelude::*;
use core_drawable::{DrawableManifest, DrawableManifestRegistry};

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
    /// Zkrácené jméno pro popis v panelu (max 16 znaků).
    pub short: String,
    /// Bevy handle na texturu — vyplnit po načtení GLTF.
    pub handle: Handle<Image>,
    /// "mat/slot" z drawable manifestu, pokud se podařilo namapovat.
    pub slot: Option<String>,
}

#[derive(Resource, Default)]
pub struct TextureBrowser {
    pub entries:        Vec<TextureEntry>,
    pub visible:        bool,
    pub scroll:         usize,
    pub loaded:         bool,
    pub extract_status: Option<String>,
}

#[derive(Component)]
pub struct TextureBrowserRoot;

#[derive(Component)]
pub struct ExtractStatusText;

// ─── Init ─────────────────────────────────────────────────────────────────────

/// Spustí se jednou jakmile jsou GLTF assety načteny; načte handle textur a
/// sestaví slot mapping z drawable manifestu.
pub fn init_texture_browser(
    mut done:      Local<bool>,
    gltf_cache:    Res<GltfHandleCache>,
    gltf_assets:   Res<Assets<Gltf>>,
    drawable_reg:  Res<DrawableManifestRegistry>,
    manifests:     Res<Assets<DrawableManifest>>,
    asset_server:  Res<AssetServer>,
    mut browser:   ResMut<TextureBrowser>,
) {
    if *done { return; }
    if gltf_cache.0.is_empty() { return; }
    if !gltf_cache.0.values().all(|(h, _)| gltf_assets.get(h).is_some()) { return; }

    for (stem, (gltf_handle, bevy_path)) in &gltf_cache.0 {
        let Some(gltf) = gltf_assets.get(gltf_handle) else { continue };

        // Mapa: image_name → "mat/slot" z .drawable
        let mut slot_map: std::collections::HashMap<String, String> = Default::default();
        if let Some(mh) = drawable_reg.0.get(stem) {
            if let Some(manifest) = manifests.get(mh) {
                for (mat, mat_def) in &manifest.materials {
                    for (slot, tex) in &mat_def.textures {
                        let key = tex.name
                            .trim_end_matches(|c: char| c == '.' || c.is_ascii_alphabetic())
                            .to_string();
                        slot_map.entry(key).or_insert_with(|| format!("{}/{}", mat, slot));
                        slot_map.entry(tex.name.clone())
                            .or_insert_with(|| format!("{}/{}", mat, slot));
                    }
                }
            }
        }

        let img_count = gltf.source.as_ref().map(|s| s.images().count()).unwrap_or(0);
        // Klonujeme cestu aby splnila 'static bound pro from_asset
        let path_owned: String = bevy_path.clone();

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

    browser.loaded = true;
    *done = true;
}

fn shorten(s: &str) -> String {
    if s.len() > 16 { format!("{}…", &s[..15]) } else { s.to_string() }
}

// ─── Keyboard ─────────────────────────────────────────────────────────────────

pub fn handle_texture_keys(
    keys:       Res<ButtonInput<KeyCode>>,
    gltf_cache: Res<GltfHandleCache>,
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
        let msg = export_all(&gltf_cache);
        browser.extract_status = Some(msg);
    }
}

// ─── Panel UI ─────────────────────────────────────────────────────────────────

/// Přestaví panel při změně viditelnosti nebo scrollu.
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
            // Záhlaví
            p.spawn((
                Text::new(format!(
                    "Textury ({})   ↑↓   T zavřít",
                    browser.entries.len()
                )),
                TextFont { font_size: 11.0, ..default() },
                TextColor(Color::srgba(0.75, 0.75, 1.0, 0.9)),
            ));

            // Mřížka thumbnailů
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
                            // Náhled textury
                            cell.spawn((
                                ImageNode { image: e.handle.clone(), ..default() },
                                Node {
                                    width:  Val::Px(THUMB),
                                    height: Val::Px(THUMB),
                                    ..default()
                                },
                            ));
                            // Jméno
                            cell.spawn((
                                Text::new(e.short.clone()),
                                TextFont  { font_size: 9.0, ..default() },
                                TextColor(Color::srgba(0.9, 0.9, 0.9, 0.85)),
                            ));
                            // Slot
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

            // Scroll indikátor
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

            // Export hint
            p.spawn((
                Text::new("E: exportovat textury na disk"),
                TextFont  { font_size: 10.0, ..default() },
                TextColor(Color::srgba(0.6, 0.85, 0.6, 0.8)),
            ));
        });
}

/// Zobrazí nebo schová stavový text extrakce (dole vlevo).
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

// ─── Extrakce ─────────────────────────────────────────────────────────────────

fn export_all(cache: &GltfHandleCache) -> String {
    let mut total = 0usize;
    let mut errors: Vec<String> = Vec::new();

    for (_stem, (_handle, bevy_path)) in &cache.0 {
        let path = std::path::PathBuf::from(bevy_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("model");
        let out_dir = path.parent().unwrap_or(Path::new(".")).join(format!("{}_textures", stem));

        match extract_glb(&path, &out_dir) {
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

/// Parsuje GLB binárně a extrahuje všechny embedded images jako soubory.
/// Vrátí počet zapsaných souborů.
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
    let doc: serde_json::Value = serde_json::from_slice(json)
        .map_err(|e| format!("JSON: {e}"))?;

    let Some(images) = doc.get("images").and_then(|v| v.as_array()) else {
        return Ok(0);
    };
    let bvs = doc.get("bufferViews")
        .and_then(|v| v.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
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
            Path::new(name).file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(name)
                .to_string()
        };

        // Blender GLTF exporter pakuje albedo + MB alpha do jednoho obrázku
        // a pojmenovává ho "<albedo_stem>-<mb_stem>". Takový název detekujeme
        // a ořežeme na první část — MB alpha je shared textura, v GLB je
        // navíc a Bevy shader ji nepotřebuje z tohoto packed souboru.
        let (file_stem, packed_warning) = if let Some(dash_pos) = raw_stem.find('-') {
            let first = &raw_stem[..dash_pos];
            let second = &raw_stem[dash_pos + 1..];
            // Heuristika: obě části obsahují podtržítko (typické pro asset jména)
            if first.contains('_') && second.contains('_') {
                warn!(
                    "[viewer] packed textura '{raw_stem}' — Blender sloučil '{first}' + '{second}'. \
                     Exportuji jako '{first}'. Přeexportuj GLB s opravou bevy_toolkit.zip."
                );
                (first.to_string(), true)
            } else {
                (raw_stem.clone(), false)
            }
        } else {
            (raw_stem.clone(), false)
        };
        let _ = packed_warning;
        let file_name = format!("{file_stem}.{ext}");

        let Some(bv_idx) = img.get("bufferView").and_then(|v| v.as_u64()) else {
            continue; // přeskočit externí URI
        };
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
