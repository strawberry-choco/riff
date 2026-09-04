//! Frontend-local window visibility plumbing (Issue 03).
//!
//! The system tray runs on its own thread and cannot call into the egui
//! context directly. Instead it pushes a [`VisibilityMessage`] over an
//! unbounded crossbeam channel that [`RiffApp`] drains on every logic tick.
//! When the app is hidden (`eframe` 0.35 calls [`logic`] while hidden), the
//! same drain keeps the visibility state fresh without ever touching backend
//! state or the audio engine.
//!
//! This file is entirely frontend: no `riff_backend` import, no
//! [`crate::domain::PlaybackCommand`]. The tray ↔ UI loop is one of the two
//! frontend-only channels (the other being the tray quit flag).

use crossbeam_channel::Receiver;

/// A frontend-local request from the tray (or the close-to-tray path in
/// [`RiffApp`]) to flip the window's visible state.
///
/// `true` → show and focus; `false` → hide. The sender never constructs
/// backend commands — this keeps the tray path independent of the audio
/// engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisibilityMessage(pub bool);

/// Handle handed back by [`spawn_visibility_listener`] so the UI thread can
/// drain visibility requests between frames.
pub struct VisibilityListener {
    rx: Receiver<VisibilityMessage>,
}

impl VisibilityListener {
    /// Drain every pending visibility request in FIFO order.
    ///
    /// Multiple requests collapse to the last one (a rapid hide after a show
    /// wins), so this returns at most one message.
    pub fn drain(&self) -> Option<VisibilityMessage> {
        let mut last = None;
        while let Ok(msg) = self.rx.try_recv() {
            last = Some(msg);
        }
        last
    }
}

/// Frontend-local message type for the visibility channel.
pub type VisibilityTx = crossbeam_channel::Sender<VisibilityMessage>;

/// Build the frontend-local visibility channel pair. Callers pass the sender
/// to the tray thread (over `create_tray`) and keep the [`VisibilityListener`]
/// on [`RiffApp`], where the UI thread drains requests on every logic tick.
pub fn spawn_visibility_listener() -> (VisibilityTx, VisibilityListener) {
    let (tx, rx) = crossbeam_channel::unbounded();
    (tx, VisibilityListener { rx })
}
