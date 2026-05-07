//! Immediate-mode GUI draw command buffer + asset load queues.
//!
//! Lua resources push `DrawCommand`s do bufferu každý frame přes `Gui.*` API.
//! Klientský `GuiRenderPlugin` v PostUpdate drainuje buffer a vykreslí příkazy
//! přes pre-alokovaný pool Sprite + Text2d entit na dedikované GUI kameře.
//!
//! Souřadnice: normalizované 0.0–1.0, origin vlevo nahoře (jako FiveM).
//! Barvy: 0–255 RGBA integers.

use std::sync::{Arc, Mutex};
use bevy::prelude::*;

/// Jeden vykreslovací příkaz enqueued z Lua threadu.
#[derive(Debug, Clone)]
pub enum DrawCommand {
    /// Vyplněný obdélník. x, y = střed; w, h = šířka/výška (normalizované).
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [u8; 4],
    },
    /// Text. x, y = anchor vlevo nahoře (normalizované); scale = 1.0 → ~24px.
    /// font_id = None → výchozí Bevy font.
    Text {
        text: String,
        x: f32,
        y: f32,
        scale: f32,
        color: [u8; 4],
        font_id: Option<String>,
    },
    /// Čára z (x1, y1) do (x2, y2). Renderuje se jako tenký Sprite.
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        color: [u8; 4],
    },
    /// Kruh (outline, aproximovaný segmenty).
    Circle {
        x: f32,
        y: f32,
        radius: f32,
        color: [u8; 4],
    },
    /// Vyplněný disk (filled circle). x, y = střed; radius = normalizovaný.
    Disc {
        x: f32,
        y: f32,
        radius: f32,
        color: [u8; 4],
    },
    /// Sprite / obrázek. image_id odkazuje na registrovaný obrázek z manifestu.
    /// x, y = střed (normalizované); w, h = rozměry (normalizované).
    /// color = RGBA tint (255,255,255,255 = beze změny).
    /// uv = volitelný UV ořez [u0,v0,u1,v1] (normalizovaně 0–1).
    /// fit = způsob přizpůsobení obrázku do dest. boxu.
    Sprite {
        image_id: String,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [u8; 4],
        uv: Option<[f32; 4]>,
        fit: SpriteFit,
        flip_x: bool,
        flip_y: bool,
    },
}

/// Způsob přizpůsobení obrázku do cílového boxu.
#[derive(Debug, Clone, Default)]
pub enum SpriteFit {
    /// Roztáhne na celý box (ignoruje poměr stran). Výchozí.
    #[default]
    Stretch,
    /// Letterbox: celý obrázek viditelný, zachová poměr stran, okraje prázdné.
    Fit,
    /// Crop-to-fill: vyplní celý box, zachová poměr stran, přebývající části ořízne.
    Fill,
}

// ---------------------------------------------------------------------------
// Asset load queues — naplňuje plugin.rs::rebuild(); drainuje gui_render.rs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct FontLoadRequest {
    pub font_id: String,
    /// Absolutní cesta k souboru fontu (.ttf / .otf).
    pub abs_path: String,
}

#[derive(Debug, Clone)]
pub struct ImageLoadRequest {
    pub image_id: String,
    /// Absolutní cesta k souboru obrázku (.webp / .png / .jpg …).
    pub abs_path: String,
}

#[derive(Resource, Clone, Default)]
pub struct FontLoadQueue(pub Arc<Mutex<Vec<FontLoadRequest>>>);

impl FontLoadQueue {
    pub fn push(&self, req: FontLoadRequest) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).push(req);
    }
    pub fn drain(&self) -> Vec<FontLoadRequest> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

#[derive(Resource, Clone, Default)]
pub struct ImageLoadQueue(pub Arc<Mutex<Vec<ImageLoadRequest>>>);

impl ImageLoadQueue {
    pub fn push(&self, req: ImageLoadRequest) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).push(req);
    }
    pub fn drain(&self) -> Vec<ImageLoadRequest> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

/// Sdílený draw buffer — Bevy Resource i Lua-side clone.
/// Lua thready pushují příkazy v PreUpdate (přes Gui.* API);
/// GuiRenderPlugin drainuje v PostUpdate.
#[derive(Resource, Clone, Default)]
pub struct GuiDrawBuffer(pub Arc<Mutex<Vec<DrawCommand>>>);

impl GuiDrawBuffer {
    pub fn push(&self, cmd: DrawCommand) {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(cmd);
    }

    /// Drainuje buffer a vrátí všechny příkazy. Volat 1× za frame z render systému.
    pub fn drain(&self) -> Vec<DrawCommand> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|p| p.into_inner()))
    }
}
