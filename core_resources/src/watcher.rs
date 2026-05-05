//! Filesystem watcher pro `/resources/`.
//!
//! `notify` běží na vlastním vlákně a posílá události přes `crossbeam-channel`.
//! Bevy systém na main threadu události čerpá a pro každý netriviální event
//! vystaví marker `ResourcesDirty`. Konzument (rescan systém) zareaguje
//! a obnoví VFS + sandboxy.
//!
//! Granularita Phase 1: jakýkoliv `Modify`/`Create`/`Remove` event ⇒ flag dirty.
//! Phase 2 si můžeme dovolit cestově-cílené reload (jen affected resource).

use std::path::PathBuf;
use std::time::Duration;

use bevy::prelude::*;
use crossbeam_channel::{Receiver, TryRecvError};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// Bevy resource držící notify watcher live + RX kanál.
#[derive(Resource)]
pub struct VfsWatcher {
    rx: Receiver<notify::Result<Event>>,
    /// Watcher musí žít — když ho dropneme, watch thread končí.
    /// `Box<dyn Watcher>` neimplementuje `Send` na všech platformách v notify v6,
    /// ale `RecommendedWatcher` ano. Držíme ho přímo.
    _watcher: RecommendedWatcher,
    /// Jak dlouho po posledním eventu čekáme s flagováním "dirty"
    /// (manuální debounce — editor často při uložení produkuje ~3 eventy).
    debounce: Duration,
    last_event_at: Option<std::time::Instant>,
    pending: bool,
}

impl VfsWatcher {
    pub fn start(root: PathBuf) -> Result<Self, WatcherError> {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut watcher = notify::recommended_watcher(move |res| {
            // Když RX strana zmizela, ignorujeme — VfsWatcher resource byl dropnut.
            let _ = tx.send(res);
        })
        .map_err(WatcherError::Setup)?;

        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|e| WatcherError::Watch {
                path: root.clone(),
                source: e,
            })?;

        Ok(Self {
            rx,
            _watcher: watcher,
            debounce: Duration::from_millis(150),
            last_event_at: None,
            pending: false,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WatcherError {
    #[error("failed to initialize notify watcher: {0}")]
    Setup(#[source] notify::Error),
    #[error("failed to watch path {path:?}: {source}")]
    Watch {
        path: PathBuf,
        #[source]
        source: notify::Error,
    },
}

/// Marker event: VFS by se měl rescannout při příštím update kroku.
#[derive(Event, Debug, Clone, Copy)]
pub struct ResourcesDirty;

/// Drainuje frontu notify eventů, debouncuje a vystavuje `ResourcesDirty`.
///
/// Přidáno do `Update` schedule plug-inem — běží každý frame na main threadu.
pub(crate) fn drain_watcher(
    mut watcher: ResMut<VfsWatcher>,
    mut dirty_writer: EventWriter<ResourcesDirty>,
) {
    loop {
        match watcher.rx.try_recv() {
            Ok(Ok(event)) => {
                if is_meaningful(&event.kind) {
                    watcher.last_event_at = Some(std::time::Instant::now());
                    watcher.pending = true;
                }
            }
            Ok(Err(e)) => {
                warn!("[core_resources] watcher error: {e}");
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                error!("[core_resources] watcher channel disconnected");
                break;
            }
        }
    }

    if watcher.pending {
        let elapsed = watcher
            .last_event_at
            .map(|t| t.elapsed())
            .unwrap_or_default();
        if elapsed >= watcher.debounce {
            watcher.pending = false;
            watcher.last_event_at = None;
            dirty_writer.write(ResourcesDirty);
        }
    }
}

fn is_meaningful(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}
