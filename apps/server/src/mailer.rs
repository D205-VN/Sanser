//! Outgoing email for sign-in and sign-up verification codes.

use std::sync::Arc;

use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use tokio::sync::Mutex;

use crate::config::{ResendConfig, SmtpConfig, SmtpSecurity};

const RESEND_URL: &str = "https://api.resend.com/emails";
/// Shorter than the apps' 12-second request timeout, so an unreachable mail
/// service returns "unavailable" instead of the app giving up first.
const SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutgoingEmail {
    pub to: String,
    pub subject: String,
    pub body: String,
}

#[derive(Clone)]
pub enum Mailer {
    /// No mail service is configured; the server does not ask for codes.
    Disabled,
    Smtp(Box<SmtpMailer>),
    Resend(Box<ResendMailer>),
    /// Keeps messages in memory instead of sending them. For tests.
    Memory(Arc<Mutex<Vec<OutgoingEmail>>>),
}

#[derive(Clone)]
pub struct SmtpMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

#[derive(Clone)]
pub struct ResendMailer {
    client: reqwest::Client,
    config: ResendConfig,
}

/// JSON body for Resend's `POST /emails`.
pub fn resend_body(from: &str, email: &OutgoingEmail) -> serde_json::Value {
    serde_json::json!({
        "from": from,
        "to": [email.to],
        "subject": email.subject,
        "text": email.body,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum MailError {
    #[error("email is not configured")]
    Disabled,
    #[error("invalid email address")]
    Address,
    #[error("email delivery failed: {0}")]
    Delivery(String),
}

impl Mailer {
    pub fn from_config(
        resend: Option<&ResendConfig>,
        smtp: Option<&SmtpConfig>,
    ) -> Result<Self, MailError> {
        if let Some(config) = resend {
            let client = reqwest::Client::builder()
                .timeout(SEND_TIMEOUT)
                .user_agent(concat!("sanser-server/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|error| MailError::Delivery(error.to_string()))?;
            return Ok(Self::Resend(Box::new(ResendMailer {
                client,
                config: config.clone(),
            })));
        }
        let Some(config) = smtp else {
            return Ok(Self::Disabled);
        };
        let builder = match config.security {
            SmtpSecurity::StartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)
            }
            SmtpSecurity::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host),
            SmtpSecurity::None => Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(
                &config.host,
            )),
        }
        .map_err(|error| MailError::Delivery(error.to_string()))?;
        let mut builder = builder.port(config.port).timeout(Some(SEND_TIMEOUT));
        if let (Some(username), Some(password)) = (&config.username, config.password()) {
            builder = builder.credentials(Credentials::new(username.clone(), password.to_owned()));
        }
        let from = config.from.parse().map_err(|_| MailError::Address)?;
        Ok(Self::Smtp(Box::new(SmtpMailer {
            transport: builder.build(),
            from,
        })))
    }

    pub fn memory() -> (Self, Arc<Mutex<Vec<OutgoingEmail>>>) {
        let outbox = Arc::new(Mutex::new(Vec::new()));
        (Self::Memory(outbox.clone()), outbox)
    }

    pub const fn enabled(&self) -> bool {
        !matches!(self, Self::Disabled)
    }

    pub async fn send(&self, email: OutgoingEmail) -> Result<(), MailError> {
        match self {
            Self::Disabled => Err(MailError::Disabled),
            Self::Memory(outbox) => {
                outbox.lock().await.push(email);
                Ok(())
            }
            Self::Smtp(smtp) => {
                let SmtpMailer { transport, from } = smtp.as_ref();
                let to: Mailbox = email.to.parse().map_err(|_| MailError::Address)?;
                let message = Message::builder()
                    .from(from.clone())
                    .to(to)
                    .subject(email.subject)
                    .header(ContentType::TEXT_PLAIN)
                    .body(email.body)
                    .map_err(|error| MailError::Delivery(error.to_string()))?;
                transport
                    .send(message)
                    .await
                    .map(|_| ())
                    .map_err(|error| MailError::Delivery(error.to_string()))
            }
            Self::Resend(resend) => {
                let ResendMailer { client, config } = resend.as_ref();
                email
                    .to
                    .parse::<Mailbox>()
                    .map_err(|_| MailError::Address)?;
                let response = client
                    .post(RESEND_URL)
                    .bearer_auth(config.api_key())
                    .json(&resend_body(&config.from, &email))
                    .send()
                    .await
                    .map_err(|error| MailError::Delivery(error.without_url().to_string()))?;
                let status = response.status();
                if status.is_success() {
                    return Ok(());
                }
                // Resend explains rejections (unverified domain, bad key) in
                // the body; keep it short for the log.
                let detail: String = response
                    .text()
                    .await
                    .unwrap_or_default()
                    .chars()
                    .take(300)
                    .collect();
                Err(MailError::Delivery(format!(
                    "Resend answered {status}: {detail}"
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resend_body_has_plain_text_and_one_recipient() {
        let email = OutgoingEmail {
            to: "user@example.com".into(),
            subject: "Your Sanser code".into(),
            body: "123456".into(),
        };
        let body = resend_body("Sanser <no-reply@example.com>", &email);
        assert_eq!(body["from"], "Sanser <no-reply@example.com>");
        assert_eq!(body["to"], serde_json::json!(["user@example.com"]));
        assert_eq!(body["subject"], "Your Sanser code");
        assert_eq!(body["text"], "123456");
        assert!(body.get("html").is_none());
    }
}
