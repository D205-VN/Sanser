//! Outgoing email for sign-in and sign-up verification codes.

use std::sync::Arc;

use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use tokio::sync::Mutex;

use crate::config::{SmtpConfig, SmtpSecurity};

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
    /// Keeps messages in memory instead of sending them. For tests.
    Memory(Arc<Mutex<Vec<OutgoingEmail>>>),
}

#[derive(Clone)]
pub struct SmtpMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
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
    pub fn from_config(config: Option<&SmtpConfig>) -> Result<Self, MailError> {
        let Some(config) = config else {
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
        let mut builder = builder
            .port(config.port)
            .timeout(Some(std::time::Duration::from_secs(15)));
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
        }
    }
}
