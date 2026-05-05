//! `manifest.lua` parser.
//!
//! Manifest je miniaturní deklarativní DSL — žádný kontrolní tok ani I/O.
//! Parsujeme ho v izolovaném `mlua` interpreteru s extrémně omezenou stdlib
//! (jen `string`/`table`/`math`), který nemá přístup k `io`, `os`, `require`
//! ani nic dalšího, co by mohlo eskalovat. DSL funkce jsou registrované
//! jako Rust closures a zapisují do `ManifestBuilder` přes `Lua::set_app_data`.
//!
//! Záměrně NEpoužívá runtime sandbox z [`crate::sandbox`] — manifest se musí
//! parsovat **dřív**, než vůbec víme, jaký runtime sandbox stavět.

use std::path::{Path, PathBuf};

use mlua::{Lua, LuaOptions, StdLib, Table};
use serde::{Deserialize, Serialize};

use crate::types::ResourceId;

/// Kategorie resource — pro Phase 1 stačí `Script`, ostatní jsou připravené.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResourceKind {
    Script,
    Asset,
    Map,
    GameMode,
}

impl ResourceKind {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "script" => Some(Self::Script),
            "asset" => Some(Self::Asset),
            "map" => Some(Self::Map),
            "gamemode" | "mode" => Some(Self::GameMode),
            _ => None,
        }
    }
}

/// Statická data jednoho resource načtená z `manifest.lua`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub id: ResourceId,
    pub root: PathBuf,
    pub kind: ResourceKind,
    pub author: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub dependencies: Vec<String>,
    pub shared_scripts: Vec<String>,
    pub client_scripts: Vec<String>,
    pub server_scripts: Vec<String>,
    pub files: Vec<String>,
    /// Relativní cesta k hlavní HTML stránce NUI overlay (Phase 4).
    /// Příklad: `'ui/index.html'`
    /// Pokud je nastaveno, klient vytvoří transparentní iframe pro tento resource.
    pub ui_page: Option<String>,
}

#[derive(Debug, Default, Clone)]
struct ManifestBuilder {
    kind: Option<String>,
    author: Option<String>,
    version: Option<String>,
    description: Option<String>,
    dependencies: Vec<String>,
    shared_scripts: Vec<String>,
    client_scripts: Vec<String>,
    server_scripts: Vec<String>,
    files: Vec<String>,
    ui_page: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("manifest.lua not found at {0}")]
    NotFound(PathBuf),
    #[error("io error reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("lua error in {path}: {source}")]
    Lua {
        path: PathBuf,
        #[source]
        source: mlua::Error,
    },
    #[error("manifest at {path:?} is missing required `resource_type`")]
    MissingKind { path: PathBuf },
    #[error("manifest at {path:?} has unknown resource_type {kind:?}")]
    UnknownKind { path: PathBuf, kind: String },
}

/// Načte a sparsuje `manifest.lua` z dané resource cesty.
///
/// `resource_root` je adresář, který obsahuje `manifest.lua` (= rodič manifestu).
pub fn parse_manifest(id: ResourceId, resource_root: &Path) -> Result<Manifest, ManifestError> {
    let manifest_path = resource_root.join("manifest.lua");
    if !manifest_path.is_file() {
        return Err(ManifestError::NotFound(manifest_path));
    }

    let source = std::fs::read_to_string(&manifest_path).map_err(|e| ManifestError::Io {
        path: manifest_path.clone(),
        source: e,
    })?;

    // Velmi omezená stdlib pro manifest DSL — žádné `os`, `io`, `package`, `require`,
    // `debug`, `coroutine`. Neumožňuje boční účinky mimo zápis do app_data.
    let lua = Lua::new_with(
        StdLib::TABLE | StdLib::STRING | StdLib::MATH,
        LuaOptions::default(),
    )
    .map_err(|e| ManifestError::Lua {
        path: manifest_path.clone(),
        source: e,
    })?;

    lua.set_app_data(ManifestBuilder::default());

    install_dsl(&lua).map_err(|e| ManifestError::Lua {
        path: manifest_path.clone(),
        source: e,
    })?;

    lua.load(&source)
        .set_name(format!("manifest:{}", id))
        .exec()
        .map_err(|e| ManifestError::Lua {
            path: manifest_path.clone(),
            source: e,
        })?;

    let builder = lua
        .app_data_ref::<ManifestBuilder>()
        .expect("ManifestBuilder was inserted as app_data above")
        .clone();

    let kind_str = builder
        .kind
        .ok_or_else(|| ManifestError::MissingKind {
            path: manifest_path.clone(),
        })?;
    let kind = ResourceKind::parse(&kind_str).ok_or_else(|| ManifestError::UnknownKind {
        path: manifest_path.clone(),
        kind: kind_str,
    })?;

    Ok(Manifest {
        id,
        root: resource_root.to_path_buf(),
        kind,
        author: builder.author,
        version: builder.version,
        description: builder.description,
        dependencies: builder.dependencies,
        shared_scripts: builder.shared_scripts,
        client_scripts: builder.client_scripts,
        server_scripts: builder.server_scripts,
        files: builder.files,
        ui_page: builder.ui_page,
    })
}

/// Nainstaluje DSL globální funkce do dané Lua VM:
/// `resource_type`, `author`, `version`, `description` (string setters),
/// `dependencies`, `shared_scripts`, `client_scripts`, `server_scripts`, `files`
/// (list-of-string setters).
fn install_dsl(lua: &Lua) -> mlua::Result<()> {
    let globals = lua.globals();

    install_string_setter(lua, &globals, "resource_type", |b, v| b.kind = Some(v))?;
    install_string_setter(lua, &globals, "author", |b, v| b.author = Some(v))?;
    install_string_setter(lua, &globals, "version", |b, v| b.version = Some(v))?;
    install_string_setter(lua, &globals, "description", |b, v| b.description = Some(v))?;
    install_string_setter(lua, &globals, "ui_page", |b, v| b.ui_page = Some(v))?;

    install_list_setter(lua, &globals, "dependencies", |b| &mut b.dependencies)?;
    install_list_setter(lua, &globals, "shared_scripts", |b| &mut b.shared_scripts)?;
    install_list_setter(lua, &globals, "client_scripts", |b| &mut b.client_scripts)?;
    install_list_setter(lua, &globals, "server_scripts", |b| &mut b.server_scripts)?;
    install_list_setter(lua, &globals, "files", |b| &mut b.files)?;

    Ok(())
}

fn install_string_setter(
    lua: &Lua,
    globals: &Table,
    name: &str,
    apply: fn(&mut ManifestBuilder, String),
) -> mlua::Result<()> {
    let f = lua.create_function(move |lua, value: String| {
        let mut b = lua
            .app_data_mut::<ManifestBuilder>()
            .expect("ManifestBuilder app_data must be present");
        apply(&mut b, value);
        Ok(())
    })?;
    globals.set(name, f)
}

fn install_list_setter(
    lua: &Lua,
    globals: &Table,
    name: &str,
    field: fn(&mut ManifestBuilder) -> &mut Vec<String>,
) -> mlua::Result<()> {
    let f = lua.create_function(move |lua, table: Table| {
        let mut b = lua
            .app_data_mut::<ManifestBuilder>()
            .expect("ManifestBuilder app_data must be present");
        let target = field(&mut b);
        for item in table.sequence_values::<String>() {
            target.push(item?);
        }
        Ok(())
    })?;
    globals.set(name, f)
}
