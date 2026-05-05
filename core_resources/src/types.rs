//! Sdílené typy pro VFS / manifest / sandbox vrstvu.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Kanonický identifikátor resource — odvozeno z cesty relativně k VFS rootu.
///
/// `/resources/core/init/manifest.lua` → `ResourceId("core/init")`.
/// `/resources/survival/metabolism/manifest.lua` → `ResourceId("survival/metabolism")`.
///
/// Vždy s normalizovanými forward slashes, i na Windows.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ResourceId(String);

impl ResourceId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Odvodí ID z resource root cesty (= rodič `manifest.lua`) relativně k `vfs_root`.
    pub fn from_resource_root(vfs_root: &Path, resource_root: &Path) -> Result<Self, IdError> {
        let rel = resource_root.strip_prefix(vfs_root).map_err(|_| IdError::OutsideRoot {
            vfs_root: vfs_root.to_path_buf(),
            resource_root: resource_root.to_path_buf(),
        })?;

        let mut parts = Vec::new();
        for component in rel.components() {
            use std::path::Component;
            match component {
                Component::Normal(part) => {
                    let s = part.to_str().ok_or_else(|| IdError::NonUtf8(rel.to_path_buf()))?;
                    parts.push(s.to_string());
                }
                _ => return Err(IdError::InvalidPath(rel.to_path_buf())),
            }
        }

        if parts.is_empty() {
            return Err(IdError::EmptyId);
        }

        Ok(Self(parts.join("/")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Poslední segment (= jméno bez kategorie).
    pub fn short_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or(&self.0)
    }
}

impl fmt::Display for ResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IdError {
    #[error("resource path is outside VFS root: vfs_root={vfs_root:?}, resource_root={resource_root:?}")]
    OutsideRoot {
        vfs_root: PathBuf,
        resource_root: PathBuf,
    },
    #[error("resource path contains non-UTF8 segment: {0:?}")]
    NonUtf8(PathBuf),
    #[error("resource path contains invalid component (parent / prefix): {0:?}")]
    InvalidPath(PathBuf),
    #[error("resource id would be empty (manifest.lua at vfs root?)")]
    EmptyId,
}

/// Která strana sítě sandbox hostí — určuje, které skripty z manifestu se nahrají.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Side {
    Server,
    Client,
}

impl Side {
    pub fn label(self) -> &'static str {
        match self {
            Side::Server => "server",
            Side::Client => "client",
        }
    }
}
