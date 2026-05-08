use bevy::asset::{io::Reader, AssetLoader, LoadContext};
use bevy::reflect::TypePath;
use thiserror::Error;

use crate::manifest::DrawableManifest;

#[derive(Default, TypePath)]
pub struct DrawableManifestLoader;

#[derive(Debug, Error)]
pub enum DrawableLoadError {
    #[error("IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("UTF-8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("TOML: {0}")]
    Toml(#[from] toml::de::Error),
}

impl AssetLoader for DrawableManifestLoader {
    type Asset = DrawableManifest;
    type Settings = ();
    type Error = DrawableLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _ctx: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).await?;
        Ok(toml::from_str(std::str::from_utf8(&buf)?)?)
    }

    fn extensions(&self) -> &[&str] {
        &["drawable"]
    }
}
