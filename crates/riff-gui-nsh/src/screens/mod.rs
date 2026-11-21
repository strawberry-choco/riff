//! The screens the nsh frontend ships. N2 adds the one dev fixture; N3 adds
//! the debug bridge readout beside it; the real screens land with their
//! units (N6 the All Tracks stage and the inspector; N7 the drill-downs…).

#[cfg(debug_assertions)]
pub mod bridge_readout;
#[cfg(debug_assertions)]
pub mod dev_tokens;

pub mod inspector;
pub mod library;
pub mod now_playing;
pub mod playlists;
pub mod queue;
pub mod settings;
pub mod smart_playlists;
