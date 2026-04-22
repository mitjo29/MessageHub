//! Verify EmailAdapter::send_reply renders a threaded RFC 5322 message when
//! MessageContent.reply_headers is Some. We drive the adapter all the way to
//! building a lettre::Message and inspect its serialized bytes — no SMTP
//! transport is opened.
//!
//! The test relies on a #[cfg(test)] helper `build_reply_message` we add to
//! email.rs in the next task. Until that helper exists this file fails to
//! compile.

use messagehub_core::adapters::email::build_reply_message;
use messagehub_core::types::{MessageContent, ReplyHeaders};

fn sample_content(subject: &str, body: &str, headers: ReplyHeaders) -> MessageContent {
    MessageContent {
        text: Some(body.to_string()),
        html: None,
        subject: Some(subject.to_string()),
        attachments: Vec::new(),
        reply_headers: Some(headers),
    }
}

#[test]
fn renders_in_reply_to_and_references() {
    let content = sample_content(
        "Re: quote",
        "Thanks, sending the updated quote.\n",
        ReplyHeaders {
            to: "bob@example.com".into(),
            in_reply_to: "abc@orig".into(),
            references: vec!["root@orig".into(), "abc@orig".into()],
        },
    );

    let msg = build_reply_message(
        "alice@example.com",
        &content,
        "smtp.example.com",
    )
    .expect("build ok");

    let formatted = msg.formatted();
    let raw = std::str::from_utf8(&formatted).expect("utf-8");

    assert!(raw.contains("From: alice@example.com"));
    assert!(raw.contains("To: bob@example.com"));
    assert!(raw.contains("Subject: Re: quote"));
    assert!(raw.contains("In-Reply-To: <abc@orig>"));
    assert!(raw.contains("References: <root@orig> <abc@orig>"));
    // Lettre auto-generates a Message-ID; we only care it's present and
    // scoped to the SMTP host.
    assert!(raw.contains("Message-ID: <"));
    assert!(raw.contains("@smtp.example.com>"));
}

#[test]
fn dedupes_re_prefix() {
    let content = sample_content(
        "Re: already tagged",
        "body",
        ReplyHeaders {
            to: "b@x".into(),
            in_reply_to: "m@x".into(),
            references: vec!["m@x".into()],
        },
    );
    let msg = build_reply_message("a@x", &content, "smtp.x")
        .expect("build ok");
    let formatted = msg.formatted();
    let raw = std::str::from_utf8(&formatted).expect("utf-8");
    // Only one "Re: ".
    let re_count = raw.matches("Subject: Re: ").count();
    assert_eq!(re_count, 1, "Subject should have exactly one Re: prefix");
    assert!(!raw.contains("Subject: Re: Re: already tagged"));
}

#[test]
fn prepends_re_when_missing() {
    let content = sample_content(
        "plain subject",
        "body",
        ReplyHeaders {
            to: "b@x".into(),
            in_reply_to: "m@x".into(),
            references: vec!["m@x".into()],
        },
    );
    let msg = build_reply_message("a@x", &content, "smtp.x")
        .expect("build ok");
    let formatted = msg.formatted();
    let raw = std::str::from_utf8(&formatted).expect("utf-8");
    assert!(raw.contains("Subject: Re: plain subject"));
}
