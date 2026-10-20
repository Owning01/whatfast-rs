//! Rust port of `tauri/tests/send-idempotency.test.ts` (frozen oracle).
//!
//! Every case of the TypeScript suite is here with the same name translated
//! to snake_case and the same assertions.
//!
//! port: the tuples of the oracle omit the `replyToChatId` element of the
//! payload (the bridge sends it as an empty string by default), so the port
//! fills that field with the empty string.

use whatfast_core::send_idempotency::{SendIdempotency, SendPayload};

/// The payloads of the oracle, in the same order.
fn payloads() -> Vec<SendPayload> {
    vec![
        SendPayload::Text {
            text: "hello".to_string(),
            reply_to_message_id: "reply-1".to_string(),
            reply_to_chat_id: String::new(),
            mentioned_jids: vec!["person@s.whatsapp.net".to_string()],
        },
        SendPayload::Image {
            image_path: "/tmp/photo.jpg".to_string(),
            caption: "caption".to_string(),
            reply_to_message_id: "reply-1".to_string(),
            reply_to_chat_id: String::new(),
        },
        SendPayload::Sticker {
            image_path: "/tmp/sticker.webp".to_string(),
            reply_to_message_id: "reply-1".to_string(),
            reply_to_chat_id: String::new(),
        },
        SendPayload::Attachment {
            file_path: "/tmp/notes.pdf".to_string(),
            kind: 1,
            caption: "caption".to_string(),
            reply_to_message_id: "reply-1".to_string(),
            reply_to_chat_id: String::new(),
            voice_note: false,
        },
        SendPayload::Poll {
            question: "Lunch?".to_string(),
            options: vec!["Pizza".to_string(), "Sushi".to_string()],
            selectable_options_count: 1,
        },
        SendPayload::Forward {
            source_chat_id: "source-chat".to_string(),
            message_id: "message-1".to_string(),
            target_chat_id: "target-chat".to_string(),
        },
    ]
}

/// The kind of a payload, the first element of the tuple of the oracle.
fn kind_of(payload: &SendPayload) -> &'static str {
    match payload {
        SendPayload::Text { .. } => "text",
        SendPayload::Image { .. } => "image",
        SendPayload::Sticker { .. } => "sticker",
        SendPayload::Attachment { .. } => "attachment",
        SendPayload::Poll { .. } => "poll",
        SendPayload::Forward { .. } => "forward",
    }
}

/// The base payload of a kind and the variants that change one element of it.
fn changed_payload_cases() -> Vec<(SendPayload, Vec<SendPayload>)> {
    let payloads = payloads();
    vec![
        (
            payloads[0].clone(),
            vec![
                SendPayload::Text {
                    text: "edited".to_string(),
                    reply_to_message_id: "reply-1".to_string(),
                    reply_to_chat_id: String::new(),
                    mentioned_jids: vec!["person@s.whatsapp.net".to_string()],
                },
                SendPayload::Text {
                    text: "hello".to_string(),
                    reply_to_message_id: "reply-2".to_string(),
                    reply_to_chat_id: String::new(),
                    mentioned_jids: vec!["person@s.whatsapp.net".to_string()],
                },
                SendPayload::Text {
                    text: "hello".to_string(),
                    reply_to_message_id: "reply-1".to_string(),
                    reply_to_chat_id: String::new(),
                    mentioned_jids: vec!["other@s.whatsapp.net".to_string()],
                },
            ],
        ),
        (
            payloads[1].clone(),
            vec![
                SendPayload::Image {
                    image_path: "/tmp/other-photo.jpg".to_string(),
                    caption: "caption".to_string(),
                    reply_to_message_id: "reply-1".to_string(),
                    reply_to_chat_id: String::new(),
                },
                SendPayload::Image {
                    image_path: "/tmp/photo.jpg".to_string(),
                    caption: "edited caption".to_string(),
                    reply_to_message_id: "reply-1".to_string(),
                    reply_to_chat_id: String::new(),
                },
                SendPayload::Image {
                    image_path: "/tmp/photo.jpg".to_string(),
                    caption: "caption".to_string(),
                    reply_to_message_id: "reply-2".to_string(),
                    reply_to_chat_id: String::new(),
                },
            ],
        ),
        (
            payloads[2].clone(),
            vec![
                SendPayload::Sticker {
                    image_path: "/tmp/other-sticker.webp".to_string(),
                    reply_to_message_id: "reply-1".to_string(),
                    reply_to_chat_id: String::new(),
                },
                SendPayload::Sticker {
                    image_path: "/tmp/sticker.webp".to_string(),
                    reply_to_message_id: "reply-2".to_string(),
                    reply_to_chat_id: String::new(),
                },
            ],
        ),
        (
            payloads[3].clone(),
            vec![
                SendPayload::Attachment {
                    file_path: "/tmp/other.pdf".to_string(),
                    kind: 1,
                    caption: "caption".to_string(),
                    reply_to_message_id: "reply-1".to_string(),
                    reply_to_chat_id: String::new(),
                    voice_note: false,
                },
                SendPayload::Attachment {
                    file_path: "/tmp/notes.pdf".to_string(),
                    kind: 2,
                    caption: "caption".to_string(),
                    reply_to_message_id: "reply-1".to_string(),
                    reply_to_chat_id: String::new(),
                    voice_note: false,
                },
                SendPayload::Attachment {
                    file_path: "/tmp/notes.pdf".to_string(),
                    kind: 1,
                    caption: "edited caption".to_string(),
                    reply_to_message_id: "reply-1".to_string(),
                    reply_to_chat_id: String::new(),
                    voice_note: false,
                },
                SendPayload::Attachment {
                    file_path: "/tmp/notes.pdf".to_string(),
                    kind: 1,
                    caption: "caption".to_string(),
                    reply_to_message_id: "reply-2".to_string(),
                    reply_to_chat_id: String::new(),
                    voice_note: false,
                },
                SendPayload::Attachment {
                    file_path: "/tmp/notes.pdf".to_string(),
                    kind: 1,
                    caption: "caption".to_string(),
                    reply_to_message_id: "reply-1".to_string(),
                    reply_to_chat_id: String::new(),
                    voice_note: true,
                },
            ],
        ),
        (
            payloads[5].clone(),
            vec![
                SendPayload::Forward {
                    source_chat_id: "other-source".to_string(),
                    message_id: "message-1".to_string(),
                    target_chat_id: "target-chat".to_string(),
                },
                SendPayload::Forward {
                    source_chat_id: "source-chat".to_string(),
                    message_id: "message-2".to_string(),
                    target_chat_id: "target-chat".to_string(),
                },
                SendPayload::Forward {
                    source_chat_id: "source-chat".to_string(),
                    message_id: "message-1".to_string(),
                    target_chat_id: "other-target".to_string(),
                },
            ],
        ),
    ]
}

#[test]
fn every_send_kind_reuses_its_id_for_a_failed_logical_retry() {
    for payload in payloads() {
        let mut sequence = 0u32;
        let mut ids = SendIdempotency::new(move || {
            sequence += 1;
            format!("request-{sequence}")
        });
        let mut observed: Vec<String> = Vec::new();

        let failed = ids.run("chat-1", &payload, |client_message_id| {
            observed.push(client_message_id.to_string());
            Err::<(), String>("response lost".to_string())
        });
        assert_eq!(failed.err(), Some("response lost".to_string()));
        let sent = ids.run("chat-1", &payload, |client_message_id| {
            observed.push(client_message_id.to_string());
            Ok::<(), String>(())
        });
        assert!(sent.is_ok());

        assert_eq!(
            observed[0],
            observed[1],
            "retry ID changed for {}",
            kind_of(&payload)
        );
    }
}

#[test]
fn a_confirmed_send_followed_by_the_same_payload_is_a_new_send() {
    let mut sequence = 0u32;
    let mut ids = SendIdempotency::new(move || {
        sequence += 1;
        format!("request-{sequence}")
    });
    let mut observed: Vec<String> = Vec::new();
    let payload = payloads().remove(0);

    let first = ids.run("chat-1", &payload, |client_message_id| {
        observed.push(client_message_id.to_string());
        Ok::<(), String>(())
    });
    assert!(first.is_ok());
    let second = ids.run("chat-1", &payload, |client_message_id| {
        observed.push(client_message_id.to_string());
        Ok::<(), String>(())
    });
    assert!(second.is_ok());

    assert_ne!(observed[0], observed[1]);
}

#[test]
fn changing_any_logical_payload_selects_a_fresh_id_after_failure() {
    for (base, variants) in changed_payload_cases() {
        for variant in variants {
            let mut sequence = 0u32;
            let mut ids = SendIdempotency::new(move || {
                sequence += 1;
                format!("request-{sequence}")
            });
            let mut failed_id = String::new();
            let mut changed_id = String::new();

            let failed = ids.run("chat-1", &base, |client_message_id| {
                failed_id = client_message_id.to_string();
                Err::<(), String>("response lost".to_string())
            });
            assert!(failed.is_err());
            let changed = ids.run("chat-1", &variant, |client_message_id| {
                changed_id = client_message_id.to_string();
                Ok::<(), String>(())
            });
            assert!(changed.is_ok());

            assert_ne!(
                failed_id,
                changed_id,
                "changed {} reused its prior ID",
                kind_of(&base)
            );
        }
    }
}
