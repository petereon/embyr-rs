//! IEmailSender port trait and supporting types.
//!
//! V1: NoopEmailSender (logs and returns Ok).
//! V2: SmtpEmailSender (lettre — future slice).
//! Tests: FakeEmailSender (in tests/admin_api_v2/common/mod.rs).

/// A message to be sent by the email port.
#[derive(Debug, Clone)]
pub struct EmailMessage {
    pub to: String,
    pub subject: String,
    pub body_text: String,
    pub body_html: Option<String>,
}

/// Error type for the email port.
#[derive(Debug)]
pub enum EmailError {
    /// Delivery failure (SMTP, network, etc.).
    DeliveryFailed(String),
}

impl std::fmt::Display for EmailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmailError::DeliveryFailed(msg) => write!(f, "email delivery failed: {}", msg),
        }
    }
}

/// Driven port: email delivery.
///
/// # Object Safety
/// This trait uses `async fn` via the `async-trait` pattern (manual `Pin<Box<...>>`).
/// Adapters must implement `send` with the correct signature.
pub trait IEmailSender: Send + Sync {
    fn send(
        &self,
        message: EmailMessage,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), EmailError>> + Send + '_>,
    >;
}

/// Noop email sender — V1. Logs and returns Ok. No external dependency.
///
/// # Probe
/// Always returns Ok (no external substrate to probe).
pub struct NoopEmailSender;

impl IEmailSender for NoopEmailSender {
    fn send(
        &self,
        message: EmailMessage,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), EmailError>> + Send + '_>,
    > {
        Box::pin(async move {
            // V1: no delivery. Drop the message silently.
            let _ = message;
            Ok(())
        })
    }
}
