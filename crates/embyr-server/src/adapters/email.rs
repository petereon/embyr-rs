//! Email adapter implementations.
//!
//! NoopEmailSender (V1): re-exported from embyr-core; logs and returns Ok.
//! SmtpEmailSender (V2): lettre-based async SMTP client (future slice).
//!
//! Both implement IEmailSender from embyr-core::admin::email.

// V1: re-export the NoopEmailSender from embyr-core (no extra dependency).
pub use embyr_core::admin::email::NoopEmailSender;

/// SMTP email sender — V2 (lettre). Not yet implemented.
pub struct SmtpEmailSender;

impl SmtpEmailSender {
    pub fn new_scaffold() -> Self {
        panic!("Not yet implemented -- RED scaffold: SmtpEmailSender requires lettre (V2 slice)")
    }
}
