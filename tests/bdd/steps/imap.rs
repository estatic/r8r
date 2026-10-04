//! Delivers test mail into GreenMail (`r8r-bdd-imap`, SMTP 127.0.0.1:3025,
//! IMAP 127.0.0.1:3143, auth disabled -- any user/password works, and a
//! mailbox is created on first delivery to it) for the Email Trigger
//! (IMAP) node's `@requires-imap` scenarios. Uses `lettre` directly
//! (already a dependency for the Send Email node) rather than going
//! through r8r itself, so these steps work independently of the node
//! under test.

use crate::world::R8rWorld;
use cucumber::when;
use lettre::message::{Attachment, MultiPart, SinglePart};
use lettre::transport::smtp::client::Tls;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

const GREENMAIL_SMTP_HOST: &str = "127.0.0.1";
const GREENMAIL_SMTP_PORT: u16 = 3025;
const TEST_SENDER: &str = "bdd-sender@r8r.test";

async fn deliver(to: &str, subject: &str, body: &str, attachment: Option<(&str, &str)>) {
    let builder = Message::builder()
        .from(TEST_SENDER.parse().unwrap_or_else(|e| panic!("invalid sender address: {e}")))
        .to(to.parse().unwrap_or_else(|e| panic!("invalid test email address {to:?}: {e}")))
        .subject(subject.to_string());
    let message = match attachment {
        None => builder.body(body.to_string()).unwrap_or_else(|e| panic!("could not build test email: {e}")),
        Some((file_name, content)) => {
            let mixed = MultiPart::mixed().singlepart(SinglePart::plain(body.to_string())).singlepart(
                Attachment::new(file_name.to_string())
                    .body(content.as_bytes().to_vec(), "text/plain".parse().unwrap_or_else(|e| panic!("invalid content type: {e}"))),
            );
            builder.multipart(mixed).unwrap_or_else(|e| panic!("could not build test email: {e}"))
        }
    };
    let transport = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(GREENMAIL_SMTP_HOST).port(GREENMAIL_SMTP_PORT).tls(Tls::None).build();
    transport.send(message).await.unwrap_or_else(|e| panic!("could not deliver test mail via GreenMail SMTP at {GREENMAIL_SMTP_HOST}:{GREENMAIL_SMTP_PORT}: {e}"));
}

#[when(expr = "I deliver a test email to {string} with subject {string} and body {string}")]
async fn deliver_step(w: &mut R8rWorld, to: String, subject: String, body: String) {
    let to = w.expand(&to);
    let subject = w.expand(&subject);
    let body = w.expand(&body);
    deliver(&to, &subject, &body, None).await;
}

#[when(expr = "I deliver a test email to {string} with subject {string} and body {string} and an attachment {string} with content {string}")]
async fn deliver_with_attachment_step(w: &mut R8rWorld, to: String, subject: String, body: String, file_name: String, content: String) {
    let to = w.expand(&to);
    let subject = w.expand(&subject);
    let body = w.expand(&body);
    deliver(&to, &subject, &body, Some((&file_name, &content))).await;
}
