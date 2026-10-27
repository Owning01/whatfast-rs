//! The identity reconciliation: one chat per person and the agenda names.
//!
//! WhatsApp speaks of one person through two jids — the phone number and
//! the hidden identity the server assigns — and the phone's agenda lives in
//! the fork's session database. This pass folds the twin conversations into
//! one chat keyed by the phone number, teaches the store the agenda names,
//! and tells the desktop which chats changed.

use std::collections::HashSet;
use std::sync::Arc;

use async_channel::Sender;
use whatfast_protocol::BackendEvent;

use crate::db::Store;
use crate::identity::Identity;
use crate::ingest::chat_upserted;

/// The one chat row a jid belongs to, merging the twins it finds.
///
/// The phone-number jid wins when the map knows the pair: it is what the
/// user reads and what the agenda keys on. Group and newsletter jids pass
/// through untouched: their ids are their own.
///
/// Answers the canonical jid and the folds it performed, so the caller can
/// tell the desktop which rows retired.
///
/// The port: the Go picked the hidden-identity chat as the winner and stored
/// it as the preferred address of an opaque chat id. This store keys a chat
/// by its address, so the winner has to be the address the contacts table and
/// the user-facing identity use: the phone number. The chat_merge events and
/// the redirects carry the rest.
pub fn canonical_chat(
    store: &Store,
    identity: &Identity,
    jid: &str,
) -> (String, Vec<(String, String)>) {
    let jid = jid.trim();
    if !jid.ends_with("@lid") && !jid.ends_with("@s.whatsapp.net") {
        return (jid.to_owned(), Vec::new());
    }
    let aliases = identity.aliases(jid);
    let existing = store.chats_of_addresses(&aliases).unwrap_or_default();
    let winner = existing
        .iter()
        .find(|chat| chat.ends_with("@s.whatsapp.net"))
        .cloned()
        .or_else(|| {
            aliases
                .iter()
                .find(|alias| alias.ends_with("@s.whatsapp.net"))
                .cloned()
        })
        .or_else(|| existing.first().cloned())
        .unwrap_or_else(|| jid.to_owned());
    if let Err(error) = store.ensure_chat(&winner, &winner) {
        tracing::warn!("ensuring the canonical chat {winner}: {error}");
        return (jid.to_owned(), Vec::new());
    }
    let mut losers: HashSet<String> = existing
        .iter()
        .filter(|chat| **chat != winner)
        .cloned()
        .collect();
    // A chat row that still carries an alias as its id is a loser even when
    // no address points at it: the Go's resolveChatTx looks at the row
    // first, and leaving that row behind would split the person in two and
    // never converge.
    for alias in &aliases {
        if *alias != winner && matches!(store.chat(alias), Ok(Some(_))) {
            losers.insert(alias.clone());
        }
    }
    for alias in &aliases {
        if let Ok(Some((old, new))) = store.adopt_address(alias, &winner)
            && old != winner
            && new == winner
        {
            losers.insert(old);
        }
    }
    // A pass that started before another one folded the pair finds the loser
    // gone: there is nothing to move and nothing to announce. Without this
    // guard both passes announce the same fold, and the second merge trips on
    // the missing row.
    losers.retain(|loser| loser != &winner && matches!(store.chat(loser), Ok(Some(_))));
    let mut folds = Vec::new();
    for loser in losers {
        if let Err(error) = store.merge_chats(&winner, &loser) {
            tracing::warn!("merging chat {loser} into {winner}: {error}");
            continue;
        }
        folds.push((loser, winner.clone()));
    }
    (winner, folds)
}

/// Teaches the store the phone's contact names. Answers the jids whose
/// contact row changed.
///
/// The pass runs on every connection, every finished history and every
/// contact update the phone sends, over the whole agenda (the real profile
/// carries 1344 contact rows). A row that already says what the agenda says
/// is left alone: the write transaction it would take is what made the pass
/// expensive, not the read.
pub fn learn_names(store: &Store, identity: &Identity) -> Vec<String> {
    let mut changed = Vec::new();
    for contact in identity.phone_contacts() {
        let name = if contact.full_name.is_empty() {
            &contact.first_name
        } else {
            &contact.full_name
        };
        if name.is_empty() && contact.push_name.is_empty() && contact.business_name.is_empty() {
            continue;
        }
        for jid in identity.aliases(&contact.jid) {
            if !is_stale(
                store,
                &jid,
                name,
                &contact.push_name,
                &contact.business_name,
            ) {
                continue;
            }
            if store
                .learn_contact(&jid, name, &contact.push_name, &contact.business_name)
                .unwrap_or(false)
            {
                changed.push(jid);
            }
        }
    }
    changed
}

/// Whether the stored contact row still has to learn something the agenda
/// knows. An empty agenda field never erases a stored one, so it never counts
/// as news.
fn is_stale(
    store: &Store,
    jid: &str,
    contact_name: &str,
    push_name: &str,
    business_name: &str,
) -> bool {
    let Ok(Some((stored_name, stored_push, stored_business, _))) = store.contact(jid) else {
        return true;
    };
    let name_matches = contact_name.is_empty() || stored_name == contact_name;
    let push_matches = push_name.is_empty() || stored_push == push_name;
    let business_matches = business_name.is_empty() || stored_business == business_name;
    !(name_matches && push_matches && business_matches)
}

/// The full pass: every hidden-identity chat folds into its phone-number
/// twin, the agenda names land in the store, and the desktop hears which
/// chats changed and which rows retired.
pub async fn reconcile(store: &Arc<Store>, identity: &Identity, events: &Sender<BackendEvent>) {
    let mut seen: HashSet<String> = HashSet::new();
    for jid in store.lid_chats().unwrap_or_default() {
        let (_, folds) = canonical_chat(store, identity, &jid);
        for (old, new) in folds {
            if seen.insert(old.clone()) {
                let _ = events
                    .send(BackendEvent {
                        sequence: 0,
                        event: Some(whatfast_protocol::backend_event::Event::ChatMerged(
                            whatfast_protocol::ChatMerged {
                                old_chat_id: old,
                                new_chat_id: new,
                            },
                        )),
                    })
                    .await;
            }
        }
    }
    for jid in learn_names(store, identity) {
        let Ok(Some(chat)) = store.chat(&jid) else {
            continue;
        };
        if seen.insert(chat.jid.clone()) {
            let _ = events.send(chat_upserted(chat)).await;
        }
    }
}
