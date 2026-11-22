//! The chat store of the client: the state the user interface renders.
//!
//! port: taken from the origin's `desktop/src/model.rs`. The only change is
//! the toolkit: GPUI's `Pixels`, `Size`, `px` and `size` became
//! `crate::geom`'s, and the protobuf DTOs come from `whatfast-protocol`
//! instead of the origin's private `proto` module.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use crate::geom::{Pixels, Size, px, size};

pub const CHAT_PAGE_SIZE: u32 = 100;
pub const MESSAGE_PAGE_SIZE: u32 = 50;
pub const MAX_ACTIVE_MESSAGES: usize = 2_000;

/// The filters of the column of chats.
///
/// port: the `ChatFilter` type of the origin (`tauri/src/state/app.ts`):
/// `all`, `unread`, `favorites`, `groups`, `archived` and `channels`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum ChatFilter {
    /// Every conversation that is not archived.
    #[default]
    All,
    /// The conversations with messages that were not read.
    Unread,
    /// The conversations marked as favorites.
    Favorites,
    /// The group conversations.
    Groups,
    /// Conversations containing @mentions.
    Mentions,
    /// The archived conversations.
    ///
    /// The archived conversations have their own destination in the column of
    /// icons, so this filter is not one of the chips of the column of chats.
    Archived,
}

impl ChatFilter {
    /// Every filter, in the order the column draws them.
    ///
    /// The archived conversations are not in this list: they have their own
    /// destination in the column of icons, the same way the reference draws
    /// them.
    pub const ALL: [Self; 5] = [
        Self::All,
        Self::Unread,
        Self::Favorites,
        Self::Groups,
        Self::Mentions,
    ];

    /// The label of the filter, in the language of the client.
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Unread => "Unread",
            Self::Favorites => "Favorites",
            Self::Groups => "Groups",
            Self::Mentions => "Mentions",
            Self::Archived => "Archived",
        }
    }

    /// Whether the filter keeps one conversation.
    ///
    /// port: the filter of `filteredChats` of the origin: the archived
    /// conversations only appear under the filter of the archived, and the
    /// rest of the filters leave them out.
    pub fn includes(self, chat: &whatfast_protocol::Chat) -> bool {
        if is_hidden_conversation(chat) {
            return false;
        }
        match self {
            Self::Archived => chat.archived,
            Self::All => !chat.archived,
            Self::Unread => !chat.archived && chat.unread_count > 0,
            Self::Groups => !chat.archived && chat.kind() == whatfast_protocol::ChatKind::Group,
            Self::Mentions => false,
            Self::Favorites => false,
        }
    }
}

/// Whether the conversation is one of the special ones that the column of
/// chats does not show.
///
/// port: `isHiddenConversation` of the origin (`tauri/src/state/app.ts`): the
/// broadcast of the states and the channels of the newsletters are not
/// conversations with a person, so they live in their own view.
pub fn is_hidden_conversation(chat: &whatfast_protocol::Chat) -> bool {
    let transport = chat.transport_jid.as_str();
    transport == "status@broadcast" || transport.ends_with("@newsletter")
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Screen {
    #[default]
    Starting,
    Pairing,
    Syncing,
    Chats,
    Fatal,
}

/// One phone call the desktop is party to. The phase strings are the
/// sidecar's contract: placing, dialing, ringing, incoming, connecting,
/// active, ended.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CallState {
    pub call_id: String,
    pub chat_id: String,
    pub phase: String,
    pub muted: bool,
    /// True when the call came from the peer: the overlay shows answer and
    /// decline instead of mute and hang up.
    pub incoming: bool,
    /// True when the call carries video in either direction: the overlay
    /// shows the panel of the camera and of the peer.
    pub video: bool,
}

impl CallState {
    /// Records the id that the backend gave to the call being placed.
    ///
    /// Every later event carries that id: without it the call never leaves
    /// the `placing` phase, the audio never starts, and the hang up does not
    /// find the call.
    pub fn placed(&mut self, call_id: String) {
        if self.call_id.is_empty() {
            self.call_id = call_id;
            if self.phase == "placing" {
                self.phase = "dialing".into();
            }
        }
    }
}

#[derive(Default)]
pub struct Store {
    pub call: Option<CallState>,
    pub screen: Screen,
    pub connection: whatfast_protocol::ConnectionState,
    pub connection_detail: String,
    pub sync_chats: u64,
    pub sync_messages: u64,
    pub sync_active: bool,
    pub sync_complete: bool,
    pub qr_code: Option<String>,
    pub qr_expires_at_ms: i64,
    pub chats: Vec<whatfast_protocol::Chat>,
    pub total_chats: u64,
    pub next_chat_cursor: String,
    pub selected_chat_id: Option<String>,
    pub messages: Vec<whatfast_protocol::Message>,
    pub has_older_messages: bool,
    pub has_newer_messages: bool,
    pub newer_activity: bool,
    pub fatal_error: Option<String>,
    pub toast_error: Option<String>,
    /// A short notice of something that went well, in the green of the theme.
    pub toast_info: Option<String>,
    pub last_mark_read_id: Option<String>,
    /// The progress of the synchronization of the account.
    ///
    /// port: the origin keeps this in the state of the web client
    /// (`state/app.ts`, `applySyncStatus`). It arrives with `GetSyncStatus`
    /// and with the `SyncStatusChanged` event.
    pub sync_status: Option<whatfast_protocol::SyncStatus>,
    /// The summary of the history of every conversation of the account.
    pub history_overview: Option<whatfast_protocol::HistoryOverview>,
    /// The health of the local database of the sidecar.
    pub integrity_status: Option<whatfast_protocol::IntegrityStatus>,
    /// The coverage of the history, one entry per conversation.
    pub history_coverage: HashMap<String, whatfast_protocol::HistoryCoverage>,
    /// The pinned messages, one list per conversation.
    pub pinned_messages: HashMap<String, Vec<whatfast_protocol::PinnedMessage>>,
    /// The messages the profile starred, by conversation and message id.
    ///
    /// The linked device does not expose the star of WhatsApp (it belongs to
    /// the app-state of the account), so the star lives in the profile, the
    /// same way the favorites of the conversations do. The conversation is
    /// the outer key so a lookup borrows both ids instead of building a pair
    /// of `String`s on every frame (`message_is_starred` runs per message).
    starred_messages: HashMap<String, HashSet<String>>,
    /// Whether a media file exists, resolved once when a message or a
    /// download enters the model. The render loop reads this map and never
    /// asks the file system: `is_file()` in the paint path is a syscall on
    /// the UI thread, and Defender or a busy disk turns it into frame drops.
    media_presence: RefCell<HashMap<String, bool>>,
    /// The library of stickers, grouped in packs.
    pub sticker_packs: Vec<whatfast_protocol::StickerPack>,
    /// The statuses of the profile, grouped by their sender.
    pub statuses: Vec<whatfast_protocol::StatusGroup>,
    /// The conversation that carries the statuses.
    pub statuses_chat_id: String,
    /// The newsletters the account is subscribed to.
    pub newsletters: Vec<whatfast_protocol::Newsletter>,
    /// The posts of each newsletter.
    pub newsletter_messages: HashMap<String, Vec<whatfast_protocol::Message>>,
    chat_index: HashMap<String, usize>,
    /// The filter of the column of chats.
    chat_filter: ChatFilter,
    /// The ids of the conversations the filter keeps, in the order of the
    /// column. The list is rebuilt when the chats or the filter change, so
    /// drawing the column does not allocate.
    visible_chat_ids: Rc<Vec<String>>,
    /// The conversations the user marked as favorites.
    ///
    /// port: the origin keeps them in the local storage of the browser
    /// (`tauri/src/state/app.ts`, the key `whatfast-favorites`): the linked
    /// device has no RPC for them, so they belong to the profile.
    favorite_chat_ids: HashSet<String>,
    message_index: HashMap<String, usize>,
    reply_metadata: HashMap<String, ReplyMetadata>,
    message_sizes: Rc<Vec<Size<Pixels>>>,
    measured_width_bucket: Option<i32>,
    message_height_cache: HashMap<i32, HashMap<String, Pixels>>,
    participant_avatar_paths: HashMap<String, String>,
    loaded_later_chat_page: bool,
}

#[derive(Default)]
struct ReplyMetadata {
    count: usize,
    first_reply_id: String,
}

impl Store {
    pub fn clear_account_state(&mut self) {
        *self = Self::default();
    }

    pub fn selected_chat(&self) -> Option<&whatfast_protocol::Chat> {
        let id = self.selected_chat_id.as_deref()?;
        self.chats.iter().find(|chat| chat.id == id)
    }

    pub fn chat(&self, id: &str) -> Option<&whatfast_protocol::Chat> {
        self.chat_index
            .get(id)
            .and_then(|index| self.chats.get(*index))
    }

    /// The ids of the conversations that the filter keeps, in the order of
    /// the column.
    ///
    /// port: `filteredChats` of the origin (`tauri/src/state/app.ts`): the
    /// states and the channels are not conversations, so they never show up
    /// in the list, and the archived ones only show up under their filter.
    pub fn chat_ids(&self, filter: ChatFilter) -> Rc<Vec<String>> {
        if filter == self.chat_filter {
            return self.visible_chat_ids.clone();
        }
        Rc::new(
            self.chats
                .iter()
                .filter(|chat| self.filter_keeps(filter, chat))
                .map(|chat| chat.id.clone())
                .collect(),
        )
    }

    /// Whether one filter keeps one conversation.
    fn filter_keeps(&self, filter: ChatFilter, chat: &whatfast_protocol::Chat) -> bool {
        match filter {
            ChatFilter::Favorites => !chat.archived && self.favorite_chat_ids.contains(&chat.id),
            ChatFilter::Mentions => !chat.archived && self.chat_has_mention(&chat.id),
            other => other.includes(chat),
        }
    }

    fn chat_has_mention(&self, chat_id: &str) -> bool {
        self.messages.iter().any(|m| {
            m.chat_id == chat_id
                && !m.from_me
                && matches!(
                    &m.content,
                    Some(whatfast_protocol::message::Content::Text(t)) if t.text.contains('@')
                )
        }) || self
            .chats
            .iter()
            .find(|c| c.id == chat_id)
            .is_some_and(|c| c.last_message_preview.contains('@'))
    }

    /// The filter of the column of chats.
    pub fn chat_filter(&self) -> ChatFilter {
        self.chat_filter
    }

    /// Changes the filter of the column of chats.
    pub fn set_chat_filter(&mut self, filter: ChatFilter) {
        if self.chat_filter == filter {
            return;
        }
        self.chat_filter = filter;
        self.rebuild_chat_ids();
    }

    /// Whether the conversation is one of the favorites.
    pub fn is_favorite(&self, chat_id: &str) -> bool {
        self.favorite_chat_ids.contains(chat_id)
    }

    /// Marks a conversation as a favorite, or takes the mark away.
    ///
    /// Returns whether the conversation is a favorite after the change.
    pub fn toggle_favorite(&mut self, chat_id: &str) -> bool {
        if !self.favorite_chat_ids.remove(chat_id) {
            self.favorite_chat_ids.insert(chat_id.to_owned());
        }
        self.rebuild_chat_ids();
        self.favorite_chat_ids.contains(chat_id)
    }

    /// Replaces the set of favorites, for the profile that loaded it.
    pub fn set_favorite_chat_ids(&mut self, favorites: HashSet<String>) {
        self.favorite_chat_ids = favorites;
        self.rebuild_chat_ids();
    }

    /// The set of conversations marked as favorites.
    pub fn favorite_chat_ids(&self) -> &HashSet<String> {
        &self.favorite_chat_ids
    }

    pub fn apply_sync_progress(&mut self, chats: u64, messages: u64, complete: bool) {
        self.sync_chats = self.sync_chats.saturating_add(chats);
        self.sync_messages = self.sync_messages.saturating_add(messages);
        // A sync that already completed stays completed: a late progress
        // event with `complete = false` (a second batch, a retry) used to
        // take the window back to "importing" and never finish.
        if complete {
            self.mark_sync_ready();
        } else if !self.sync_complete {
            self.sync_active = true;
        }
    }

    /// Marks the initial import as done.
    ///
    /// The sounds and the toasts of the messages wait for this mark. A
    /// resumed account never receives the history again, so waiting for the
    /// `complete` event of a chunk leaves them silent forever: the caller
    /// marks the sync ready when the account already has its conversations.
    pub fn mark_sync_ready(&mut self) {
        self.sync_active = false;
        self.sync_complete = true;
    }

    pub fn message(&self, id: &str) -> Option<&whatfast_protocol::Message> {
        self.message_index
            .get(id)
            .and_then(|index| self.messages.get(*index))
    }

    pub fn reply_count(&self, message_id: &str) -> usize {
        self.reply_metadata
            .get(message_id)
            .map_or(0, |metadata| metadata.count)
    }

    pub fn first_reply_id(&self, message_id: &str) -> Option<&str> {
        self.reply_metadata
            .get(message_id)
            .map(|metadata| metadata.first_reply_id.as_str())
    }

    pub fn select_chat(&mut self, id: String) {
        self.selected_chat_id = Some(id);
        self.messages.clear();
        self.has_older_messages = false;
        self.has_newer_messages = false;
        self.newer_activity = false;
        self.message_index.clear();
        self.reply_metadata.clear();
        self.message_sizes = Rc::new(Vec::new());
        self.measured_width_bucket = None;
        self.last_mark_read_id = None;
    }

    pub fn merge_chat_id(&mut self, old_id: &str, new_id: &str) -> bool {
        self.chats.retain(|chat| chat.id != old_id);
        self.reindex_chats();
        if self.selected_chat_id.as_deref() == Some(old_id) {
            self.selected_chat_id = Some(new_id.to_owned());
            return true;
        }
        false
    }

    pub fn upsert_chat(&mut self, mut chat: whatfast_protocol::Chat) {
        let old_index = self.chat_index.remove(&chat.id);
        // El avatar es una foto local: un evento que vuelve a mandar el chat
        // sin el camino no borra la que ya se tiene.
        if chat.avatar_path.is_empty()
            && let Some(index) = old_index
            && let Some(existing) = self.chats.get(index)
        {
            chat.avatar_path = existing.avatar_path.clone();
        }
        if let Some(index) = old_index {
            self.chats.remove(index);
        }
        let index = self
            .chats
            .binary_search_by(|existing| compare_chats(existing, &chat))
            .unwrap_or_else(|index| index);
        self.chats.insert(index, chat);
        let start = old_index.map_or(index, |old| old.min(index));
        let end = old_index.map_or(self.chats.len(), |old| old.max(index).saturating_add(1));
        for item_index in start..end.min(self.chats.len()) {
            self.chat_index
                .insert(self.chats[item_index].id.clone(), item_index);
        }
        self.rebuild_chat_ids();
    }

    pub fn upsert_message(&mut self, mut message: whatfast_protocol::Message) {
        if self.selected_chat_id.as_deref() != Some(message.chat_id.as_str()) {
            return;
        }
        // Un mensaje eliminado no se dibuja: el hilo lo salta. El backend
        // conserva la fila para la historia; la ventana no la pinta.
        if message_is_hidden(&message) {
            let chat_id = message.chat_id.clone();
            let id = message.id.clone();
            self.remove_message(&chat_id, &id);
            return;
        }
        self.remember_message_media(&message);
        for heights in self.message_height_cache.values_mut() {
            heights.remove(&message.id);
        }
        let old_index = self.message_index.remove(&message.id);
        if let Some(index) = old_index {
            preserve_local_image_path(&mut message, &self.messages[index]);
            preserve_sender_details(&mut message, &self.messages[index]);
            self.messages.remove(index);
        }
        let index = self
            .messages
            .binary_search_by(|existing| compare_messages(existing, &message))
            .unwrap_or_else(|index| index);
        self.messages.insert(index, message);
        if self.trim_after_live_append() {
            self.reindex_messages();
        } else {
            let start = old_index.map_or(index, |old| old.min(index));
            let end = old_index.map_or(self.messages.len(), |old| old.max(index).saturating_add(1));
            self.reindex_message_range(start, end);
        }
        self.rebuild_reply_metadata();
        self.rebuild_message_sizes();
    }

    pub fn prepend_messages(&mut self, incoming: Vec<whatfast_protocol::Message>) {
        self.merge_messages(incoming);
        if self.messages.len() > MAX_ACTIVE_MESSAGES {
            self.messages.truncate(MAX_ACTIVE_MESSAGES);
            self.has_newer_messages = true;
        }
        self.reindex_messages();
        self.rebuild_reply_metadata();
        self.rebuild_message_sizes();
    }

    pub fn append_newer_messages(
        &mut self,
        incoming: Vec<whatfast_protocol::Message>,
        has_more: bool,
    ) {
        self.merge_messages(incoming);
        if self.messages.len() > MAX_ACTIVE_MESSAGES {
            let excess = self.messages.len() - MAX_ACTIVE_MESSAGES;
            self.messages.drain(..excess);
            self.has_older_messages = true;
        }
        self.has_newer_messages = has_more || self.newer_activity;
        // The page was applied: the flag of the events that it already covers
        // goes back down. What it did not cover stays as `has_newer_messages`,
        // which is what the renderer reads to keep offering the next page.
        self.newer_activity = false;
        self.reindex_messages();
        self.rebuild_reply_metadata();
        self.rebuild_message_sizes();
    }

    /// Merges one page of the thread into the window that is on screen.
    ///
    /// A page is already sorted, and so is the window: the new rows are
    /// ordered among themselves and the two runs merge in one pass. Rebuilding
    /// a map of every message and sorting the whole window again turned each
    /// page of a scroll into O(n log n) work with a clone of every id; this
    /// leaves the order of the window alone and touches only the page.
    ///
    /// The caller reindexes after this: the positions move when the page
    /// lands between rows that were already there.
    fn merge_messages(&mut self, incoming: Vec<whatfast_protocol::Message>) {
        let mut fresh: Vec<whatfast_protocol::Message> = Vec::with_capacity(incoming.len());
        let mut removed: Vec<String> = Vec::new();
        for mut message in incoming {
            for heights in self.message_height_cache.values_mut() {
                heights.remove(&message.id);
            }
            // Un mensaje eliminado no entra al hilo, ni siquiera en la
            // historia que se pide al abrir la conversacion.
            if message_is_hidden(&message) {
                removed.push(message.id);
                continue;
            }
            self.remember_message_media(&message);
            if let Some(index) = self.message_index.get(&message.id).copied() {
                if let Some(existing) = self.messages.get(index) {
                    preserve_local_image_path(&mut message, existing);
                }
                self.messages[index] = message;
            } else {
                fresh.push(message);
            }
        }
        if !removed.is_empty() {
            self.messages
                .retain(|message| !removed.contains(&message.id));
        }
        if !fresh.is_empty() {
            fresh.sort_by(compare_messages);
            let existing = std::mem::take(&mut self.messages);
            let mut merged = Vec::with_capacity(existing.len() + fresh.len());
            let mut existing = existing.into_iter().peekable();
            let mut fresh = fresh.into_iter().peekable();
            loop {
                let take_fresh = match (existing.peek(), fresh.peek()) {
                    (Some(left), Some(right)) => compare_messages(left, right).is_gt(),
                    (Some(_), None) => false,
                    (None, Some(_)) => true,
                    (None, None) => break,
                };
                if take_fresh {
                    if let Some(message) = fresh.next() {
                        merged.push(message);
                    }
                } else if let Some(message) = existing.next() {
                    merged.push(message);
                }
            }
            self.messages = merged;
        }
        // A row that was already there keeps its place; if the page changed
        // its timestamp the window is sorted again, which is the rare case.
        if !self
            .messages
            .is_sorted_by(|a, b| !compare_messages(a, b).is_gt())
        {
            self.messages.sort_by(compare_messages);
        }
    }

    pub fn replace_message_window(
        &mut self,
        mut messages: Vec<whatfast_protocol::Message>,
        has_older: bool,
        has_newer: bool,
    ) {
        // Lo que ya venia eliminado no se dibuja.
        messages.retain(|message| !message_is_hidden(message));
        for message in &messages {
            self.remember_message_media(message);
        }
        messages.sort_by(compare_messages);
        if messages.len() > MAX_ACTIVE_MESSAGES {
            messages.drain(..messages.len() - MAX_ACTIVE_MESSAGES);
        }
        self.messages = messages;
        self.has_older_messages = has_older;
        self.has_newer_messages = has_newer;
        self.newer_activity = false;
        self.reindex_messages();
        self.rebuild_reply_metadata();
        self.rebuild_message_sizes();
    }

    pub fn update_receipt(&mut self, receipt: whatfast_protocol::ReceiptUpdated) {
        if let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.id == receipt.message_id && message.chat_id == receipt.chat_id)
        {
            // The receipts of the phone arrive out of order: a late
            // `Delivered` must not take a message back from `Read`.
            if receipt_advances(message.status(), receipt.status()) {
                message.status = receipt.status;
            }
        }
    }

    pub fn mark_selected_chat_read_locally(&mut self) -> Option<u32> {
        let id = self.selected_chat_id.as_deref()?;
        if let Some(chat) = self.chats.iter_mut().find(|chat| chat.id == id) {
            let previous = chat.unread_count;
            chat.unread_count = 0;
            return Some(previous);
        }
        None
    }

    pub fn restore_unread(&mut self, chat_id: &str, unread: u32) {
        if let Some(chat) = self.chats.iter_mut().find(|chat| chat.id == chat_id) {
            chat.unread_count = unread;
        }
    }

    /// Marks a conversation as unread.
    ///
    /// port: `markChatUnread` of the origin (`tauri/src/state/app.ts`): the
    /// count goes up by one and never goes below one.
    pub fn mark_chat_unread(&mut self, chat_id: &str) {
        if let Some(chat) = self.chats.iter_mut().find(|chat| chat.id == chat_id) {
            chat.unread_count = chat.unread_count.saturating_add(1).max(1);
        }
    }

    /// Clears the count of unread messages of a conversation.
    ///
    /// port: `markChatRead` of the origin: the count goes to zero while the
    /// backend confirms it.
    pub fn mark_chat_read_locally(&mut self, chat_id: &str) -> Option<u32> {
        let chat = self.chats.iter_mut().find(|chat| chat.id == chat_id)?;
        let previous = chat.unread_count;
        chat.unread_count = 0;
        Some(previous)
    }

    pub fn set_chat_avatar(&mut self, chat_id: &str, avatar_path: String) {
        // The avatar is read by `media_present` on every paint: resolve it
        // once now, as the paths of messages do.
        self.refresh_media_path(&avatar_path);
        if let Some(chat) = self
            .chat_index
            .get(chat_id)
            .and_then(|index| self.chats.get_mut(*index))
        {
            chat.avatar_path = avatar_path;
        }
    }

    pub fn set_participant_avatar(&mut self, participant_id: String, avatar_path: String) {
        self.refresh_media_path(&avatar_path);
        self.participant_avatar_paths
            .insert(participant_id, avatar_path);
    }

    pub fn set_message_image_paths(
        &mut self,
        chat_id: &str,
        message_id: &str,
        image_path: String,
        thumbnail_path: String,
    ) -> bool {
        // The download just told the model where the file is: resolve its
        // presence here, once, and not in the paint of every frame.
        self.refresh_media_path(&image_path);
        self.refresh_media_path(&thumbnail_path);
        let mut updated = false;
        for group in &mut self.statuses {
            for message in &mut group.messages {
                if message.id == message_id
                    && let Some(whatfast_protocol::message::Content::Image(image)) =
                        message.content.as_mut()
                {
                    image.local_path = image_path.clone();
                    image.thumbnail_path = thumbnail_path.clone();
                    updated = true;
                }
            }
        }
        for pack in &mut self.sticker_packs {
            for sticker in &mut pack.stickers {
                if sticker.source_chat_id == chat_id && sticker.source_message_id == message_id {
                    sticker.local_path = image_path.clone();
                    updated = true;
                }
            }
        }
        let Some(message) = self
            .message_index
            .get(message_id)
            .and_then(|index| self.messages.get_mut(*index))
            .filter(|message| message.chat_id == chat_id)
        else {
            return updated;
        };
        let Some(whatfast_protocol::message::Content::Image(image)) = message.content.as_mut()
        else {
            return updated;
        };
        if image.local_path == image_path && image.thumbnail_path == thumbnail_path {
            return updated;
        }
        image.local_path = image_path;
        image.thumbnail_path = thumbnail_path;
        true
    }

    /// Answers whether a media file exists, from the map the model filled
    /// when the message arrived or the download ended.
    ///
    /// A path the model never saw answers `true`: the element that paints the
    /// picture shows its own fallback if the file is not there. The important
    /// half is that this never touches the file system.
    pub fn media_exists(&self, path: &str) -> bool {
        if path.is_empty() {
            return false;
        }
        self.media_presence
            .borrow()
            .get(path)
            .copied()
            .unwrap_or(true)
    }

    /// Resolves the presence of one media path and keeps the answer.
    ///
    /// Called when a message enters the model (history, live event, status or
    /// download). The file system is read here, in the event, never in the
    /// render of a frame.
    pub fn remember_media_path(&self, path: &str) {
        if path.is_empty() {
            return;
        }
        let mut presence = self.media_presence.borrow_mut();
        if !presence.contains_key(path) {
            presence.insert(path.to_owned(), std::path::Path::new(path).is_file());
        }
    }

    /// Resolves one media path again and overwrites the answer.
    ///
    /// A download that just wrote the file, or a path that was cleared, must
    /// not keep the answer of the previous state. Like [`Self::remember_media_path`],
    /// this reads the file system in the event, never in the render of a frame.
    pub fn refresh_media_path(&self, path: &str) {
        if path.is_empty() {
            return;
        }
        self.media_presence
            .borrow_mut()
            .insert(path.to_owned(), std::path::Path::new(path).is_file());
    }

    /// Forgets every answered path: the media cache was emptied and the files
    /// may be gone. The next message that enters the model fills the map
    /// again.
    pub fn forget_media_presence(&self) {
        self.media_presence.borrow_mut().clear();
    }

    /// Resolves the presence of every file one message paints.
    fn remember_message_media(&self, message: &whatfast_protocol::Message) {
        match message.content.as_ref() {
            Some(whatfast_protocol::message::Content::Image(image)) => {
                self.remember_media_path(&image.local_path);
                self.remember_media_path(&image.thumbnail_path);
            }
            Some(whatfast_protocol::message::Content::Attachment(attachment)) => {
                self.remember_media_path(&attachment.local_path);
            }
            _ => {}
        }
    }

    pub fn participant_avatar_path(&self, participant_id: &str) -> Option<&str> {        self.participant_avatar_paths
            .get(participant_id)
            .map(String::as_str)
            .filter(|path| !path.is_empty())
    }

    /// Forgets every avatar path this session knows.
    ///
    /// The media cache was cleared and the files are gone: a path that
    /// survives would paint a hole. The next paint finds the empty path and
    /// asks the backend for the picture again.
    pub fn forget_avatars(&mut self) {
        for chat in &mut self.chats {
            chat.avatar_path.clear();
        }
        self.participant_avatar_paths.clear();
        for message in &mut self.messages {
            message.sender_avatar_path.clear();
        }
    }

    pub fn apply_reaction(&mut self, reaction: whatfast_protocol::Reaction, removed: bool) {
        let Some(index) = self.message_index.get(&reaction.message_id).copied() else {
            return;
        };
        let message = &mut self.messages[index];
        if message.chat_id != reaction.chat_id {
            return;
        }
        if message.reactions.iter().any(|existing| {
            existing.sender_id == reaction.sender_id
                && existing.timestamp_ms > reaction.timestamp_ms
        }) {
            return;
        }
        message
            .reactions
            .retain(|existing| existing.sender_id != reaction.sender_id);
        if !removed && !reaction.emoji.is_empty() {
            message.reactions.push(reaction);
            message.reactions.sort_by(|left, right| {
                (left.timestamp_ms, left.sender_id.as_str())
                    .cmp(&(right.timestamp_ms, right.sender_id.as_str()))
            });
        }
        for heights in self.message_height_cache.values_mut() {
            heights.remove(&message.id);
        }
        self.measured_width_bucket = None;
    }

    pub fn message_sizes(&self) -> Rc<Vec<Size<Pixels>>> {
        self.message_sizes.clone()
    }

    pub fn message_sizes_need_measurement(&self, width_bucket: i32) -> bool {
        self.measured_width_bucket != Some(width_bucket)
            || self.message_sizes.len() != self.messages.len()
    }

    pub fn set_measured_message_sizes(&mut self, width_bucket: i32, sizes: Vec<Size<Pixels>>) {
        debug_assert_eq!(sizes.len(), self.messages.len());
        self.message_sizes = Rc::new(sizes);
        self.measured_width_bucket = Some(width_bucket);
        self.message_height_cache
            .retain(|bucket, _| *bucket == width_bucket);
    }

    pub fn cached_message_height(&self, id: &str, width_bucket: i32) -> Option<Pixels> {
        self.message_height_cache
            .get(&width_bucket)
            .and_then(|heights| heights.get(id))
            .copied()
    }

    pub fn cache_message_height(&mut self, id: String, width_bucket: i32, height: Pixels) {
        self.message_height_cache
            .entry(width_bucket)
            .or_default()
            .insert(id, height);
    }

    pub fn invalidate_all_message_heights(&mut self) {
        self.message_height_cache.clear();
        self.measured_width_bucket = None;
    }

    pub fn replace_chat_page(
        &mut self,
        cursor: &str,
        chats: Vec<whatfast_protocol::Chat>,
        total_count: u64,
        next_cursor: String,
    ) {
        let mut by_id: HashMap<String, whatfast_protocol::Chat> = self
            .chats
            .drain(..)
            .map(|chat| (chat.id.clone(), chat))
            .collect();
        // El avatar es una foto local: el backend nunca manda el camino, asi
        // que la pagina nueva no borra la que ya se tiene.
        for mut chat in chats {
            if chat.avatar_path.is_empty()
                && let Some(existing) = by_id.get(&chat.id)
            {
                chat.avatar_path = existing.avatar_path.clone();
            }
            by_id.insert(chat.id.clone(), chat);
        }
        self.chats = by_id.into_values().collect();
        self.chats.sort_by(compare_chats);
        self.reindex_chats();
        self.total_chats = total_count;
        if cursor.is_empty() {
            if !self.loaded_later_chat_page {
                self.next_chat_cursor = next_cursor;
            }
        } else {
            self.loaded_later_chat_page = true;
            self.next_chat_cursor = next_cursor;
        }
    }

    fn trim_after_live_append(&mut self) -> bool {
        if self.messages.len() > MAX_ACTIVE_MESSAGES {
            let excess = self.messages.len() - MAX_ACTIVE_MESSAGES;
            self.messages.drain(..excess);
            self.has_older_messages = true;
            return true;
        }
        false
    }

    /// Applies the status of the synchronization of the account.
    ///
    /// port: the origin applies a status only when its revision is not older
    /// than the one it already has (`shouldApplySyncStatus` in
    /// `state/sync.ts`). The port keeps the same rule.
    ///
    /// Returns whether the status was applied.
    pub fn apply_sync_status(&mut self, status: whatfast_protocol::SyncStatus) -> bool {
        if let Some(current) = &self.sync_status
            && status.revision < current.revision
        {
            return false;
        }
        // The status panel and the first sync tell the same fact: a phase of
        // complete is the mark that the sounds and the toasts wait for. The
        // status arrives on its own (`GetSyncStatus`, `SyncStatusChanged`),
        // so a sync that completed in a previous run does not leave the
        // window waiting for a chunk that never comes.
        if status.phase == whatfast_protocol::SyncPhase::Complete as i32 {
            self.mark_sync_ready();
        }
        self.sync_status = Some(status);
        true
    }

    /// Keeps the summary of the history of the account.
    pub fn set_history_overview(&mut self, overview: whatfast_protocol::HistoryOverview) {
        self.history_overview = Some(overview);
    }

    /// Keeps the health of the local database of the sidecar.
    pub fn set_integrity_status(&mut self, status: whatfast_protocol::IntegrityStatus) {
        self.integrity_status = Some(status);
    }

    /// Keeps the coverage of the history of one conversation.
    ///
    /// port: the origin only replaces a coverage with one that is not older
    /// than the one it has (`coverage.revision >= current.revision`).
    ///
    /// Returns whether the coverage was applied.
    pub fn set_history_coverage(&mut self, coverage: whatfast_protocol::HistoryCoverage) -> bool {
        let chat_id = coverage.chat_id.clone();
        if chat_id.is_empty() {
            return false;
        }
        if let Some(current) = self.history_coverage.get(&chat_id)
            && coverage.revision < current.revision
        {
            return false;
        }
        self.history_coverage.insert(chat_id, coverage);
        true
    }

    /// The coverage of the history of one conversation.
    pub fn history_coverage(&self, chat_id: &str) -> Option<&whatfast_protocol::HistoryCoverage> {
        self.history_coverage.get(chat_id)
    }

    /// Keeps the pinned messages of one conversation.
    pub fn set_pinned_messages(
        &mut self,
        chat_id: String,
        pins: Vec<whatfast_protocol::PinnedMessage>,
    ) {
        self.pinned_messages.insert(chat_id, pins);
    }

    /// The pinned messages of one conversation.
    pub fn pinned_messages(&self, chat_id: &str) -> &[whatfast_protocol::PinnedMessage] {
        self.pinned_messages.get(chat_id).map_or(&[], Vec::as_slice)
    }

    /// Whether one message of one conversation is pinned.
    pub fn message_is_pinned(&self, chat_id: &str, message_id: &str) -> bool {
        self.pinned_messages
            .get(chat_id)
            .is_some_and(|pins| pins.iter().any(|pin| pin.message_id == message_id))
    }

    /// The messages the profile starred, grouped by conversation.
    pub fn starred_messages(&self) -> &HashMap<String, HashSet<String>> {
        &self.starred_messages
    }

    /// Replaces the starred messages, when the file of the profile is read.
    pub fn set_starred_messages(&mut self, starred: HashMap<String, HashSet<String>>) {
        self.starred_messages = starred;
    }

    /// Whether one message of one conversation is starred.
    pub fn message_is_starred(&self, chat_id: &str, message_id: &str) -> bool {
        self.starred_messages
            .get(chat_id)
            .is_some_and(|messages| messages.contains(message_id))
    }

    /// Stars or unstars one message. Returns the new state.
    pub fn toggle_starred_message(&mut self, chat_id: &str, message_id: &str) -> bool {
        if let Some(messages) = self.starred_messages.get_mut(chat_id) {
            if messages.remove(message_id) {
                if messages.is_empty() {
                    self.starred_messages.remove(chat_id);
                }
                return false;
            }
            messages.insert(message_id.to_owned());
            return true;
        }
        self.starred_messages
            .insert(chat_id.to_owned(), HashSet::from([message_id.to_owned()]));
        true
    }

    /// Keeps the library of stickers.
    pub fn set_sticker_packs(&mut self, packs: Vec<whatfast_protocol::StickerPack>) {
        self.sticker_packs = packs;
    }

    /// The library of stickers.
    pub fn sticker_packs(&self) -> &[whatfast_protocol::StickerPack] {
        &self.sticker_packs
    }

    /// Keeps the statuses of the profile and the conversation that carries
    /// them.
    pub fn set_statuses(&mut self, chat_id: String, mut groups: Vec<whatfast_protocol::StatusGroup>) {
        // Los estados que publica este dispositivo (texto y foto) no viajan al
        // backend: viven solo en memoria. Un snapshot del backend no los debe
        // borrar, o el estado que el usuario acaba de subir desaparece de la
        // tira al llegar el primer refresco.
        let local: Vec<whatfast_protocol::Message> = self
            .statuses
            .iter()
            .flat_map(|group| group.messages.iter())
            .filter(|message| message.id.starts_with("local-status-"))
            .cloned()
            .collect();
        for message in local {
            if groups
                .iter()
                .any(|group| group.messages.iter().any(|item| item.id == message.id))
            {
                continue;
            }
            if let Some(group) = groups
                .iter_mut()
                .find(|group| group.sender_id == message.sender_id)
            {
                group.messages.push(message);
            } else {
                groups.insert(
                    0,
                    whatfast_protocol::StatusGroup {
                        sender_id: message.sender_id.clone(),
                        sender_name: message.sender_name.clone(),
                        sender_phone_number: message.sender_phone_number.clone(),
                        messages: vec![message],
                    },
                );
            }
        }
        // Un estado eliminado no se muestra: se va de su anillo. Si el
        // anillo tenia estados y se queda sin ninguno, se va de la tira, con
        // su progreso y sus botones, en vez de dejar una pantalla que diga
        // que se borro. Un anillo que ya llego vacio se queda como estaba.
        groups.retain_mut(|group| {
            let had_messages = !group.messages.is_empty();
            group.messages.retain(|message| !message.revoked);
            !had_messages || !group.messages.is_empty()
        });
        for group in &mut groups {
            group
                .messages
                .sort_by(|a, b| a.timestamp_ms.cmp(&b.timestamp_ms).then_with(|| a.id.cmp(&b.id)));
        }
        for group in &groups {
            for message in &group.messages {
                self.remember_message_media(message);
            }
        }
        self.statuses_chat_id = chat_id;
        self.statuses = groups;
    }

    /// The statuses of the profile.
    pub fn statuses(&self) -> &[whatfast_protocol::StatusGroup] {
        &self.statuses
    }

    /// The conversation that carries the statuses.
    pub fn statuses_chat_id(&self) -> &str {
        &self.statuses_chat_id
    }

    /// Keeps the newsletters the account is subscribed to.
    pub fn set_newsletters(&mut self, newsletters: Vec<whatfast_protocol::Newsletter>) {
        self.newsletters = newsletters;
    }

    /// The newsletters the account is subscribed to.
    pub fn newsletters(&self) -> &[whatfast_protocol::Newsletter] {
        &self.newsletters
    }

    /// Adds one newsletter, replacing the one with the same jid.
    ///
    /// port: the origin subscribes with
    /// `[...items.filter((item) => item.jid !== jid), newsletter]`.
    pub fn add_newsletter(&mut self, newsletter: whatfast_protocol::Newsletter) {
        self.newsletters.retain(|item| item.jid != newsletter.jid);
        self.newsletters.push(newsletter);
    }

    /// Drops one newsletter. Returns whether it was there.
    pub fn remove_newsletter(&mut self, jid: &str) -> bool {
        let before = self.newsletters.len();
        self.newsletters.retain(|item| item.jid != jid);
        self.newsletters.len() != before
    }

    /// Keeps the posts of one newsletter.
    pub fn set_newsletter_messages(
        &mut self,
        jid: String,
        messages: Vec<whatfast_protocol::Message>,
    ) {
        self.newsletter_messages.insert(jid, messages);
    }

    /// The posts of one newsletter.
    pub fn newsletter_messages(&self, jid: &str) -> &[whatfast_protocol::Message] {
        self.newsletter_messages.get(jid).map_or(&[], Vec::as_slice)
    }

    /// Keeps the path of the cached file of one attachment.
    ///
    /// Returns whether the path changed.
    pub fn set_message_attachment_path(
        &mut self,
        chat_id: &str,
        message_id: &str,
        path: String,
    ) -> bool {
        // The download just wrote the file: the answer of the map changes
        // here, in the event, and the paint of the bubble only reads it.
        self.refresh_media_path(&path);
        let mut updated = false;
        for group in &mut self.statuses {
            for message in &mut group.messages {
                if message.id == message_id
                    && let Some(whatfast_protocol::message::Content::Attachment(attachment)) =
                        message.content.as_mut()
                {
                    attachment.local_path = path.clone();
                    updated = true;
                }
            }
        }
        let Some(message) = self
            .message_index
            .get(message_id)
            .and_then(|index| self.messages.get_mut(*index))
            .filter(|message| message.chat_id == chat_id)
        else {
            return updated;
        };
        let Some(whatfast_protocol::message::Content::Attachment(attachment)) =
            message.content.as_mut()
        else {
            return updated;
        };
        if attachment.local_path == path {
            return updated;
        }
        attachment.local_path = path;
        // A voice note changes shape when its file arrives: it was a card
        // with a download arrow and becomes a player. The cached height
        // belonged to the other shape and left a gap under the bubble.
        self.invalidate_message_height(message_id);
        true
    }

    /// Pins or unpins one conversation, keeping the order of the sidebar.
    pub fn set_chat_pinned(&mut self, chat_id: &str, pinned: bool) -> bool {
        let Some(index) = self.chat_index.get(chat_id).copied() else {
            return false;
        };
        let Some(chat) = self.chats.get_mut(index) else {
            return false;
        };
        if chat.pinned == pinned {
            return true;
        }
        chat.pinned = pinned;
        // port: the origin sorts the list again when a conversation is
        // pinned, because the pinned ones go first.
        self.sort_chats();
        true
    }

    /// Archives or unarchives one conversation.
    pub fn set_chat_archived(&mut self, chat_id: &str, archived: bool) -> bool {
        let Some(index) = self.chat_index.get(chat_id).copied() else {
            return false;
        };
        let Some(chat) = self.chats.get_mut(index) else {
            return false;
        };
        chat.archived = archived;
        // port: the archived conversations leave the inbox of the sidebar.
        self.rebuild_chat_ids();
        true
    }

    /// Mutes or unmutes one conversation.
    pub fn set_chat_muted(&mut self, chat_id: &str, muted: bool, mute_until_ms: i64) -> bool {
        let Some(index) = self.chat_index.get(chat_id).copied() else {
            return false;
        };
        let Some(chat) = self.chats.get_mut(index) else {
            return false;
        };
        chat.muted = muted;
        chat.mute_until_ms = mute_until_ms;
        true
    }

    /// Blocks or unblocks one conversation.
    pub fn set_chat_blocked(&mut self, chat_id: &str, blocked: bool) -> bool {
        let Some(index) = self.chat_index.get(chat_id).copied() else {
            return false;
        };
        let Some(chat) = self.chats.get_mut(index) else {
            return false;
        };
        chat.blocked = blocked;
        true
    }

    /// Drops one message of the open conversation and reindexes the rest.
    ///
    /// A message that was deleted disappears from the thread: the window does
    /// not draw it, the same way WhatsApp hides a message deleted for
    /// everyone. The caller keeps the copy it needs to put it back when the
    /// backend refuses the change.
    pub fn remove_message(
        &mut self,
        chat_id: &str,
        message_id: &str,
    ) -> Option<whatfast_protocol::Message> {
        let index = self.message_index.get(message_id).copied()?;
        if self.messages.get(index)?.chat_id != chat_id {
            return None;
        }
        let previous = self.messages.remove(index);
        self.invalidate_message_height(message_id);
        self.reindex_messages();
        self.rebuild_reply_metadata();
        self.rebuild_message_sizes();
        Some(previous)
    }

    /// Drops one message the way a delete looks in the thread, and returns the
    /// message it took out so the caller can roll it back.
    ///
    /// port: the origin marked the message as revoked and kept the row saying
    /// "this message was deleted"; here the row leaves the thread, because the
    /// user asked not to see a deleted message at all.
    pub fn revoke_message(
        &mut self,
        chat_id: &str,
        message_id: &str,
    ) -> Option<whatfast_protocol::Message> {
        self.remove_message(chat_id, message_id)
    }

    /// Puts a message back where it was, after a failed change.
    pub fn restore_message(&mut self, message: whatfast_protocol::Message) {
        self.upsert_message(message);
    }

    /// Applies one complete vote intent to the poll of one message and
    /// returns the poll it replaced, so the caller can roll it back.
    pub fn apply_poll_vote(
        &mut self,
        chat_id: &str,
        message_id: &str,
        selected_options: &[String],
    ) -> Option<whatfast_protocol::PollContent> {
        let index = self.message_index.get(message_id).copied()?;
        let message = self.messages.get_mut(index)?;
        if message.chat_id != chat_id {
            return None;
        }
        let Some(whatfast_protocol::message::Content::Poll(poll)) = message.content.as_mut() else {
            return None;
        };
        let previous = poll.clone();
        *poll = optimistic_poll_vote(poll, selected_options);
        Some(previous)
    }

    /// Puts the poll of one message back where it was, after a failed vote.
    pub fn restore_poll_content(
        &mut self,
        chat_id: &str,
        message_id: &str,
        content: whatfast_protocol::PollContent,
    ) -> bool {
        let Some(index) = self.message_index.get(message_id).copied() else {
            return false;
        };
        let Some(message) = self.messages.get_mut(index) else {
            return false;
        };
        if message.chat_id != chat_id {
            return false;
        }
        let Some(whatfast_protocol::message::Content::Poll(poll)) = message.content.as_mut() else {
            return false;
        };
        *poll = content;
        true
    }

    /// Forgets the height of one message, because its content changed.
    fn invalidate_message_height(&mut self, id: &str) {
        for heights in self.message_height_cache.values_mut() {
            heights.remove(id);
        }
        self.measured_width_bucket = None;
    }

    /// Drops the cached height of one message.
    ///
    /// The paint depends on state that lives outside the row (a download that
    /// failed, a player that replaced a card); after that state changes the
    /// next frame has to measure the bubble again or the row keeps the height
    /// of the previous shape and overlaps its neighbour.
    pub fn invalidate_height(&mut self, message_id: &str) {
        self.invalidate_message_height(message_id);
    }

    /// Sorts the conversations of the sidebar again.
    fn sort_chats(&mut self) {
        self.chats.sort_by(compare_chats);
        self.reindex_chats();
    }

    fn reindex_chats(&mut self) {
        self.chat_index.clear();
        self.chat_index.reserve(self.chats.len());
        for (index, chat) in self.chats.iter().enumerate() {
            self.chat_index.insert(chat.id.clone(), index);
        }
        self.rebuild_chat_ids();
    }

    fn rebuild_chat_ids(&mut self) {
        let filter = self.chat_filter;
        let favorites = &self.favorite_chat_ids;
        let mut visible = Vec::with_capacity(self.chats.len());
        for chat in &self.chats {
            let keeps = match filter {
                ChatFilter::Favorites => !chat.archived && favorites.contains(&chat.id),
                ChatFilter::Mentions => !chat.archived && self.chat_has_mention(&chat.id),
                other => other.includes(chat),
            };
            if keeps {
                visible.push(chat.id.clone());
            }
        }
        self.visible_chat_ids = Rc::new(visible);
    }

    fn reindex_messages(&mut self) {
        self.message_index.clear();
        self.message_index.reserve(self.messages.len());
        for (index, message) in self.messages.iter().enumerate() {
            self.message_index.insert(message.id.clone(), index);
        }
    }

    fn reindex_message_range(&mut self, start: usize, end: usize) {
        for index in start..end.min(self.messages.len()) {
            self.message_index
                .insert(self.messages[index].id.clone(), index);
        }
    }

    fn rebuild_reply_metadata(&mut self) {
        self.reply_metadata.clear();
        for message in &self.messages {
            if message.reply_to_message_id.is_empty() {
                continue;
            }
            self.reply_metadata
                .entry(message.reply_to_message_id.clone())
                .and_modify(|metadata| metadata.count += 1)
                .or_insert_with(|| ReplyMetadata {
                    count: 1,
                    first_reply_id: message.id.clone(),
                });
        }
    }

    fn rebuild_message_sizes(&mut self) {
        self.message_sizes = Rc::new(
            self.messages
                .iter()
                .map(|message| size(px(1.), estimate_message_height(message)))
                .collect(),
        );
        self.measured_width_bucket = None;
    }
}

pub fn message_text(message: &whatfast_protocol::Message) -> &str {
    if message.revoked {
        return "This message was deleted";
    }
    match &message.content {
        Some(whatfast_protocol::message::Content::Text(text)) => text.text.as_str(),
        Some(whatfast_protocol::message::Content::Image(image)) => image.caption.as_str(),
        Some(whatfast_protocol::message::Content::Attachment(attachment)) => {
            if !attachment.caption.is_empty() {
                attachment.caption.as_str()
            } else if !attachment.file_name.is_empty() {
                attachment.file_name.as_str()
            } else {
                attachment.kind.as_str()
            }
        }
        Some(whatfast_protocol::message::Content::Contacts(contacts)) => contacts
            .contacts
            .first()
            .map(|contact| contact.display_name.as_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("Contact"),
        Some(whatfast_protocol::message::Content::Location(location)) => {
            if !location.name.is_empty() {
                location.name.as_str()
            } else if !location.address.is_empty() {
                location.address.as_str()
            } else {
                "Location"
            }
        }
        Some(whatfast_protocol::message::Content::Unsupported(unsupported)) => {
            unsupported.fallback_text.as_str()
        }
        Some(whatfast_protocol::message::Content::Poll(poll)) => poll.question.as_str(),
        None => "Unsupported message",
    }
}

/// Applies one complete local vote intent to a poll.
///
/// port: the origin reduces the vote in `tauri/src/lib/polls.ts`, over the
/// DTOs of `tauri/src/lib/types.ts`. The store works over the DTOs of the
/// bridge, so the reducer is repeated here with the same rules: the selected
/// options are the whole intent, an empty list retracts the vote and the
/// total of voters follows.
fn optimistic_poll_vote(
    poll: &whatfast_protocol::PollContent,
    selected_options: &[String],
) -> whatfast_protocol::PollContent {
    let previous: HashSet<&str> = poll
        .options
        .iter()
        .filter(|option| option.selected_by_me)
        .map(|option| option.name.as_str())
        .collect();
    let next: HashSet<&str> = selected_options.iter().map(String::as_str).collect();

    let total_voters = if previous.is_empty() && !next.is_empty() {
        poll.total_voters.saturating_add(1)
    } else if !previous.is_empty() && next.is_empty() {
        poll.total_voters.saturating_sub(1)
    } else {
        poll.total_voters
    };

    whatfast_protocol::PollContent {
        question: poll.question.clone(),
        options: poll
            .options
            .iter()
            .map(|option| {
                let was_selected = previous.contains(option.name.as_str());
                let is_selected = next.contains(option.name.as_str());
                let vote_count = if was_selected && !is_selected {
                    option.vote_count.saturating_sub(1)
                } else if !was_selected && is_selected {
                    option.vote_count.saturating_add(1)
                } else {
                    option.vote_count
                };
                whatfast_protocol::PollOption {
                    name: option.name.clone(),
                    vote_count,
                    selected_by_me: is_selected,
                    voters: option.voters.clone(),
                }
            })
            .collect(),
        selectable_options_count: poll.selectable_options_count,
        total_voters,
    }
}

fn preserve_sender_details(
    incoming: &mut whatfast_protocol::Message,
    existing: &whatfast_protocol::Message,
) {
    if incoming.sender_name.is_empty() {
        incoming.sender_name.clone_from(&existing.sender_name);
    }
    if incoming.sender_phone_number.is_empty() {
        incoming
            .sender_phone_number
            .clone_from(&existing.sender_phone_number);
    }
    if incoming.sender_avatar_path.is_empty() {
        incoming
            .sender_avatar_path
            .clone_from(&existing.sender_avatar_path);
    }
}

fn preserve_local_image_path(
    incoming: &mut whatfast_protocol::Message,
    existing: &whatfast_protocol::Message,
) {
    let Some(whatfast_protocol::message::Content::Image(incoming_image)) =
        incoming.content.as_mut()
    else {
        return;
    };
    if let Some(whatfast_protocol::message::Content::Image(existing_image)) =
        existing.content.as_ref()
    {
        if incoming_image.local_path.is_empty() {
            incoming_image
                .local_path
                .clone_from(&existing_image.local_path);
        }
        if incoming_image.thumbnail_path.is_empty() {
            incoming_image
                .thumbnail_path
                .clone_from(&existing_image.thumbnail_path);
        }
        if incoming_image.jpeg_thumbnail.is_empty() {
            incoming_image
                .jpeg_thumbnail
                .clone_from(&existing_image.jpeg_thumbnail);
        }
    }
}

pub fn image_row_path(image: &whatfast_protocol::ImageContent) -> &str {
    image.thumbnail_path.as_str()
}

pub fn image_viewer_path(image: &whatfast_protocol::ImageContent) -> &str {
    image.local_path.as_str()
}

#[derive(Debug, Eq, PartialEq)]
pub struct ReactionCount<'a> {
    pub emoji: &'a str,
    pub count: usize,
    pub from_me: bool,
}

pub fn reaction_counts(message: &whatfast_protocol::Message) -> Vec<ReactionCount<'_>> {
    let mut counts: Vec<ReactionCount<'_>> = Vec::new();
    for reaction in message
        .reactions
        .iter()
        .filter(|reaction| !reaction.emoji.is_empty())
    {
        if let Some(existing) = counts
            .iter_mut()
            .find(|existing| existing.emoji == reaction.emoji.as_str())
        {
            existing.count += 1;
            existing.from_me |= reaction.from_me;
        } else {
            counts.push(ReactionCount {
                emoji: reaction.emoji.as_str(),
                count: 1,
                from_me: reaction.from_me,
            });
        }
    }
    counts
}

pub fn reactions_for_emoji(
    message: &whatfast_protocol::Message,
    emoji: &str,
) -> Vec<whatfast_protocol::Reaction> {
    let mut reactions: Vec<whatfast_protocol::Reaction> = Vec::new();
    for reaction in message
        .reactions
        .iter()
        .filter(|reaction| reaction.emoji == emoji)
    {
        let existing = reactions.iter().position(|candidate| {
            (reaction.from_me && candidate.from_me)
                || (!reaction.sender_id.is_empty() && reaction.sender_id == candidate.sender_id)
        });
        if let Some(index) = existing {
            if reaction.timestamp_ms >= reactions[index].timestamp_ms {
                reactions[index] = reaction.clone();
            }
        } else {
            reactions.push(reaction.clone());
        }
    }
    reactions.sort_by(|left, right| {
        right.from_me.cmp(&left.from_me).then_with(|| {
            reaction_sender_name(left)
                .to_lowercase()
                .cmp(&reaction_sender_name(right).to_lowercase())
        })
    });
    reactions
}

pub fn reaction_sender_name(reaction: &whatfast_protocol::Reaction) -> &str {
    if reaction.from_me {
        "You"
    } else if !reaction.sender_name.trim().is_empty() {
        reaction.sender_name.trim()
    } else if !reaction.sender_phone_number.trim().is_empty() {
        reaction.sender_phone_number.trim()
    } else if !reaction.sender_id.trim().is_empty() {
        reaction.sender_id.trim()
    } else {
        "Unknown contact"
    }
}

pub fn reaction_sender_detail(reaction: &whatfast_protocol::Reaction) -> &str {
    let phone = reaction.sender_phone_number.trim();
    if !phone.is_empty() && phone != reaction_sender_name(reaction) {
        phone
    } else if reaction.from_me {
        "Your reaction"
    } else {
        let sender_id = reaction.sender_id.trim();
        if !sender_id.is_empty() && sender_id != reaction_sender_name(reaction) {
            sender_id
        } else {
            "WhatsApp contact"
        }
    }
}

fn compare_chats(
    left: &whatfast_protocol::Chat,
    right: &whatfast_protocol::Chat,
) -> std::cmp::Ordering {
    right
        .pinned
        .cmp(&left.pinned)
        .then_with(|| {
            right
                .last_message_timestamp_ms
                .cmp(&left.last_message_timestamp_ms)
        })
        .then_with(|| left.id.cmp(&right.id))
}

fn compare_messages(
    left: &whatfast_protocol::Message,
    right: &whatfast_protocol::Message,
) -> std::cmp::Ordering {
    (left.timestamp_ms, left.id.as_str()).cmp(&(right.timestamp_ms, right.id.as_str()))
}

/// Whether a receipt may replace the status a message already shows.
///
/// The wire values are not an order, as the contract of the enum says: a
/// failed message may recover, and a receipt that arrives late never takes a
/// message back from read to delivered.
fn receipt_advances(
    current: whatfast_protocol::MessageStatus,
    next: whatfast_protocol::MessageStatus,
) -> bool {
    use whatfast_protocol::MessageStatus::{Delivered, Failed, Pending, Read, Sent, Unspecified};
    matches!(
        (current, next),
        (Unspecified | Pending, Sent | Delivered | Read | Failed)
            | (Failed, Sent | Delivered | Read)
            | (Sent, Delivered | Read)
            | (Delivered, Read)
    )
}

/// Whether the thread skips the message: the deleted ones and the rows that
/// carry no visible content at all.
///
/// WhatsApp sends envelopes of protocol (a revoke whose target is gone, an
/// edit, a poll update) and some versions of the sidecar stored them as
/// empty rows. A row without text and without content is not a message: it
/// is the envelope of one, and the thread never paints it.
fn message_is_hidden(message: &whatfast_protocol::Message) -> bool {
    if message.revoked {
        return true;
    }
    let Some(whatfast_protocol::message::Content::Unsupported(content)) = message.content.as_ref()
    else {
        return false;
    };
    content.type_name == "revoked"
        || content.type_name == "signal"
        || (content.fallback_text.trim().is_empty() && message.reactions.is_empty())
}

fn estimate_message_height(message: &whatfast_protocol::Message) -> Pixels {
    let text = message_text(message);
    let lines = text
        .lines()
        .map(|line| (line.chars().count().max(1) as f32 / 46.0).ceil())
        .sum::<f32>()
        .max(1.0);
    let image_height = match message.content.as_ref() {
        Some(whatfast_protocol::message::Content::Image(image)) => estimated_image_height(image),
        _ => 0.0,
    };
    let reply_height = if message.reply_to_message_id.is_empty() {
        0.0
    } else {
        54.0
    };
    px(26.0 + (lines - 1.0) * 18.0 + image_height + reply_height)
}

fn estimated_image_height(image: &whatfast_protocol::ImageContent) -> f32 {
    if image.width == 0 || image.height == 0 {
        return 220.0;
    }
    let max_edge = if image.sticker { 220.0 } else { 320.0 };
    let source_width = image.width as f32;
    let source_height = image.height as f32;
    let scale = (max_edge / source_width).min(max_edge / source_height);
    source_height * scale
}

