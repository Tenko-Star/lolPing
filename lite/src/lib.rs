//! lolPing Lite: Alt+? drops League's "?" ping on everyone's screen on the LAN.

pub mod anim;
pub mod limiter;
pub mod net;
pub mod node;
pub mod proto;
pub mod spot;
pub mod wav;
#[cfg(windows)]
pub mod win;
