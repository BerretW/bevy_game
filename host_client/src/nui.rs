//! Phase 4 — NUI overlay plugin.
//!
//! Vytváří transparentní `wry::WebView` embeddovaný do herního okna (child window).
//! Host stránka (nui_host.html) obsahuje `<iframe>` pro každý resource s `ui_page`.
//!
//! Komunikace:
//! ```text
//! Lua                       Bevy (main thread)               Browser
//! ─────────────────────────────────────────────────────────────────────
//! SendNUIMessage(data) ──→  NuiOutQueue ──→ __nui_dispatch() ──→ postMessage()
//! RegisterNUICallback()                ←── POST nui://host/callback/name
//! SetNUIFocus(true)    ──→  NuiOutQueue ──→ __nui_set_focus()
//! ```
//!
//! Custom protocol `nui://`:
//! * `GET  nui://resource__name/path/to/file` — soubor z cache_root
//! * `POST nui://resource__name/callback/cbname` — NuiInMsg pro Lua handler

use std::borrow::Cow;
use std::path::PathBuf;

// Win32: po vytvoření WebView child okna přidáme WS_EX_NOREDIRECTIONBITMAP.
// To říká DWM aby nepoužíval GDI redirection bitmap pro toto okno — WebView2 si
// směřuje vlastní DComp vizulní strom a transparentnost pak správně funguje
// přes Bevy DirectX swap chain.
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::HWND as SysHWND,
    UI::WindowsAndMessaging::{
        FindWindowExW, GetWindowLongPtrW, SetWindowLongPtrW,
        GWL_EXSTYLE, WS_EX_NOREDIRECTIONBITMAP,
    },
};

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
// V Bevy 0.18 je WinitWindows thread-local static, ne NonSend resource.
use bevy::winit::WINIT_WINDOWS;

use core_resources::{host_to_resource_id_str, NuiInMsg, NuiInQueue, NuiOutMsg, NuiOutQueue, ResourceId};

use wry::{Rect, WebView, WebViewBuilder, WebViewId};
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::http::{Request, Response};

const NUI_HOST_HTML: &str = include_str!("nui_host.html");

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

pub struct NuiPlugin {
    pub cache_root: PathBuf,
}

impl Plugin for NuiPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(NuiCacheRoot(self.cache_root.clone()));
        app.insert_non_send_resource(NuiState { webview: None });
        app.init_resource::<NuiFocusState>();
        // create_nui_webview běží v Update — opakuje se každý frame dokud WinitWindows
        // není k dispozici, pak se zastaví (early-exit pokud webview already Some).
        app.add_systems(Update, (create_nui_webview, flush_nui_out, sync_nui_bounds).chain());
    }
}

// ---------------------------------------------------------------------------
// NuiFocusState — sdíleno s gameplay systémy pro cursor lock a input blokování
// ---------------------------------------------------------------------------

/// Aktuální stav NUI fokusu. Gameplay systémy ho čtou pro:
/// * `apply_cursor_mode` — uvolní cursor grab pokud `has_focus`
/// * `collect_and_send_input` — pošle nulový input pokud `has_focus`
#[derive(Resource, Default, Clone, Copy)]
pub struct NuiFocusState {
    pub has_focus: bool,
    pub has_cursor: bool,
}

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

#[derive(Resource)]
pub struct NuiCacheRoot(pub PathBuf);

/// Drží živý WebView. NonSend — musí zůstat na main threadu.
pub struct NuiState {
    pub webview: Option<WebView>,
}

// ---------------------------------------------------------------------------
// Startup — vytvoří WebView jako child herního okna
// ---------------------------------------------------------------------------

fn create_nui_webview(
    mut nui_state: NonSendMut<NuiState>,
    primary_window: Query<(Entity, &Window), With<PrimaryWindow>>,
    cache_root: Res<NuiCacheRoot>,
    nui_in: Res<NuiInQueue>,
) {
    if nui_state.webview.is_some() {
        return; // již vytvořen
    }
    
    trace!("[nui] create_nui_webview: trying");
    
    let Some((entity, window)) = primary_window.iter().next() else {
        warn!("[nui] PrimaryWindow not found — NUI disabled");
        return;
    };

    // V Bevy 0.18 je WinitWindows thread-local static, ne NonSend resource.
    // Musíme použít WINIT_WINDOWS.with_borrow().
    let has_window = WINIT_WINDOWS.with_borrow(|ww| ww.get_window(entity).is_some());
    if !has_window {
        trace!("[nui] WinitWindows not yet available — retrying next frame");
        return;
    }

    let w: f64 = window.physical_width() as f64;
    let h: f64 = window.physical_height() as f64;

    let cache_root_path = cache_root.0.clone();
    let nui_in_clone = nui_in.clone();

    // wry 0.46 custom_protocol handler bere (request, async_responder) — synchronně
    // zavoláme responder.respond() okamžitě, čímž emulujeme blocking protokol.
    let webview_result = WINIT_WINDOWS.with_borrow(|ww| {
        let winit_win = ww.get_window(entity)
            .expect("WinitWindows entry must exist (checked above)");
        WebViewBuilder::new()
            .with_bounds(Rect {
                position: LogicalPosition::new(0.0_f64, 0.0_f64).into(),
                size: LogicalSize::new(w, h).into(),
            })
            .with_transparent(true)
            .with_background_color((0, 0, 0, 0))
            .with_devtools(cfg!(debug_assertions))
            .with_html(NUI_HOST_HTML)
            .with_custom_protocol(
                "nui".to_string(),
                move |_id: WebViewId, request: Request<Vec<u8>>| -> Response<Cow<'static, [u8]>> {
                    handle_nui_request(&cache_root_path, &nui_in_clone, request)
                },
            )
            // WindowWrapper<winit::window::Window> dereferencujeme na &winit::window::Window
            // přes **winit_win (WindowWrapper implementuje Deref<Target = winit::window::Window>).
            .build_as_child(&**winit_win)
    });
    
    // (No log here, result is logged below)

    match webview_result {
        Ok(wv) => {
            info!("[nui] WebView CREATED successfully ({}x{})", w, h);
            // Přidáme WS_EX_NOREDIRECTIONBITMAP na WRY_WEBVIEW child HWND.
            // Bez tohoto flagu DWM kompozituje child okno přes GDI redirection bitmap
            // (opaque bílá), která překrývá Bevy DirectX swap chain.
            // FindWindowExW najde child "WRY_WEBVIEW" okno přímo pod parent HWND.
            #[cfg(windows)]
            WINIT_WINDOWS.with_borrow(|ww| {
                if let Some(winit_win) = ww.get_window(entity) {
                    use wry::raw_window_handle::{HasWindowHandle, RawWindowHandle};
                    if let Ok(rh) = winit_win.window_handle() {
                        if let RawWindowHandle::Win32(h) = rh.as_raw() {
                            let parent_hwnd = h.hwnd.get() as SysHWND;
                            // Najdi WRY_WEBVIEW child okno
                            let class: Vec<u16> = "WRY_WEBVIEW\0".encode_utf16().collect();
                            let child = unsafe {
                                FindWindowExW(parent_hwnd, 0, class.as_ptr(), std::ptr::null())
                            };
                            if child != 0 {
                                unsafe {
                                    let ex = GetWindowLongPtrW(child, GWL_EXSTYLE);
                                    SetWindowLongPtrW(child, GWL_EXSTYLE,
                                        ex | WS_EX_NOREDIRECTIONBITMAP as isize);
                                }
                                info!("[nui] WS_EX_NOREDIRECTIONBITMAP set on WRY_WEBVIEW HWND");
                            } else {
                                warn!("[nui] WRY_WEBVIEW child window not found");
                            }
                        }
                    }
                }
            });
            nui_state.webview = Some(wv);
        }
        Err(e) => {
            error!("[nui] WebView creation FAILED: {} — NUI disabled", e);
        }
    }
}

// ---------------------------------------------------------------------------
// Update — drain NuiOutQueue → evaluate JS v WebView
// ---------------------------------------------------------------------------

fn flush_nui_out(
    nui_state: NonSend<NuiState>,
    nui_out: Res<NuiOutQueue>,
    mut focus: ResMut<NuiFocusState>,
) {
    let Some(ref wv) = nui_state.webview else {
        trace!("[nui] flush_nui_out: WebView not ready, messages stay in queue");
        return;
    };

    let msgs = nui_out.drain();
    if msgs.is_empty() {
        return;
    }
    
    debug!("[nui] processing {} message(s) from NuiOutQueue", msgs.len());

    for msg in msgs {
        let js = match msg {
            NuiOutMsg::Dispatch { resource_host, json } => {
                // Escapujeme json pro bezpečné vložení do JS string literálu.
                let escaped = escape_js_string(&json);
                format!("window.__nui_dispatch('{}', '{}')", resource_host, escaped)
            }
            NuiOutMsg::SetFocus { has_focus, has_cursor } => {
                focus.has_focus = has_focus;
                focus.has_cursor = has_cursor;
                format!(
                    "window.__nui_set_focus({}, {})",
                    has_focus, has_cursor
                )
            }
            NuiOutMsg::AddFrame { resource_host, page } => {
                format!("window.__nui_add_frame('{}', '{}')", resource_host, page)
            }
            NuiOutMsg::RemoveFrame { resource_host } => {
                format!("window.__nui_remove_frame('{}')", resource_host)
            }
        };

        debug!("[nui] evaluating: {}", js);
        if let Err(e) = wv.evaluate_script(&js) {
            warn!("[nui] evaluate_script error: {}", e);
        } else {
            trace!("[nui] script evaluated successfully");
        }
    }
}

// ---------------------------------------------------------------------------
// Update — synchronizuj bounds WebView s rozlišením okna
// ---------------------------------------------------------------------------

fn sync_nui_bounds(
    nui_state: NonSend<NuiState>,
    primary_window: Query<&Window, With<PrimaryWindow>>,
) {
    let Some(ref wv) = nui_state.webview else { return };
    let Some(window) = primary_window.iter().next() else { return };

    let w: f64 = window.physical_width() as f64;
    let h: f64 = window.physical_height() as f64;

    let _ = wv.set_bounds(Rect {
        position: LogicalPosition::new(0.0_f64, 0.0_f64).into(),
        size: LogicalSize::new(w, h).into(),
    });
}

// ---------------------------------------------------------------------------
// Custom protocol handler — soubory + callbacks
// ---------------------------------------------------------------------------

fn handle_nui_request(
    cache_root: &PathBuf,
    nui_in: &NuiInQueue,
    request: Request<Vec<u8>>,
) -> Response<Cow<'static, [u8]>> {
    let uri = request.uri();
    let host = uri.host().unwrap_or("");
    let path = uri.path();
    let method = request.method().as_str();

    debug!("[nui] {} {} nui://{}{}", method, request.method().as_str(), host, path);

    if method == "POST" {
        return handle_nui_callback(nui_in, host, path, request.body());
    }
    serve_nui_file(cache_root, host, path)
}

fn handle_nui_callback(
    nui_in: &NuiInQueue,
    host: &str,
    path: &str,
    body: &[u8],
) -> Response<Cow<'static, [u8]>> {
    let cb_name = path
        .strip_prefix("/callback/")
        .unwrap_or("")
        .trim_end_matches('/');

    if cb_name.is_empty() {
        return error_response(400, "missing callback name");
    }

    let resource_id = ResourceId::new(host_to_resource_id_str(host));
    nui_in.push(NuiInMsg {
        resource_id,
        callback_name: cb_name.to_string(),
        data: body.to_vec(),
    });
    ok_json_response()
}

fn serve_nui_file(
    cache_root: &PathBuf,
    host: &str,
    path: &str,
) -> Response<Cow<'static, [u8]>> {
    let resource_rel = host_to_resource_id_str(host);
    let file_rel = path.trim_start_matches('/');

    if file_rel.is_empty() {
        // Cesta je prázdná — vrátit 404 místo prázdné stránky
        // (JavaScript by měl žádat úplnou cestu, např. 'ui/index.html')
        debug!("[nui] empty file path for {}, returning 404", host);
        return error_response(404, "file path required");
    }

    // Bezpečnostní kontrola — zakázáno path traversal
    for part in file_rel.split('/') {
        if part == ".." || part == "." {
            return error_response(403, "path traversal not allowed");
        }
    }

    let mut full_path = cache_root.clone();
    for segment in resource_rel.split('/') {
        full_path.push(segment);
    }
    full_path.push(file_rel);

    match std::fs::read(&full_path) {
        Ok(bytes) => {
            let mime = mime_type_for(file_rel);
            Response::builder()
                .status(200)
                .header("Content-Type", mime)
                .header("Access-Control-Allow-Origin", "*")
                .body(Cow::Owned(bytes))
                .unwrap_or_else(|_| error_response(500, "response build error"))
        }
        Err(e) => {
            debug!("[nui] file not found: {} — {}", full_path.display(), e);
            error_response(404, "not found")
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ok_json_response() -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .header("Access-Control-Allow-Origin", "*")
        .body(Cow::Borrowed(b"{}".as_slice()))
        .unwrap_or_else(|_| error_response(500, "build error"))
}

fn ok_html_response(body: &'static [u8]) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(200)
        .header("Content-Type", "text/html; charset=utf-8")
        .header("Access-Control-Allow-Origin", "*")
        .body(Cow::Borrowed(body))
        .unwrap_or_else(|_| error_response(500, "build error"))
}

fn error_response(status: u16, msg: &'static str) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/plain")
        .body(Cow::Borrowed(msg.as_bytes()))
        .unwrap_or_else(|_| {
            Response::builder()
                .status(500)
                .body(Cow::Borrowed(b"internal error".as_slice()))
                .unwrap()
        })
}

fn mime_type_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css"          => "text/css; charset=utf-8",
        "js" | "mjs"   => "text/javascript; charset=utf-8",
        "json"         => "application/json",
        "png"          => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif"          => "image/gif",
        "svg"          => "image/svg+xml",
        "woff"         => "font/woff",
        "woff2"        => "font/woff2",
        "ttf"          => "font/ttf",
        "ico"          => "image/x-icon",
        "mp3"          => "audio/mpeg",
        "ogg"          => "audio/ogg",
        "wav"          => "audio/wav",
        _              => "application/octet-stream",
    }
}

/// Escapuje string pro bezpečné vložení jako JS single-quoted string literál.
fn escape_js_string(s: &str) -> String {
    s.replace('\\', "\\\\")
     .replace('\'', "\\'")
     .replace('\n', "\\n")
     .replace('\r', "\\r")
}
