//! The screens the nsh frontend ships. N2 adds the one dev fixture; N3 adds
//! the debug bridge readout beside it; the real screens land with their
//! units (N4 the shell, N6 the library…).

#[cfg(debug_assertions)]
pub mod bridge_readout;
#[cfg(debug_assertions)]
pub mod dev_tokens;
