//! File digest pro resource synchronizaci.
//!
//! Server před každým handshakem spočítá pro každý známý resource hash
//! (blake3) všech souborů, které do něj patří: `manifest.lua` + všechny
//! `shared_scripts` / `server_scripts` / `client_scripts` / `files`.
//!
//! Klient digest dostane v [`crate::protocol::ServerHello`], porovná
//! s lokální cache a stáhne, co chybí nebo se liší.
//!
//! Cesty v digestu jsou vždy normalizované na forward-slash (i na Windows),
//! aby digesty byly bit-shodné napříč platformami → stabilní cache klíče.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use core_resources::{Manifest, ResourceKind};

/// Hash + velikost jednoho souboru patřícího do resource.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileDigest {
    /// Cesta relativní k resource rootu (= rodič `manifest.lua`).
    /// Vždy forward-slash, žádné `..`, žádné absolute paths.
    pub rel_path: String,
    /// blake3 hash obsahu souboru (32 bajtů).
    pub blake3: [u8; 32],
    /// Velikost v bajtech (užitečné pro progress bar i jako sanity check).
    pub size: u64,
}

/// Wire-format obraz manifestu + hash všech jeho souborů.
///
/// Klient ho dostane od serveru a stačí mu k:
/// 1. rozhodnutí, které soubory potřebuje stáhnout (porovná `blob` s cache),
/// 2. lokálnímu zápisu `manifest.lua` (z polí níže ho umí znovu složit /
///    nebo prostě stáhne původní soubor přes HTTP).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceDigest {
    /// Kanonické ID resource (např. `"core/init"`).
    pub id: String,
    pub kind: ResourceKind,
    pub version: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub dependencies: Vec<String>,
    pub shared_scripts: Vec<String>,
    pub client_scripts: Vec<String>,
    pub server_scripts: Vec<String>,
    pub files: Vec<String>,
    /// Hash všech souborů, které tvoří fyzickou distribuci resource:
    /// `manifest.lua` + všechny scripts + assety. Setříděno podle `rel_path`,
    /// duplicity odstraněné — pole je tak deterministické a digestovatelné.
    pub blob: Vec<FileDigest>,
}

/// Souhrn všech resource digestů — užitečný i jako stand-alone JSON
/// (např. `GET /digest` na HTTP serveru pro debug).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceDigestSet {
    pub manifests: Vec<ResourceDigest>,
}

#[derive(Debug, thiserror::Error)]
pub enum DigestError {
    #[error("io error reading {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Spočítá digest pro jeden resource: přečte všechny soubory referencované
/// manifestem (+ samotný `manifest.lua`), spočítá blake3 a vrátí `ResourceDigest`.
///
/// Pokud některý soubor chybí, vrátí [`DigestError::Io`] — server takto pozná,
/// že má broken resource a může ho z handshake digestu vynechat.
pub fn compute_resource_digest(manifest: &Manifest) -> Result<ResourceDigest, DigestError> {
    // Posbírat všechny relativní cesty, které se mají zaheashovat.
    let mut rels: Vec<String> = Vec::new();
    rels.push("manifest.lua".to_string());
    rels.extend(manifest.shared_scripts.iter().cloned());
    rels.extend(manifest.client_scripts.iter().cloned());
    rels.extend(manifest.server_scripts.iter().cloned());
    rels.extend(manifest.files.iter().cloned());
    rels.extend(manifest.fonts.iter().map(|f| f.path.clone()));
    rels.extend(manifest.images.iter().map(|i| i.path.clone()));

    // Normalizovat (forward slashes), setřídit, odstranit duplicity.
    for r in rels.iter_mut() {
        *r = normalize_rel_path(r);
    }
    rels.sort();
    rels.dedup();

    let mut blob = Vec::with_capacity(rels.len());
    for rel in rels {
        // K `manifest.root` připojujeme platform-native verzi, pak hashujeme bytes.
        let abs = manifest.root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        let bytes = std::fs::read(&abs).map_err(|e| DigestError::Io {
            path: abs.clone(),
            source: e,
        })?;
        let hash = blake3::hash(&bytes);
        blob.push(FileDigest {
            rel_path: rel,
            blake3: *hash.as_bytes(),
            size: bytes.len() as u64,
        });
    }

    Ok(ResourceDigest {
        id: manifest.id.as_str().to_string(),
        kind: manifest.kind,
        version: manifest.version.clone(),
        author: manifest.author.clone(),
        description: manifest.description.clone(),
        dependencies: manifest.dependencies.clone(),
        shared_scripts: manifest.shared_scripts.iter().map(|s| normalize_rel_path(s)).collect(),
        client_scripts: manifest.client_scripts.iter().map(|s| normalize_rel_path(s)).collect(),
        server_scripts: manifest.server_scripts.iter().map(|s| normalize_rel_path(s)).collect(),
        files: manifest.files.iter().map(|s| normalize_rel_path(s)).collect(),
        blob,
    })
}

/// Normalizace relativní cesty: backslashe → slashe, žádné leading slashe.
/// Volání musí garantovat, že cesta je relativní; absolutní cesty `/foo`
/// se prostě obrátí na `foo` (vstup z manifestu by absolutní být neměl).
pub fn normalize_rel_path(p: &str) -> String {
    let s = p.replace('\\', "/");
    s.trim_start_matches('/').to_string()
}
