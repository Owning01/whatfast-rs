//! The chat store of the client: the state the user interface renders.
//!
//! port: taken from the origin's `desktop/src/model.rs`. The only change is
//! the toolkit: GPUI's `Pixels`, `Size`, `px` and `size` became
//! `crate::geom`'s, and the protobuf DTOs come from `whatfast-protocol`
//! instead of the origin's private `proto` module.

use std::{
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
    pub const ALL: [Self; 4] = [Self::All, Self::Unread, Self::Favorites, Self::Groups];

    /// The label of the filter, in the language of the client.
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Unread => "Unread",
            Self::Favorites => "Favorites",
            Self::Groups => "Groups",
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

#[derive(Default)]
pub struct Store {
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
    #[cfg(test)]
    full_message_reindexes: usize,
    #[cfg(test)]
    chat_id_rebuilds: usize,
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
            other => other.includes(chat),
        }
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
        self.sync_active = !complete;
        self.sync_complete = complete;
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
        for heights in self.message_height_cache.values_mut() {
            heights.remove(&message.id);
        }
        let old_index = self.message_index.remove(&message.id);
        if let Some(index) = old_index {
            preserve_local_image_path(&mut message, &self.messages[index]);
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
        let mut by_id: HashMap<String, whatfast_protocol::Message> = self
            .messages
            .drain(..)
            .map(|message| (message.id.clone(), message))
            .collect();
        for mut message in incoming {
            for heights in self.message_height_cache.values_mut() {
                heights.remove(&message.id);
            }
            if let Some(existing) = by_id.get(&message.id) {
                preserve_local_image_path(&mut message, existing);
            }
            by_id.insert(message.id.clone(), message);
        }
        self.messages = by_id.into_values().collect();
        self.messages.sort_by(compare_messages);
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
        let mut by_id: HashMap<String, whatfast_protocol::Message> = self
            .messages
            .drain(..)
            .map(|message| (message.id.clone(), message))
            .collect();
        for mut message in incoming {
            for heights in self.message_height_cache.values_mut() {
                heights.remove(&message.id);
            }
            if let Some(existing) = by_id.get(&message.id) {
                preserve_local_image_path(&mut message, existing);
            }
            by_id.insert(message.id.clone(), message);
        }
        self.messages = by_id.into_values().collect();
        self.messages.sort_by(compare_messages);
        if self.messages.len() > MAX_ACTIVE_MESSAGES {
            let excess = self.messages.len() - MAX_ACTIVE_MESSAGES;
            self.messages.drain(..excess);
            self.has_older_messages = true;
        }
        self.has_newer_messages = has_more || self.newer_activity;
        self.reindex_messages();
        self.rebuild_reply_metadata();
        self.rebuild_message_sizes();
    }

    pub fn replace_message_window(
        &mut self,
        mut messages: Vec<whatfast_protocol::Message>,
        has_older: bool,
        has_newer: bool,
    ) {
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
            message.status = receipt.status;
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
        if let Some(chat) = self
            .chat_index
            .get(chat_id)
            .and_then(|index| self.chats.get_mut(*index))
        {
            chat.avatar_path = avatar_path;
        }
    }

    pub fn set_participant_avatar(&mut self, participant_id: String, avatar_path: String) {
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
        let Some(message) = self
            .message_index
            .get(message_id)
            .and_then(|index| self.messages.get_mut(*index))
            .filter(|message| message.chat_id == chat_id)
        else {
            return false;
        };
        let Some(whatfast_protocol::message::Content::Image(image)) = message.content.as_mut()
        else {
            return false;
        };
        if image.local_path == image_path && image.thumbnail_path == thumbnail_path {
            return false;
        }
        image.local_path = image_path;
        image.thumbnail_path = thumbnail_path;
        true
    }

    pub fn participant_avatar_path(&self, participant_id: &str) -> Option<&str> {
        self.participant_avatar_paths
            .get(participant_id)
            .map(String::as_str)
            .filter(|path| !path.is_empty())
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

    /// Keeps the library of stickers.
    pub fn set_sticker_packs(&mut self, packs: Vec<whatfast_protocol::StickerPack>) {
        self.sticker_packs = packs;
    }

    /// The library of stickers.
    pub fn sticker_packs(&self) -> &[whatfast_protocol::StickerPack] {
        &self.sticker_packs
    }

    /// One sticker of the library, by its id.
    pub fn find_sticker(&self, sticker_id: &str) -> Option<&whatfast_protocol::Sticker> {
        self.sticker_packs
            .iter()
            .flat_map(|pack| pack.stickers.iter())
            .find(|sticker| sticker.id == sticker_id)
    }

    /// Keeps the statuses of the profile and the conversation that carries
    /// them.
    pub fn set_statuses(&mut self, chat_id: String, groups: Vec<whatfast_protocol::StatusGroup>) {
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
        let Some(message) = self
            .message_index
            .get(message_id)
            .and_then(|index| self.messages.get_mut(*index))
            .filter(|message| message.chat_id == chat_id)
        else {
            return false;
        };
        let Some(whatfast_protocol::message::Content::Attachment(attachment)) =
            message.content.as_mut()
        else {
            return false;
        };
        if attachment.local_path == path {
            return false;
        }
        attachment.local_path = path;
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

    /// Marks one message as revoked, the way a delete looks locally, and
    /// returns the message it replaced so the caller can roll it back.
    ///
    /// port: the origin sets `{ ...current, revoked: true, content: null }`.
    pub fn revoke_message(
        &mut self,
        chat_id: &str,
        message_id: &str,
    ) -> Option<whatfast_protocol::Message> {
        let index = self.message_index.get(message_id).copied()?;
        let message = self.messages.get_mut(index)?;
        if message.chat_id != chat_id {
            return None;
        }
        let previous = message.clone();
        message.revoked = true;
        message.content = None;
        self.invalidate_message_height(message_id);
        Some(previous)
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
                other => other.includes(chat),
            };
            if keeps {
                visible.push(chat.id.clone());
            }
        }
        self.visible_chat_ids = Rc::new(visible);
        #[cfg(test)]
        {
            self.chat_id_rebuilds += 1;
        }
    }

    fn reindex_messages(&mut self) {
        self.message_index.clear();
        self.message_index.reserve(self.messages.len());
        for (index, message) in self.messages.iter().enumerate() {
            self.message_index.insert(message.id.clone(), index);
        }
        #[cfg(test)]
        {
            self.full_message_reindexes += 1;
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
    px(48.0 + (lines - 1.0) * 18.0 + image_height + reply_height)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::hint::black_box;

    fn message(id: usize) -> whatfast_protocol::Message {
        whatfast_protocol::Message {
            id: id.to_string(),
            chat_id: "chat".into(),
            timestamp_ms: id as i64,
            ..Default::default()
        }
    }

    #[test]
    fn message_window_is_bounded_and_sorted() {
        let mut store = Store {
            selected_chat_id: Some("chat".into()),
            ..Default::default()
        };
        for id in (0..MAX_ACTIVE_MESSAGES + 50).rev() {
            store.upsert_message(message(id));
        }
        assert_eq!(store.messages.len(), MAX_ACTIVE_MESSAGES);
        assert_eq!(store.messages.first().unwrap().id, "50");
        assert!(store.has_older_messages);
    }

    #[test]
    fn upserts_are_idempotent() {
        let mut store = Store {
            selected_chat_id: Some("chat".into()),
            ..Default::default()
        };
        store.upsert_message(message(1));
        store.upsert_message(message(1));
        assert_eq!(store.messages.len(), 1);
    }

    #[test]
    fn reply_count_tracks_messages_pointing_at_the_same_target() {
        let mut store = Store {
            selected_chat_id: Some("chat".into()),
            ..Default::default()
        };
        store.upsert_message(message(1));
        let mut first_reply = message(2);
        first_reply.reply_to_message_id = "1".into();
        store.upsert_message(first_reply);
        let mut second_reply = message(3);
        second_reply.reply_to_message_id = "1".into();
        store.upsert_message(second_reply);

        assert_eq!(store.reply_count("1"), 2);
        assert_eq!(store.reply_count("2"), 0);
        assert_eq!(store.first_reply_id("1"), Some("2"));
        assert_eq!(store.first_reply_id("2"), None);

        store.upsert_message(message(3));
        assert_eq!(store.reply_count("1"), 1);
        assert_eq!(store.first_reply_id("1"), Some("2"));
    }

    #[test]
    fn the_filters_keep_the_archived_conversations_apart() {
        let mut store = Store::default();
        store.upsert_chat(whatfast_protocol::Chat {
            id: "inbox".into(),
            title: "Inbox".into(),
            ..Default::default()
        });
        store.upsert_chat(whatfast_protocol::Chat {
            id: "archived".into(),
            title: "Archived".into(),
            archived: true,
            ..Default::default()
        });

        assert_eq!(store.chat_ids(ChatFilter::All).as_slice(), ["inbox"]);
        assert_eq!(
            store.chat_ids(ChatFilter::Archived).as_slice(),
            ["archived"]
        );
    }

    #[test]
    fn the_photo_of_the_avatar_survives_the_arrival_of_the_chat() {
        let mut store = Store::default();
        let mut chat = whatfast_protocol::Chat {
            id: "chat".into(),
            title: "Chat".into(),
            ..Default::default()
        };
        chat.avatar_path = "C:/media/avatar.png".into();
        store.upsert_chat(chat.clone());
        assert_eq!(
            store.chat("chat").map(|chat| chat.avatar_path.clone()),
            Some("C:/media/avatar.png".to_string())
        );

        // El backend vuelve a mandar el chat sin el camino del avatar: la
        // foto local no se borra.
        let mut again = chat.clone();
        again.avatar_path = String::new();
        store.upsert_chat(again);
        assert_eq!(
            store.chat("chat").map(|chat| chat.avatar_path.clone()),
            Some("C:/media/avatar.png".to_string())
        );

        // Y tampoco se borra cuando llega una pagina nueva de la lista.
        let mut page = chat.clone();
        page.avatar_path = String::new();
        store.replace_chat_page("", vec![page], 1, String::new());
        assert_eq!(
            store.chat("chat").map(|chat| chat.avatar_path.clone()),
            Some("C:/media/avatar.png".to_string())
        );
    }

    #[test]
    fn the_states_and_the_channels_never_show_up_as_chats() {
        let mut store = Store::default();
        store.upsert_chat(whatfast_protocol::Chat {
            id: "chat".into(),
            title: "Chat".into(),
            ..Default::default()
        });
        store.upsert_chat(whatfast_protocol::Chat {
            id: "status@broadcast".into(),
            title: "status@broadcast".into(),
            transport_jid: "status@broadcast".into(),
            ..Default::default()
        });
        store.upsert_chat(whatfast_protocol::Chat {
            id: "channel".into(),
            title: "Channel".into(),
            transport_jid: "12345@newsletter".into(),
            ..Default::default()
        });

        assert_eq!(store.chat_ids(ChatFilter::All).as_slice(), ["chat"]);
        assert_eq!(
            store.chat_ids(ChatFilter::Archived).as_slice(),
            [] as [String; 0]
        );
        assert!(is_hidden_conversation(&whatfast_protocol::Chat {
            transport_jid: "status@broadcast".into(),
            ..Default::default()
        }));
        assert!(is_hidden_conversation(&whatfast_protocol::Chat {
            transport_jid: "12345@newsletter".into(),
            ..Default::default()
        }));
        assert!(!is_hidden_conversation(&whatfast_protocol::Chat {
            transport_jid: "12345@s.whatsapp.net".into(),
            ..Default::default()
        }));
    }

    #[test]
    fn the_favorite_filter_keeps_the_conversations_the_user_marked() {
        let mut store = Store::default();
        store.upsert_chat(whatfast_protocol::Chat {
            id: "one".into(),
            title: "One".into(),
            ..Default::default()
        });
        store.upsert_chat(whatfast_protocol::Chat {
            id: "two".into(),
            title: "Two".into(),
            ..Default::default()
        });

        assert_eq!(
            store.chat_ids(ChatFilter::Favorites).as_slice(),
            [] as [String; 0]
        );
        assert!(store.toggle_favorite("one"));
        assert!(store.is_favorite("one"));
        assert_eq!(store.chat_ids(ChatFilter::Favorites).as_slice(), ["one"]);
        assert!(!store.toggle_favorite("one"));
        assert!(!store.is_favorite("one"));
        assert_eq!(
            store.chat_ids(ChatFilter::Favorites).as_slice(),
            [] as [String; 0]
        );
    }

    #[test]
    fn the_unread_and_the_group_filters_pick_their_own_conversations() {
        let mut store = Store::default();
        store.upsert_chat(whatfast_protocol::Chat {
            id: "unread".into(),
            title: "Unread".into(),
            unread_count: 3,
            ..Default::default()
        });
        store.upsert_chat(whatfast_protocol::Chat {
            id: "group".into(),
            title: "Group".into(),
            kind: whatfast_protocol::ChatKind::Group as i32,
            ..Default::default()
        });

        assert_eq!(store.chat_ids(ChatFilter::Unread).as_slice(), ["unread"]);
        assert_eq!(store.chat_ids(ChatFilter::Groups).as_slice(), ["group"]);
    }

    #[test]
    fn cached_sidebar_ids_allocate_only_when_chats_mutate() {
        let mut store = Store::default();
        let chats = (0..1_000)
            .map(|id| whatfast_protocol::Chat {
                id: format!("chat-{id}"),
                archived: id % 10 == 0,
                last_message_timestamp_ms: id,
                ..Default::default()
            })
            .collect();
        store.replace_chat_page("", chats, 1_000, String::new());
        let original = store.chat_ids(ChatFilter::All);
        let rebuilds = store.chat_id_rebuilds;

        for _ in 0..100 {
            black_box(store.chat_ids(ChatFilter::All));
        }

        // port: the origin counted the allocations of the global allocator,
        // which needs `unsafe`. This crate forbids `unsafe`, so the cache is
        // observed through the rebuild counter: reading the ids must not
        // rebuild them.
        assert_eq!(store.chat_id_rebuilds, rebuilds);
        assert!(Rc::ptr_eq(&original, &store.chat_ids(ChatFilter::All)));

        let uncached = store
            .chats
            .iter()
            .filter(|chat| ChatFilter::All.includes(chat))
            .map(|chat| chat.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(uncached, original.as_slice());

        store.upsert_chat(whatfast_protocol::Chat {
            id: "new-chat".into(),
            ..Default::default()
        });
        assert!(!Rc::ptr_eq(&original, &store.chat_ids(ChatFilter::All)));
        assert!(store.chat_id_rebuilds > rebuilds);
    }

    #[test]
    fn live_upserts_repair_only_the_shifted_message_index_range() {
        let mut store = Store {
            selected_chat_id: Some("chat".into()),
            ..Default::default()
        };
        store.replace_message_window((0..1_000).map(message).collect(), false, false);
        let full_reindexes = store.full_message_reindexes;
        let mut moved = message(500);
        moved.timestamp_ms = 1_500;
        store.upsert_message(moved);

        assert_eq!(store.full_message_reindexes, full_reindexes);
        for (index, message) in store.messages.iter().enumerate() {
            assert_eq!(store.message_index[&message.id], index);
        }
        assert_eq!(store.messages.last().unwrap().id, "500");
    }

    #[test]
    fn sync_progress_is_cumulative_and_records_completion() {
        let mut store = Store::default();
        store.apply_sync_progress(8, 120, false);
        store.apply_sync_progress(3, 40, true);

        assert_eq!(store.sync_chats, 11);
        assert_eq!(store.sync_messages, 160);
        assert!(!store.sync_active);
        assert!(store.sync_complete);
    }

    #[test]
    fn new_message_kinds_have_readable_timeline_text() {
        let with_content = |content| whatfast_protocol::Message {
            content: Some(content),
            ..Default::default()
        };
        let attachment = with_content(whatfast_protocol::message::Content::Attachment(
            whatfast_protocol::AttachmentContent {
                file_name: "agenda.pdf".into(),
                kind: "document".into(),
                ..Default::default()
            },
        ));
        let contacts = with_content(whatfast_protocol::message::Content::Contacts(
            whatfast_protocol::ContactsContent {
                contacts: vec![whatfast_protocol::ContactContent {
                    display_name: "Fast Friend".into(),
                    ..Default::default()
                }],
            },
        ));
        let location = with_content(whatfast_protocol::message::Content::Location(
            whatfast_protocol::LocationContent {
                address: "Cat Street".into(),
                ..Default::default()
            },
        ));

        assert_eq!(message_text(&attachment), "agenda.pdf");
        assert_eq!(message_text(&contacts), "Fast Friend");
        assert_eq!(message_text(&location), "Cat Street");
    }

    #[test]
    fn reactions_replace_by_sender_remove_and_invalidate_height() {
        let mut store = Store {
            selected_chat_id: Some("chat".into()),
            ..Default::default()
        };
        store.upsert_message(message(1));
        store.set_measured_message_sizes(42, vec![size(px(1.), px(48.))]);
        let reaction = |emoji: &str| whatfast_protocol::Reaction {
            chat_id: "chat".into(),
            message_id: "1".into(),
            sender_id: "participant".into(),
            emoji: emoji.into(),
            ..Default::default()
        };

        store.apply_reaction(reaction("👍"), false);
        assert_eq!(store.messages[0].reactions[0].emoji, "👍");
        assert!(store.message_sizes_need_measurement(42));

        store.apply_reaction(reaction("❤️"), false);
        assert_eq!(store.messages[0].reactions.len(), 1);
        assert_eq!(store.messages[0].reactions[0].emoji, "❤️");

        let mut stale_removal = reaction("");
        stale_removal.timestamp_ms = -1;
        store.apply_reaction(stale_removal, true);
        assert_eq!(store.messages[0].reactions[0].emoji, "❤️");

        store.apply_reaction(reaction(""), true);
        assert!(store.messages[0].reactions.is_empty());
    }

    #[test]
    fn historical_reactions_survive_message_upsert_and_group_into_chips() {
        let mut store = Store {
            selected_chat_id: Some("chat".into()),
            ..Default::default()
        };
        let mut historical = message(1);
        historical.reactions = vec![
            whatfast_protocol::Reaction {
                emoji: "👍".into(),
                sender_id: "one".into(),
                ..Default::default()
            },
            whatfast_protocol::Reaction {
                emoji: "❤️".into(),
                sender_id: "two".into(),
                ..Default::default()
            },
            whatfast_protocol::Reaction {
                emoji: "👍".into(),
                sender_id: "three".into(),
                from_me: true,
                ..Default::default()
            },
            whatfast_protocol::Reaction::default(),
        ];
        store.upsert_message(historical);

        assert_eq!(
            reaction_counts(&store.messages[0]),
            vec![
                ReactionCount {
                    emoji: "👍",
                    count: 2,
                    from_me: true,
                },
                ReactionCount {
                    emoji: "❤️",
                    count: 1,
                    from_me: false,
                },
            ]
        );
    }

    #[test]
    fn reactor_details_filter_one_emoji_put_you_first_and_resolve_identity() {
        let mut historical = message(1);
        historical.reactions = vec![
            whatfast_protocol::Reaction {
                emoji: "👍".into(),
                sender_id: "lid-one".into(),
                sender_name: "Saved Contact".into(),
                sender_phone_number: "+10000000001".into(),
                ..Default::default()
            },
            whatfast_protocol::Reaction {
                emoji: "❤️".into(),
                sender_name: "Other Emoji".into(),
                ..Default::default()
            },
            whatfast_protocol::Reaction {
                emoji: "👍".into(),
                sender_id: "me".into(),
                from_me: true,
                ..Default::default()
            },
            whatfast_protocol::Reaction {
                emoji: "👍".into(),
                sender_id: "lid-phone".into(),
                sender_phone_number: "+10000000002".into(),
                ..Default::default()
            },
        ];

        let reactors = reactions_for_emoji(&historical, "👍");
        assert_eq!(reactors.len(), 3);
        assert_eq!(reaction_sender_name(&reactors[0]), "You");
        assert_eq!(reaction_sender_detail(&reactors[0]), "Your reaction");
        assert_eq!(reaction_sender_name(&reactors[1]), "+10000000002");
        assert_eq!(reaction_sender_name(&reactors[2]), "Saved Contact");
        assert_eq!(reaction_sender_detail(&reactors[2]), "+10000000001");
    }

    #[test]
    fn reaction_events_never_create_message_bubbles() {
        let mut store = Store {
            selected_chat_id: Some("chat".into()),
            ..Default::default()
        };
        store.apply_reaction(
            whatfast_protocol::Reaction {
                chat_id: "chat".into(),
                message_id: "missing-message".into(),
                sender_id: "participant".into(),
                emoji: "👍".into(),
                ..Default::default()
            },
            false,
        );
        assert!(store.messages.is_empty());
    }

    #[test]
    fn participant_avatars_are_cached_by_sender() {
        let mut store = Store::default();
        store.set_participant_avatar("participant".into(), "/tmp/avatar.jpg".into());
        assert_eq!(
            store.participant_avatar_path("participant"),
            Some("/tmp/avatar.jpg")
        );
        assert_eq!(store.participant_avatar_path("unknown"), None);
    }

    #[test]
    fn image_download_path_survives_message_replay() {
        let mut store = Store {
            selected_chat_id: Some("chat".into()),
            ..Default::default()
        };
        let mut image_message = message(1);
        image_message.content = Some(whatfast_protocol::message::Content::Image(
            whatfast_protocol::ImageContent {
                mime_type: "image/jpeg".into(),
                downloadable: true,
                ..Default::default()
            },
        ));
        store.upsert_message(image_message.clone());
        assert!(store.set_message_image_paths(
            "chat",
            "1",
            "/cache/photo.jpg".into(),
            "/cache/photo.thumb.png".into()
        ));
        store.upsert_message(image_message);
        let Some(whatfast_protocol::message::Content::Image(image)) =
            store.messages[0].content.as_ref()
        else {
            panic!("image content missing");
        };
        assert_eq!(image.local_path, "/cache/photo.jpg");
        assert_eq!(image.thumbnail_path, "/cache/photo.thumb.png");
        assert_eq!(image_row_path(image), "/cache/photo.thumb.png");
        assert_eq!(image_viewer_path(image), "/cache/photo.jpg");
        assert_eq!(message_text(&store.messages[0]), "");
    }

    #[test]
    fn chat_pages_bulk_merge_to_ten_thousand_without_duplicates() {
        let mut store = Store::default();
        for page in 0..100 {
            let cursor = if page == 0 {
                String::new()
            } else {
                (page * 100).to_string()
            };
            let chats = (page * 100..(page + 1) * 100)
                .map(|id| whatfast_protocol::Chat {
                    id: format!("chat-{id}"),
                    last_message_timestamp_ms: 10_000 - id as i64,
                    ..Default::default()
                })
                .collect();
            store.replace_chat_page(&cursor, chats, 10_000, ((page + 1) * 100).to_string());
        }
        assert_eq!(store.chats.len(), 10_000);
        assert_eq!(store.chat_index.len(), 10_000);
        assert_eq!(store.chats.first().unwrap().id, "chat-0");
    }

    #[test]
    fn first_page_refresh_preserves_loaded_pages_selection_order_and_cursor() {
        let mut store = Store::default();
        store.replace_chat_page(
            "",
            vec![
                whatfast_protocol::Chat {
                    id: "first".into(),
                    title: "old title".into(),
                    last_message_timestamp_ms: 400,
                    ..Default::default()
                },
                whatfast_protocol::Chat {
                    id: "second".into(),
                    last_message_timestamp_ms: 300,
                    ..Default::default()
                },
            ],
            4,
            "page-2".into(),
        );
        store.replace_chat_page(
            "page-2",
            vec![
                whatfast_protocol::Chat {
                    id: "third".into(),
                    last_message_timestamp_ms: 200,
                    ..Default::default()
                },
                whatfast_protocol::Chat {
                    id: "fourth".into(),
                    last_message_timestamp_ms: 100,
                    ..Default::default()
                },
            ],
            4,
            "page-3".into(),
        );
        store.selected_chat_id = Some("third".into());

        store.replace_chat_page(
            "",
            vec![
                whatfast_protocol::Chat {
                    id: "first".into(),
                    title: "refreshed title".into(),
                    last_message_timestamp_ms: 500,
                    ..Default::default()
                },
                whatfast_protocol::Chat {
                    id: "second".into(),
                    last_message_timestamp_ms: 300,
                    ..Default::default()
                },
            ],
            4,
            "page-2-regressed".into(),
        );

        assert_eq!(store.chats.len(), 4);
        assert_eq!(
            store
                .chats
                .iter()
                .map(|chat| chat.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second", "third", "fourth"]
        );
        assert_eq!(store.chat("first").unwrap().title, "refreshed title");
        assert_eq!(store.selected_chat_id.as_deref(), Some("third"));
        assert_eq!(store.next_chat_cursor, "page-3");
    }

    #[test]
    fn prepend_at_capacity_keeps_older_window_and_exposes_newer_navigation() {
        let mut store = Store {
            selected_chat_id: Some("chat".into()),
            ..Default::default()
        };
        for id in 100..100 + MAX_ACTIVE_MESSAGES {
            store.upsert_message(message(id));
        }
        store.prepend_messages((0..100).map(message).collect());
        assert_eq!(store.messages.len(), MAX_ACTIVE_MESSAGES);
        assert_eq!(store.messages.first().unwrap().id, "0");
        assert_eq!(store.messages.last().unwrap().id, "1999");
        assert!(store.has_newer_messages);
    }

    #[test]
    fn forward_pages_append_in_order_without_duplicates() {
        let mut store = Store {
            selected_chat_id: Some("chat".into()),
            ..Default::default()
        };
        store.replace_message_window(vec![message(1), message(2)], true, true);
        store.append_newer_messages(vec![message(2), message(3)], false);
        assert_eq!(
            store
                .messages
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            ["1", "2", "3"]
        );
        assert!(!store.has_newer_messages);
        assert!(store.has_older_messages);
    }

    #[test]
    fn forward_page_keeps_activity_that_arrived_during_the_request() {
        let mut store = Store {
            selected_chat_id: Some("chat".into()),
            ..Default::default()
        };
        store.replace_message_window(vec![message(1)], false, true);
        store.newer_activity = true;
        store.append_newer_messages(vec![message(2)], false);
        assert!(store.has_newer_messages);
    }

    #[test]
    fn clearing_account_state_removes_cross_account_data() {
        let mut store = Store {
            selected_chat_id: Some("private-chat".into()),
            total_chats: 1,
            qr_code: Some("stale".into()),
            ..Default::default()
        };
        store.upsert_chat(whatfast_protocol::Chat {
            id: "private-chat".into(),
            ..Default::default()
        });
        store.upsert_message(message(1));
        store.apply_sync_status(whatfast_protocol::SyncStatus {
            revision: 9,
            ..Default::default()
        });
        store.set_history_overview(whatfast_protocol::HistoryOverview {
            local_message_count: 12,
            ..Default::default()
        });
        store.set_integrity_status(whatfast_protocol::IntegrityStatus {
            database_ok: false,
            ..Default::default()
        });
        store.set_pinned_messages(
            "private-chat".into(),
            vec![whatfast_protocol::PinnedMessage {
                message_id: "1".into(),
                ..Default::default()
            }],
        );
        store.set_sticker_packs(vec![whatfast_protocol::StickerPack {
            id: "favorites".into(),
            ..Default::default()
        }]);
        store.set_statuses("status@broadcast".into(), Vec::new());
        store.set_newsletters(vec![whatfast_protocol::Newsletter {
            jid: "news@newsletter".into(),
            ..Default::default()
        }]);
        store.clear_account_state();
        assert!(store.chats.is_empty());
        assert!(store.messages.is_empty());
        assert!(store.selected_chat_id.is_none());
        assert!(store.qr_code.is_none());
        assert_eq!(store.total_chats, 0);
        assert!(store.sync_status.is_none());
        assert!(store.history_overview.is_none());
        assert!(store.integrity_status.is_none());
        assert!(store.pinned_messages.is_empty());
        assert!(store.sticker_packs.is_empty());
        assert!(store.newsletters.is_empty());
        assert!(store.statuses_chat_id.is_empty());
    }

    #[test]
    fn a_message_of_another_conversation_is_ignored() {
        let mut store = Store {
            selected_chat_id: Some("chat".into()),
            ..Default::default()
        };
        let mut other = message(1);
        other.chat_id = "other".into();
        store.upsert_message(other);

        assert!(store.messages.is_empty());
    }

    #[test]
    fn live_chat_reorder_updates_only_affected_indices_correctly() {
        let mut store = Store::default();
        store.replace_chat_page(
            "",
            (0..1_000)
                .map(|id| whatfast_protocol::Chat {
                    id: format!("chat-{id}"),
                    last_message_timestamp_ms: 1_000 - id,
                    ..Default::default()
                })
                .collect(),
            1_000,
            String::new(),
        );
        let moved_id = store.chats[800].id.clone();
        store.upsert_chat(whatfast_protocol::Chat {
            id: moved_id.clone(),
            last_message_timestamp_ms: 50_000,
            ..Default::default()
        });
        assert_eq!(store.chats[0].id, moved_id);
        for (index, chat) in store.chats.iter().enumerate() {
            assert_eq!(store.chat_index.get(&chat.id), Some(&index));
        }
    }

    #[test]
    fn chat_merge_redirects_the_active_selection_and_removes_the_loser() {
        let mut store = Store::default();
        store.upsert_chat(whatfast_protocol::Chat {
            id: "c:old".into(),
            title: "Old".into(),
            ..Default::default()
        });
        store.upsert_chat(whatfast_protocol::Chat {
            id: "c:new".into(),
            title: "New".into(),
            ..Default::default()
        });
        store.select_chat("c:old".into());

        assert!(store.merge_chat_id("c:old", "c:new"));
        assert_eq!(store.selected_chat_id.as_deref(), Some("c:new"));
        assert!(store.chats.iter().all(|chat| chat.id != "c:old"));
        assert_eq!(store.chat_index.get("c:new"), Some(&0));
    }

    #[test]
    fn centered_message_window_preserves_order_and_navigation_flags() {
        let mut store = Store {
            selected_chat_id: Some("chat".into()),
            ..Default::default()
        };
        store.replace_message_window(vec![message(3), message(1), message(2)], true, true);
        assert_eq!(
            store
                .messages
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            ["1", "2", "3"]
        );
        assert!(store.has_older_messages);
        assert!(store.has_newer_messages);
        assert!(!store.newer_activity);
        assert_eq!(
            store.message("2").map(|message| message.id.as_str()),
            Some("2")
        );
    }

    fn chat(id: &str) -> whatfast_protocol::Chat {
        whatfast_protocol::Chat {
            id: id.into(),
            title: id.into(),
            ..Default::default()
        }
    }

    fn poll_message(id: &str) -> whatfast_protocol::Message {
        let mut message = message(1);
        message.id = id.into();
        message.content = Some(whatfast_protocol::message::Content::Poll(
            whatfast_protocol::PollContent {
                question: "Lunch?".into(),
                selectable_options_count: 2,
                total_voters: 1,
                options: vec![
                    whatfast_protocol::PollOption {
                        name: "Pizza".into(),
                        vote_count: 1,
                        selected_by_me: true,
                        voters: vec![whatfast_protocol::PollVoter {
                            user_id: "me".into(),
                            display_name: "You".into(),
                            from_me: true,
                            ..Default::default()
                        }],
                    },
                    whatfast_protocol::PollOption {
                        name: "Sushi".into(),
                        vote_count: 0,
                        selected_by_me: false,
                        voters: Vec::new(),
                    },
                ],
            },
        ));
        message
    }

    #[test]
    fn the_sync_status_ignores_an_older_revision() {
        let mut store = Store::default();
        assert!(store.apply_sync_status(whatfast_protocol::SyncStatus {
            revision: 4,
            chats_processed: 10,
            ..Default::default()
        }));
        assert!(!store.apply_sync_status(whatfast_protocol::SyncStatus {
            revision: 3,
            chats_processed: 99,
            ..Default::default()
        }));
        assert_eq!(
            store.sync_status.as_ref().map(|status| status.revision),
            Some(4)
        );
        assert_eq!(
            store
                .sync_status
                .as_ref()
                .map(|status| status.chats_processed),
            Some(10)
        );
    }

    #[test]
    fn the_history_coverage_ignores_an_older_revision() {
        let mut store = Store::default();
        let coverage = |revision: u64| whatfast_protocol::HistoryCoverage {
            chat_id: "chat".into(),
            revision,
            local_message_count: revision,
            ..Default::default()
        };
        assert!(store.set_history_coverage(coverage(7)));
        assert!(!store.set_history_coverage(coverage(6)));
        assert_eq!(
            store
                .history_coverage("chat")
                .map(|coverage| coverage.local_message_count),
            Some(7)
        );
        assert!(store.history_coverage("other").is_none());
    }

    #[test]
    fn the_overview_and_the_integrity_status_keep_the_last_answer() {
        let mut store = Store::default();
        store.set_history_overview(whatfast_protocol::HistoryOverview {
            local_message_count: 120,
            chats_with_messages: 3,
            ..Default::default()
        });
        store.set_integrity_status(whatfast_protocol::IntegrityStatus {
            database_ok: false,
            pending_inbound: 5,
            ..Default::default()
        });

        assert_eq!(
            store
                .history_overview
                .as_ref()
                .map(|overview| overview.local_message_count),
            Some(120)
        );
        assert_eq!(
            store
                .integrity_status
                .as_ref()
                .map(|status| status.pending_inbound),
            Some(5)
        );
        assert!(
            store
                .integrity_status
                .as_ref()
                .is_some_and(|status| !status.database_ok)
        );
    }

    #[test]
    fn the_pinned_messages_are_kept_per_conversation() {
        let mut store = Store::default();
        let pin = |message_id: &str| whatfast_protocol::PinnedMessage {
            message_id: message_id.into(),
            pinned_at_ms: 1_900_000_000_000,
            pinned_by: "me".into(),
            available: true,
            ..Default::default()
        };
        store.set_pinned_messages("chat".into(), vec![pin("m1")]);
        store.set_pinned_messages("other".into(), vec![pin("m2")]);

        assert_eq!(store.pinned_messages("chat").len(), 1);
        assert!(store.message_is_pinned("chat", "m1"));
        assert!(!store.message_is_pinned("chat", "m2"));
        assert!(store.message_is_pinned("other", "m2"));
        assert!(store.pinned_messages("missing").is_empty());
    }

    #[test]
    fn the_sticker_library_is_searchable_by_id() {
        let mut store = Store::default();
        store.set_sticker_packs(vec![whatfast_protocol::StickerPack {
            id: "favorites".into(),
            name: "Favourites".into(),
            stickers: vec![whatfast_protocol::Sticker {
                id: "fav-1".into(),
                favorite: true,
                ..Default::default()
            }],
        }]);

        assert_eq!(store.sticker_packs().len(), 1);
        assert_eq!(
            store.find_sticker("fav-1").map(|sticker| sticker.favorite),
            Some(true)
        );
        assert!(store.find_sticker("missing").is_none());
    }

    #[test]
    fn the_statuses_keep_the_conversation_that_carries_them() {
        let mut store = Store::default();
        store.set_statuses(
            "status@broadcast".into(),
            vec![whatfast_protocol::StatusGroup {
                sender_id: "maya@s.whatsapp.net".into(),
                sender_name: "Maya Nair".into(),
                ..Default::default()
            }],
        );

        assert_eq!(store.statuses().len(), 1);
        assert_eq!(store.statuses_chat_id(), "status@broadcast");
    }

    #[test]
    fn the_newsletters_are_added_once_and_dropped_by_jid() {
        let mut store = Store::default();
        store.set_newsletters(vec![whatfast_protocol::Newsletter {
            jid: "news@newsletter".into(),
            name: "Whatfast News".into(),
            ..Default::default()
        }]);
        store.add_newsletter(whatfast_protocol::Newsletter {
            jid: "news@newsletter".into(),
            name: "Whatfast News Renamed".into(),
            ..Default::default()
        });

        assert_eq!(store.newsletters().len(), 1);
        assert_eq!(store.newsletters()[0].name, "Whatfast News Renamed");

        assert!(store.remove_newsletter("news@newsletter"));
        assert!(!store.remove_newsletter("news@newsletter"));
        assert!(store.newsletters().is_empty());
    }

    #[test]
    fn the_posts_of_a_newsletter_are_kept_per_jid() {
        let mut store = Store::default();
        let mut post = message(1);
        post.chat_id = "news@newsletter".into();
        store.set_newsletter_messages("news@newsletter".into(), vec![post]);

        assert_eq!(store.newsletter_messages("news@newsletter").len(), 1);
        assert!(store.newsletter_messages("other").is_empty());
    }

    #[test]
    fn the_path_of_an_attachment_lands_on_its_message() {
        let mut store = Store {
            selected_chat_id: Some("chat".into()),
            ..Default::default()
        };
        let mut with_attachment = message(1);
        with_attachment.content = Some(whatfast_protocol::message::Content::Attachment(
            whatfast_protocol::AttachmentContent {
                file_name: "agenda.pdf".into(),
                downloadable: true,
                ..Default::default()
            },
        ));
        store.upsert_message(with_attachment);

        assert!(store.set_message_attachment_path("chat", "1", "/cache/agenda.pdf".into()));
        assert!(
            !store.set_message_attachment_path("chat", "1", "/cache/agenda.pdf".into()),
            "the same path is not a change"
        );
        let Some(whatfast_protocol::message::Content::Attachment(attachment)) =
            store.messages[0].content.as_ref()
        else {
            panic!("attachment content missing");
        };
        assert_eq!(attachment.local_path, "/cache/agenda.pdf");
        assert!(!store.set_message_attachment_path("other", "1", "/cache/x".into()));
        assert!(!store.set_message_attachment_path("chat", "missing", "/cache/x".into()));
    }

    #[test]
    fn pinning_a_conversation_moves_it_to_the_front_of_the_sidebar() {
        let mut store = Store::default();
        store.replace_chat_page(
            "",
            vec![chat("first"), chat("second"), chat("third")],
            3,
            String::new(),
        );

        assert!(store.set_chat_pinned("third", true));
        assert_eq!(store.chats[0].id, "third");
        assert!(store.chat("third").is_some_and(|chat| chat.pinned));

        assert!(store.set_chat_pinned("third", false));
        assert_eq!(store.chats[0].id, "first");
        assert!(!store.set_chat_pinned("missing", true));
    }

    #[test]
    fn archiving_a_conversation_takes_it_out_of_the_inbox() {
        let mut store = Store::default();
        store.replace_chat_page("", vec![chat("inbox"), chat("archived")], 2, String::new());

        assert!(store.set_chat_archived("archived", true));
        assert_eq!(store.chat_ids(ChatFilter::All).as_slice(), ["inbox"]);
        assert_eq!(
            store.chat_ids(ChatFilter::Archived).as_slice(),
            ["archived"]
        );

        assert!(store.set_chat_archived("archived", false));
        assert_eq!(
            store.chat_ids(ChatFilter::All).as_slice(),
            ["archived", "inbox"]
        );
        assert!(!store.set_chat_archived("missing", true));
    }

    #[test]
    fn muting_and_blocking_a_conversation_keep_their_flags() {
        let mut store = Store::default();
        store.replace_chat_page("", vec![chat("chat")], 1, String::new());

        assert!(store.set_chat_muted("chat", true, 1_900_000_000_000));
        assert!(store.chat("chat").is_some_and(|chat| chat.muted));
        assert_eq!(
            store.chat("chat").map(|chat| chat.mute_until_ms),
            Some(1_900_000_000_000)
        );

        assert!(store.set_chat_blocked("chat", true));
        assert!(store.chat("chat").is_some_and(|chat| chat.blocked));

        assert!(store.set_chat_muted("chat", false, 0));
        assert!(!store.chat("chat").is_some_and(|chat| chat.muted));
        assert_eq!(store.chat("chat").map(|chat| chat.mute_until_ms), Some(0));
        assert!(!store.set_chat_blocked("missing", true));
    }

    #[test]
    fn a_deleted_message_is_revoked_and_can_be_put_back() {
        let mut store = Store {
            selected_chat_id: Some("chat".into()),
            ..Default::default()
        };
        store.upsert_message(message(1));

        let previous = store
            .revoke_message("chat", "1")
            .expect("the message is there");
        assert!(store.messages[0].revoked);
        assert!(store.messages[0].content.is_none());
        assert_eq!(previous.id, "1");

        store.restore_message(previous);
        assert!(!store.messages[0].revoked);
        assert!(store.revoke_message("other", "1").is_none());
    }

    #[test]
    fn the_vote_of_a_poll_is_optimistic_and_can_be_rolled_back() {
        let mut store = Store {
            selected_chat_id: Some("chat".into()),
            ..Default::default()
        };
        store.upsert_message(poll_message("poll"));

        let previous = store
            .apply_poll_vote("chat", "poll", &["Sushi".to_string()])
            .expect("the poll is there");
        let Some(whatfast_protocol::message::Content::Poll(poll)) =
            store.messages[0].content.as_ref()
        else {
            panic!("poll content missing");
        };
        assert_eq!(poll.total_voters, 1);
        assert_eq!(poll.options[0].vote_count, 0);
        assert!(!poll.options[0].selected_by_me);
        assert_eq!(poll.options[1].vote_count, 1);
        assert!(poll.options[1].selected_by_me);

        assert!(store.restore_poll_content("chat", "poll", previous));
        let Some(whatfast_protocol::message::Content::Poll(restored)) =
            store.messages[0].content.as_ref()
        else {
            panic!("poll content missing");
        };
        assert_eq!(restored.options[0].vote_count, 1);
        assert!(restored.options[0].selected_by_me);
        assert!(store.apply_poll_vote("chat", "missing", &[]).is_none());
    }

    #[test]
    fn the_poll_reducer_retracts_and_restores_a_complete_vote_intent() {
        let mut store = Store {
            selected_chat_id: Some("chat".into()),
            ..Default::default()
        };
        store.upsert_message(poll_message("poll"));

        let retracted = store
            .apply_poll_vote("chat", "poll", &[])
            .expect("the poll is there");
        let Some(whatfast_protocol::message::Content::Poll(poll)) =
            store.messages[0].content.as_ref()
        else {
            panic!("poll content missing");
        };
        assert_eq!(poll.total_voters, 0);
        assert_eq!(poll.options[0].vote_count, 0);

        store.restore_poll_content("chat", "poll", retracted);
        store.apply_poll_vote("chat", "poll", &["Pizza".to_string(), "Sushi".to_string()]);
        let Some(whatfast_protocol::message::Content::Poll(restored)) =
            store.messages[0].content.as_ref()
        else {
            panic!("poll content missing");
        };
        let counts: Vec<u32> = restored
            .options
            .iter()
            .map(|option| option.vote_count)
            .collect();
        assert_eq!(counts, vec![1, 1]);
    }
}
