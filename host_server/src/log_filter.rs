//! Custom `tracing` layer, který tlumí Windows-specific UDP ICMP spam.
//!
//! Po odpojení klienta Windows posílá `ICMP destination unreachable` zpět
//! na server, OS to překládá na `WSAECONNRESET` (os error 10054) a
//! `lightyear_udp::server` to logguje na ERROR — opakovaně, dokud se
//! soket nepročistí. Linux/macOS tenhle ICMP tiše ignorují, problém je
//! čistě Windows.
//!
//! Reálný fix je `SIO_UDP_CONNRESET` ioctl na soketu, ale lightyear/aeronet
//! ho neaplikují a soket je v privátním fieldu — bez upstream patche se k
//! němu nedostaneme. Filtrujeme tedy log: na **ERROR** úrovni z modulu
//! `lightyear_udp::server` zahodíme. Nižší úrovně (INFO `Server UDP socket
//! bound to ...`, WARN, …) projdou nedotčené.
//!
//! Filter je instalovaný jako `LogPlugin::custom_layer`. `Layer::enabled()`
//! vrácené `false` pro daný event tlumí ho i v níže-stackovaných layers
//! (tracing-subscriber sémantika), takže formatter ho nikdy nezformátuje.

use bevy::log::tracing::{Metadata, Subscriber};
use bevy::log::tracing::Level;
use bevy::log::tracing_subscriber::layer::Context;
use bevy::log::tracing_subscriber::Layer;

/// Targets, které spamují WSAECONNRESET. Když lightyear přidá další transport
/// (websocket, webtransport), může ho být potřeba doplnit.
const NOISY_TARGETS: &[&str] = &["lightyear_udp::server", "lightyear_udp"];

pub struct LightyearUdpNoiseFilter;

impl<S> Layer<S> for LightyearUdpNoiseFilter
where
    S: Subscriber,
{
    fn enabled(&self, metadata: &Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        // Pustíme všechno, co není ERROR.
        if *metadata.level() != Level::ERROR {
            return true;
        }
        // ERROR z lightyear_udp targets zahodíme.
        let target = metadata.target();
        !NOISY_TARGETS.iter().any(|t| target == *t || target.starts_with(&format!("{}::", t)))
    }
}
