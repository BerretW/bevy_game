//! VFS = registr všech známých resource manifestů.
//!
//! `Vfs` je Bevy `Resource`, žije po celou dobu života procesu.
//! Skenuje strom `/resources/` a najde všechny `manifest.lua`,
//! parsuje je a uloží do `manifests` mapy. Hot-reload provedeme
//! prostým rescanem (Phase 1 jednoduchá strategie).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bevy::prelude::*;
use walkdir::WalkDir;

use crate::manifest::{parse_manifest, Manifest, ManifestError};
use crate::types::{IdError, ResourceId};

#[derive(Resource, Debug)]
pub struct Vfs {
    /// Kořen, ve kterém hledáme `manifest.lua` soubory.
    root: PathBuf,
    manifests: HashMap<ResourceId, Manifest>,
}

impl Vfs {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            manifests: HashMap::new(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn manifests(&self) -> &HashMap<ResourceId, Manifest> {
        &self.manifests
    }

    pub fn get(&self, id: &ResourceId) -> Option<&Manifest> {
        self.manifests.get(id)
    }

    /// Iteruje strom a parsuje všechny nalezené manifesty.
    /// Vrací zvlášť úspěšně načtené ID a chyby, aby jeden vadný resource
    /// neshodil celý startup — víc resources se prostě "neaktivuje".
    pub fn rescan(&mut self) -> ScanReport {
        let mut report = ScanReport::default();
        self.manifests.clear();

        if !self.root.is_dir() {
            report.errors.push(ScanError::RootMissing(self.root.clone()));
            return report;
        }

        for entry in WalkDir::new(&self.root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_file() {
                continue;
            }
            if entry.file_name() != "manifest.lua" {
                continue;
            }

            let manifest_path = entry.path();
            let resource_root = match manifest_path.parent() {
                Some(p) => p,
                None => continue,
            };

            let id = match ResourceId::from_resource_root(&self.root, resource_root) {
                Ok(id) => id,
                Err(e) => {
                    report.errors.push(ScanError::Id(e));
                    continue;
                }
            };

            match parse_manifest(id.clone(), resource_root) {
                Ok(manifest) => {
                    report.loaded.push(id.clone());
                    self.manifests.insert(id, manifest);
                }
                Err(e) => {
                    report.errors.push(ScanError::Manifest(e));
                }
            }
        }

        report.loaded.sort();
        report
    }
}

#[derive(Debug, Default)]
pub struct ScanReport {
    pub loaded: Vec<ResourceId>,
    pub errors: Vec<ScanError>,
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("VFS root does not exist or is not a directory: {0:?}")]
    RootMissing(PathBuf),
    #[error(transparent)]
    Id(#[from] IdError),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
}
