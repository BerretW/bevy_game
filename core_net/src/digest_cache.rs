//! Bevy plugin, který drží předpočítaný digest všech resources serveru.
//!
//! Server ho potřebuje, aby mohl po `lightyear` connectu klienta okamžitě
//! poslat `ServerHello` se seznamem manifestů a hashů. Recomputujeme jen
//! při změně VFS (initial scan + hot-reload), ne každý frame.
//!
//! `compute_resource_digest` čte soubory z disku — to je blokující I/O,
//! ale běží jen občas (na startu + při file watch eventu) a `/resources/`
//! mívá řádově desítky souborů. Až bude potřeba, převedeme na
//! `IoTaskPool` (sledujeme jako Phase 4 follow-up).

use bevy::prelude::*;
use core_resources::{ServerResourceAllowlist, Vfs};

use crate::digest::{compute_resource_digest, ResourceDigestSet};

/// Aktuální digest všech resources, jak je server nabízí klientům.
///
/// `generation` se inkrementuje při každém recomputu — handshake systém
/// může čekat na `generation > 0`, aby nikdy neposlal prázdný digest
/// hned po startu, kdy ještě nedoběhl initial scan.
#[derive(Resource, Debug, Default, Clone)]
pub struct ResourceDigestCache {
    set: ResourceDigestSet,
    generation: u64,
}

impl ResourceDigestCache {
    pub fn set(&self) -> &ResourceDigestSet {
        &self.set
    }
    pub fn manifests(&self) -> &[crate::digest::ResourceDigest] {
        &self.set.manifests
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn is_ready(&self) -> bool {
        self.generation > 0
    }
}

/// Server-side plugin. Klient si digest nepočítá — dostane ho v `ServerHello`.
pub struct DigestPlugin;

impl Plugin for DigestPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ResourceDigestCache>();
        // Po `Vfs::rescan` Bevy fires change detection na `Res<Vfs>` —
        // náš systém zareaguje a recomputne digest.
        app.add_systems(Update, refresh_on_vfs_change);
    }
}

fn refresh_on_vfs_change(
    vfs: Res<Vfs>,
    allowlist: Res<ServerResourceAllowlist>,
    mut cache: ResMut<ResourceDigestCache>,
) {
    // Zatím i initial Startup load zachytíme — Vfs::new vytvoří prázdnou,
    // potom Startup `initial_load` system zavolá rescan a tím změní stav.
    // V dalším Update tickovém kroku tu projdeme s `is_changed() == true`.
    if !vfs.is_changed() {
        return;
    }

    let mut manifests: Vec<&core_resources::Manifest> = vfs.manifests().values().collect();
    if let Some(allowed) = allowlist.ids.as_ref() {
        manifests.retain(|manifest| allowed.contains(&manifest.id));
    }
    manifests.sort_by(|a, b| a.id.cmp(&b.id));

    let mut set = ResourceDigestSet::default();
    let mut errors = 0usize;
    for manifest in manifests {
        match compute_resource_digest(manifest) {
            Ok(d) => set.manifests.push(d),
            Err(e) => {
                warn!(
                    "[core_net] digest failed for {} — resource will not be advertised: {}",
                    manifest.id, e
                );
                errors += 1;
            }
        }
    }

    cache.set = set;
    cache.generation = cache.generation.saturating_add(1);

    info!(
        "[core_net] digest cache refreshed (generation {}, {} resource(s){})",
        cache.generation,
        cache.set.manifests.len(),
        if errors > 0 {
            format!(", {errors} error(s)")
        } else {
            String::new()
        }
    );
}
