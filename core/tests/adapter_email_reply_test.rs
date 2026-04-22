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
    let headers_section = raw.split("\r\n\r\n").next().unwrap();
    let header_lines: Vec<&str> = headers_section.lines().collect();

    assert!(header_lines.iter().any(|l| *l == "From: alice@example.com"));
    assert!(header_lines.iter().any(|l| *l == "To: bob@example.com"));
    assert!(header_lines.iter().any(|l| *l == "Subject: Re: quote"));
    assert!(header_lines.iter().any(|l| *l == "In-Reply-To: <abc@orig>"));
    assert!(header_lines.iter().any(|l| *l == "References: <root@orig> <abc@orig>"));

    // Exactly one Message-ID, scoped to smtp host.
    let message_id_lines: Vec<&&str> = header_lines.iter().filter(|l| l.starts_with("Message-ID:")).collect();
    assert_eq!(message_id_lines.len(), 1, "expected exactly one Message-ID header");
    assert!(message_id_lines[0].ends_with("@smtp.example.com>"));
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

#[test]
fn rejects_header_injection_in_in_reply_to() {
    // If a caller (mistakenly, or maliciously) stuffs a CRLF + extra header
    // into the In-Reply-To value, wrap_angle must reject it (returns None)
    // so the In-Reply-To header is simply omitted rather than emitted with
    // an attacker-controlled follow-on header line.
    let content = sample_content(
        "Re: injection",
        "body",
        ReplyHeaders {
            to: "b@x".into(),
            in_reply_to: "abc@orig\r\nX-Evil: pwned".into(),
            references: vec!["abc@orig".into()],
        },
    );
    let msg = build_reply_message("a@x", &content, "smtp.x")
        .expect("build ok");
    let formatted = msg.formatted();
    let raw = std::str::from_utf8(&formatted).expect("utf-8");

    // No X-Evil header in the output.
    assert!(!raw.contains("X-Evil"), "header injection leaked");
    // No empty In-Reply-To either — the header should be omitted entirely
    // because wrap_angle rejects the malformed input.
    assert!(!raw.contains("In-Reply-To: \r\n"), "empty In-Reply-To emitted");
}

#[test]
fn drops_empty_references_header() {
    // If every entry of the references chain is malformed/empty, no
    // References header should be emitted (rather than an empty one).
    let content = sample_content(
        "Re: no refs",
        "body",
        ReplyHeaders {
            to: "b@x".into(),
            in_reply_to: "m@x".into(),
            references: vec!["".into(), "  ".into()], // all empty after trim
        },
    );
    let msg = build_reply_message("a@x", &content, "smtp.x")
        .expect("build ok");
    let formatted = msg.formatted();
    let raw = std::str::from_utf8(&formatted).expect("utf-8");
    assert!(!raw.contains("References:"), "empty References header emitted");
}
