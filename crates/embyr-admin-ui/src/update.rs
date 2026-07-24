//! Pure update function — the heart of the TEA loop.
//!
//! Contract:
//!   - No IO, no async, no tokio::spawn, no println!
//!   - Deterministic: same (model, msg) → same new model state
//!   - Exhaustive: all Msg variants handled (enforced by compiler)
//!
//! Walking skeleton: catch-all match — no-op for all messages until
//! individual user stories are implemented in subsequent slices.

use crate::model::AppModel;
use crate::msg::Msg;

/// Apply `msg` to `model`, mutating in place.
///
/// Walking skeleton implementation: all variants are no-ops.
/// Individual variants are implemented as each user story's slice is delivered.
pub fn update(_model: &mut AppModel, msg: Msg) {
    match msg {
        _ => {}
    }
}
