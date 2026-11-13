//! Email adapter implementations of `IEmailSender` (embyr-core::admin::email).
//!
//! - `SmtpEmailSender`: real delivery over SMTP (lettre), used when
//!   `EMBYR_SMTP_URL` and `EMBYR_EMAIL_FROM` are both set.
//! - `NoopEmailSender`: re-exported from embyr-core; drops the message. Used
//!   when neither variable is set (local/dev), with a startup warning.

use std::sync::Arc;

use embyr_core::admin::email::{EmailError, EmailMessage, IEmailSender};
use lettre::message::{Mailbox, MultiPart};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

pub use embyr_core::admin::email::NoopEmailSender;

/// Sends mail through one SMTP relay. The transport pools connections.
pub struct SmtpEmailSender {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl SmtpEmailSender {
    /// `smtp_url` is a lettre connection URL, e.g.
    /// `smtps://user:pass@smtp.example.com:465` (implicit TLS),
    /// `smtp://user:pass@smtp.example.com:587?tls=required` (STARTTLS), or
    /// `smtp://localhost:1025` (plaintext, local relays only).
    /// `from` is an RFC 5322 mailbox, e.g. `Embyr <no-reply@example.com>`.
    pub fn new(smtp_url: &str, from: &str) -> Result<Self, String> {
        // Never echo the URL in errors: it carries the SMTP password.
        let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(smtp_url)
            .map_err(|e| format!("EMBYR_SMTP_URL is not a valid SMTP URL: {e}"))?
            .build();
        let from = from
            .parse()
            .map_err(|e| format!("EMBYR_EMAIL_FROM is not a valid mailbox: {e}"))?;
        Ok(Self { transport, from })
    }
}

fn build_message(from: &Mailbox, message: EmailMessage) -> Result<Message, EmailError> {
    let to: Mailbox = message
        .to
        .parse()
        .map_err(|e| EmailError::DeliveryFailed(format!("invalid recipient address: {e}")))?;
    let builder = Message::builder().from(from.clone()).to(to).subject(message.subject);
    match message.body_html {
        Some(html) => builder.multipart(MultiPart::alternative_plain_html(message.body_text, html)),
        None => builder.body(message.body_text),
    }
    .map_err(|e| EmailError::DeliveryFailed(e.to_string()))
}

impl IEmailSender for SmtpEmailSender {
    fn send(
        &self,
        message: EmailMessage,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), EmailError>> + Send + '_>>
    {
        Box::pin(async move {
            let email = build_message(&self.from, message)?;
            self.transport
                .send(email)
                .await
                .map(|_| ())
                .map_err(|e| EmailError::DeliveryFailed(e.to_string()))
        })
    }
}

/// Pick the email sender from `EMBYR_SMTP_URL` / `EMBYR_EMAIL_FROM`.
///
/// Both set → SMTP. Neither set → no-op (emails are dropped). Exactly one
/// set, or an unparseable value → `Err`, so a half-configured deployment
/// fails at startup instead of silently never delivering invites or
/// password resets.
pub fn sender_from_settings(
    smtp_url: Option<&str>,
    from: Option<&str>,
) -> Result<Arc<dyn IEmailSender + Send + Sync>, String> {
    match (smtp_url, from) {
        (Some(url), Some(from)) => Ok(Arc::new(SmtpEmailSender::new(url, from)?)),
        (None, None) => Ok(Arc::new(NoopEmailSender)),
        (Some(_), None) => Err("EMBYR_SMTP_URL is set but EMBYR_EMAIL_FROM is not".into()),
        (None, Some(_)) => Err("EMBYR_EMAIL_FROM is set but EMBYR_SMTP_URL is not".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(html: Option<&str>) -> EmailMessage {
        EmailMessage {
            to: "dana@example.com".into(),
            subject: "You're invited".into(),
            body_text: "plain body".into(),
            body_html: html.map(Into::into),
        }
    }

    #[test]
    fn builds_plain_and_multipart_messages() {
        let from: Mailbox = "Embyr <no-reply@example.com>".parse().unwrap();
        let plain = String::from_utf8(build_message(&from, msg(None)).unwrap().formatted()).unwrap();
        assert!(plain.contains("To: dana@example.com") && plain.contains("plain body"));
        let multi =
            String::from_utf8(build_message(&from, msg(Some("<b>hi</b>"))).unwrap().formatted())
                .unwrap();
        assert!(multi.contains("multipart/alternative") && multi.contains("<b>hi</b>"));
    }

    #[test]
    fn invalid_recipient_is_a_delivery_error_not_a_panic() {
        let from: Mailbox = "no-reply@example.com".parse().unwrap();
        let mut m = msg(None);
        m.to = "not an address".into();
        assert!(build_message(&from, m).is_err());
    }

    // `Arc<dyn IEmailSender>` isn't Debug, so `.unwrap_err()` won't compile.
    fn err(url: Option<&str>, from: Option<&str>) -> String {
        sender_from_settings(url, from).err().expect("expected a configuration error")
    }

    #[tokio::test]
    async fn settings_select_sender_and_reject_half_configuration() {
        assert!(sender_from_settings(None, None).is_ok());
        assert!(sender_from_settings(Some("smtp://localhost:1025"), Some("a@example.com")).is_ok());
        assert!(err(Some("smtp://localhost:1025"), None).contains("EMBYR_EMAIL_FROM"));
        assert!(err(None, Some("a@example.com")).contains("EMBYR_SMTP_URL"));
        assert!(err(Some("not a url"), Some("a@example.com")).contains("EMBYR_SMTP_URL"));
        let secret = err(Some("smtp://u:hunter2@localhost"), Some("not a mailbox"));
        assert!(!secret.contains("hunter2"), "error must not leak the SMTP password");
    }
}
