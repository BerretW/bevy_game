// Drawable systém žije v core_drawable — re-exportujeme vše sem
// aby ostatní moduly (native_assets, main.rs) mohly i nadále používat
// `crate::drawable::DrawablePlugin` atd. bez změny.
pub use core_drawable::*;
