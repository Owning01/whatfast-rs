//! The state of the application and the actions that change it.
//!
//! port: taken from the origin's `desktop/src/main.rs`, the part that is not
//! drawing (lines 80-3116): the small types, the constants, the whole
//! `Whatfast` state and every action the user interface can trigger.
//!
//! The drawing lives in `crate::render`; the pure text helpers live in
//! `crate::text`; the lifecycle of the requests lives in `crate::rpc`. This
//! module owns the state and decides what to do with what the backend
//! answers.

// The drawing half of this batch (`crate::render`) is the consumer of many of
// the items below: the types it reads, the constants it paints with and the
// actions it binds. While the two halves live in different branches the lint
// cannot see the uses.
// TODO(port): drop this allow once `render.rs` is in the same crate.
#![allow(dead_code)]

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{
    AnyWindowHandle, App, AppContext as _, Context, Entity, FocusHandle, Focusable as _,
    PathPromptOptions, Pixels, Point, ScrollDelta, ScrollHandle, ScrollStrategy, ScrollWheelEvent,
    Subscription, UniformListScrollHandle, WeakEntity, Window, actions, point, px,
};
use gpui_component::{
    Theme, ThemeMode, VirtualListScrollHandle,
    input::{InputEvent, InputState},
};
use smol::Timer;
use uuid::Uuid;
use whatfast_core::theme::{Theme as AppTheme, ThemeAppearance, ThemeToken};
use whatfast_i18n as i18n;
use whatfast_model::{
    emoji_picker::{EmojiCategory, filtered as filter_emojis},
    favorites,
    model::{CHAT_PAGE_SIZE, ChatFilter, MESSAGE_PAGE_SIZE, Screen, Store, reactions_for_emoji},
    settings::{self, Density, Settings},
    starred, sticker,
};
use whatfast_protocol::{
    AttachmentKind, BackendEvent, Chat, ChatParticipant, ConnectionState, ContactSearchResult,
    Message, MessageSearchResult, RpcResponse, backend_event, rpc_request, rpc_response,
};

use crate::audio::{VoiceAudio, VoiceState};
use crate::composer_attachments::PendingImages;
use crate::media_editor::{MediaEditor, MediaEditorEvent};
use crate::members::MemberPanel;
use crate::message_actions::{
    AttachmentAction, AttachmentBar, ForwardAction, ForwardPicker, MessageAction, MessageMenu,
    PollAction, PollComposer,
};
use crate::navrail::{LinkState, NavAction, NavRail};
use crate::newsletters::NewsletterPanel;
use crate::notifications::{Note, Notifier};
use crate::recording::{MAX_DURATION_SECONDS, MIN_DURATION_SECONDS, RecorderEvent, VoiceRecorder};
use crate::rpc::{
    BridgeMessage, OutgoingAttachment, PROTOCOL_VERSION, PendingRequest, REACTION_REPAIR_RETRY,
    RpcClient, RpcIncoming, TIMEOUT_SWEEP_INTERVAL, avatar_retry, reaction_retry,
};
use crate::statuses::StatusStrip;
use crate::stickers::StickerTray;
use crate::tabs::{TabAction, TabEntry, TabStrip};
use crate::text::{
    encode_mentions, format_typing_label, mention_display_name, mention_token_start,
    normalized_ui_scale, validate_image_message, validate_text_message,
};
use crate::tokens;

// The actions the application answers to.
//
// port: the origin declared them at the root of its `main.rs`. They live
// here because the global handler of the chat switcher is a method of
// `Whatfast`. **They must not be declared twice**: GPUI panics when two
// actions share a name. The user interface imports them from this module.
//
// The three destinations that are not conversations (the states, the
// channels and the archived ones) also answer to the keyboard: they are
// views of their own, and the column of icons is not the only way in.
actions!(
    whatfast,
    [
        CycleRecentChat,
        CycleRecentChatReverse,
        ToggleStates,
        ToggleChannels,
        ToggleArchived
    ]
);

/// The colour of the accent of the light theme.
pub(crate) const DARK_GREEN: u32 = 0x075e54;
/// The colour of the background of the light theme.
pub(crate) const PANEL: u32 = 0xf7faf9;
/// How many emoji the picker shows per row.
pub(crate) const EMOJI_COLUMNS: usize = 9;
/// How often a participant's avatar is downloaded again.
pub(crate) const PARTICIPANT_AVATAR_REFRESH: Duration = Duration::from_secs(15 * 60);
/// The biggest text message the client sends.
pub(crate) const MAX_TEXT_BYTES: usize = 65_536;
/// The biggest caption the client sends with an image.
pub(crate) const MAX_CAPTION_BYTES: usize = 4_096;
/// The biggest image the client sends.
pub(crate) const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;
// The scale of the interface belongs to the tokens of the theme: they are
// re-exported here so that the rest of the crate keeps reading them from
// the state module, and there is only one definition.
pub(crate) use crate::tokens::{MAX_UI_SCALE, MIN_UI_SCALE, UI_SCALE_STEP};
/// How many conversations the recency history keeps.
pub(crate) const MAX_RECENT_CHATS: usize = 10;
/// The width of the column of the chats, before the scale of the screen.
pub(crate) const DEFAULT_SIDEBAR_WIDTH: f32 = 384.0;
/// The narrowest the column of the chats may be dragged to.
pub(crate) const MIN_SIDEBAR_WIDTH: f32 = 260.0;
/// The widest the column of the chats may be dragged to.
pub(crate) const MAX_SIDEBAR_WIDTH: f32 = 560.0;
/// How long two wheel events may be apart to be part of the same gesture.
pub(crate) const SMOOTH_SCROLL_INPUT_RESET: Duration = Duration::from_millis(180);
/// How long one frame of the inertial scroll lasts.
pub(crate) const SMOOTH_SCROLL_FRAME: Duration = Duration::from_millis(16);
/// The speed one wheel event adds.
pub(crate) const SMOOTH_SCROLL_IMPULSE: f32 = 900.0;
/// The fastest the inertial scroll may go.
pub(crate) const SMOOTH_SCROLL_MAX_VELOCITY: f32 = 3_600.0;
/// How fast the inertial scroll loses speed.
pub(crate) const SMOOTH_SCROLL_FRICTION: f32 = 10.0;
/// The speed under which the inertial scroll stops.
pub(crate) const SMOOTH_SCROLL_STOP_VELOCITY: f32 = 18.0;
/// How many participants the mention picker shows at most.
pub(crate) const MAX_MENTION_ROWS: usize = 8;
/// How many posts of a newsletter the client asks for.
pub(crate) const NEWSLETTER_PAGE_SIZE: i32 = 25;
/// How often the statuses of the profile are read again.
///
/// port: the origin polls the store every forty five seconds, because the
/// statuses arrive without a push event.
pub(crate) const STATUS_REFRESH: Duration = Duration::from_secs(45);
/// The moment a conversation muted without an end stops being muted.
///
/// port: the origin uses the last moment of the calendar of JavaScript,
/// `253_402_300_799_000`, for a mute that has no end.
pub(crate) const MUTE_UNTIL_INDEFINITE: i64 = 253_402_300_799_000;
/// How long a typing indicator lives without an update.
pub(crate) const TYPING_INDICATOR_TTL: Duration = Duration::from_secs(10);
/// How often the composer re-broadcasts "still typing" while text keeps
/// changing. WhatsApp peers expire remote composing state after ~10 seconds.
pub(crate) const TYPING_RESIGNAL: Duration = Duration::from_secs(8);

/// The surface a scroll gesture happened on.
#[derive(Clone, Copy)]
pub(crate) enum ScrollSurface {
    ChatList,
    Search,
    Messages,
    ChatInfo,
}

/// Adds a short inertial tail to discrete mouse-wheel input. Precise pixel
/// input is left alone because touchpads already provide their own gesture
/// phases and momentum through the window system.
#[derive(Default)]
pub(crate) struct SmoothScrollState {
    pub(crate) velocity_y: f32,
    pub(crate) last_input: Option<Instant>,
    pub(crate) running: bool,
    pub(crate) generation: u64,
}

impl SmoothScrollState {
    fn push_wheel(&mut self, delta_y: f32, now: Instant) -> Option<u64> {
        let direction = delta_y.signum();
        if direction == 0.0 {
            return None;
        }
        let continues_gesture = self.velocity_y.signum() == direction
            && self
                .last_input
                .is_some_and(|last| now.duration_since(last) <= SMOOTH_SCROLL_INPUT_RESET);
        if !continues_gesture {
            self.velocity_y = 0.0;
        }
        self.velocity_y = (self.velocity_y + direction * SMOOTH_SCROLL_IMPULSE)
            .clamp(-SMOOTH_SCROLL_MAX_VELOCITY, SMOOTH_SCROLL_MAX_VELOCITY);
        self.last_input = Some(now);
        if self.running {
            None
        } else {
            self.running = true;
            self.generation = self.generation.wrapping_add(1);
            Some(self.generation)
        }
    }

    fn advance(&mut self, generation: u64, elapsed_seconds: f32) -> Option<f32> {
        if !self.running || self.generation != generation {
            return None;
        }
        if self.velocity_y.abs() < SMOOTH_SCROLL_STOP_VELOCITY {
            self.finish();
            return None;
        }
        let distance = self.velocity_y * elapsed_seconds;
        self.velocity_y *= (-SMOOTH_SCROLL_FRICTION * elapsed_seconds).exp();
        Some(distance)
    }

    fn finish(&mut self) {
        self.velocity_y = 0.0;
        self.running = false;
        self.generation = self.generation.wrapping_add(1);
    }
}

/// Where an emoji goes when the user picks one.
#[derive(Clone, Debug)]
pub(crate) enum EmojiTarget {
    Composer,
    Reaction { chat_id: String, message_id: String },
}

/// The reactions of one emoji of one message.
#[derive(Clone)]
pub(crate) struct ReactionDetails {
    pub(crate) chat_id: String,
    pub(crate) message_id: String,
    pub(crate) emoji: String,
    pub(crate) reactions: Rc<Vec<whatfast_protocol::Reaction>>,
}

/// The image the user is looking at, full size.
#[derive(Clone, Debug)]
pub(crate) struct ImageViewer {
    pub(crate) chat_id: String,
    pub(crate) message_id: String,
    pub(crate) path: String,
    pub(crate) caption: String,
    pub(crate) sticker: bool,
}

/// The voice note that the microphone is keeping.
pub(crate) struct RecordingState {
    /// When the capture started: the clock of the composer counts from here.
    pub(crate) started: Instant,
    /// Whether the user already pressed send and the encoder is working.
    pub(crate) finishing: bool,
    /// The conversation where the note started: the note goes there even if
    /// the user moves to another chat while the encoder finishes.
    pub(crate) chat_id: String,
    /// The user threw the note away: the file is deleted when it comes out
    /// of the encoder, even if a `Stop` was already queued before the
    /// `Cancel`.
    pub(crate) discarded: bool,
}

impl RecordingState {
    /// The length of the note so far, in seconds.
    pub(crate) fn elapsed_seconds(&self) -> f32 {
        self.started.elapsed().as_secs_f32()
    }
}

/// The conversation that gets the note, or `None` when the note was thrown
/// away before the encoder finished.
fn note_target(state: Option<&RecordingState>) -> Option<&str> {
    state
        .filter(|state| !state.discarded)
        .map(|state| state.chat_id.as_str())
}

/// What the user wrote in one conversation, and what it replies to.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ChatDraft {
    pub(crate) text: String,
    pub(crate) reply_to_message_id: Option<String>,
    /// Participants tagged via the @ picker. Each entry backs an `@Name`
    /// token in `text`; tokens the user has since deleted are dropped at send.
    pub(crate) mentions: Vec<MentionEntry>,
    /// The message the user is editing, when the composer holds an edit and
    /// not a new message.
    ///
    /// port: the origin keeps the same field in its draft
    /// (`editingMessageId`) and sends an edit instead of a text when it is
    /// there.
    pub(crate) editing_message_id: Option<String>,
}

/// One participant the user tagged with the @ picker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MentionEntry {
    pub(crate) display_name: String,
    pub(crate) jid: String,
}

/// The mention picker that is open over the composer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MentionPicker {
    pub(crate) chat_id: String,
    /// Byte offset of the `@` that opened the picker in the composer text.
    pub(crate) token_start: usize,
    pub(crate) query: String,
    pub(crate) highlighted: usize,
}

impl ChatDraft {
    /// Whether the draft holds nothing.
    ///
    /// port: the origin checks the same fields of its draft
    /// (`draftIsEmpty`): a draft that edits a message is not empty, because
    /// the message it edits is still the one the user is working on.
    pub(crate) fn is_empty(&self) -> bool {
        self.text.is_empty()
            && self.reply_to_message_id.is_none()
            && self.editing_message_id.is_none()
            && self.mentions.is_empty()
    }
}

/// The switcher that cycles through the conversations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ChatSwitcher {
    pub(crate) chat_ids: Vec<String>,
    pub(crate) highlighted: usize,
}

/// The panel with the details of one conversation.
pub(crate) struct ChatInfoState {
    pub(crate) chat_id: String,
    pub(crate) loading: bool,
    pub(crate) error: Option<String>,
    pub(crate) info: Option<whatfast_protocol::GetChatInfoResponse>,
}

/// The menu that the right click opens on a row of the list of conversations.
///
/// port: the `chat-context-menu` of the origin
/// (`tauri/src/components/Sidebar.tsx`): it opens where the pointer is and
/// it works on the conversation under the pointer.
pub(crate) struct ChatContextMenu {
    /// The conversation the menu works on.
    pub(crate) chat_id: String,
    /// Where the pointer was when the menu opened, in pixels of the window.
    pub(crate) position: Point<Pixels>,
}

/// The actions of the menu of a row of the list of conversations.
///
/// port: the items of the `chat-context-menu` of the origin
/// (`tauri/src/components/Sidebar.tsx`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ChatContextAction {
    /// Pins or unpins the conversation.
    Pin,
    /// Mutes or unmutes the conversation.
    Mute,
    /// Archives or unarchives the conversation.
    Archive,
    /// Marks the conversation as a favorite, or takes the mark away.
    Favorite,
    /// Marks the conversation as read.
    MarkRead,
    /// Marks the conversation as unread.
    MarkUnread,
    /// Blocks or unblocks the contact.
    Block,
}

/// The menu of the options of a conversation.
///
/// The reference of WhatsApp Desktop has many more rows (the calls, the
/// temporary messages, the export, the report): the menu only draws the
/// ones the app can do.
pub(crate) struct ChatMenu {
    /// The conversation the menu is of.
    pub(crate) chat_id: String,
    /// Where the menu opens, in pixels of the window.
    pub(crate) position: Point<Pixels>,
}

/// The actions of the menu of the options of a conversation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ChatMenuAction {
    /// Opens the info of the contact.
    Info,
    /// Searches in the conversation.
    Search,
    /// Mutes or unmutes the conversation.
    Mute,
    /// Marks the conversation as a favorite, or takes the mark away.
    Favorite,
    /// Blocks or unblocks the contact.
    Block,
}

/// The tab of the panel of the composer.
///
/// The panel of the reference has three tabs: the emojis, the pictures with
/// movement and the stickers. The app has no pictures with movement, so
/// that tab is drawn but it cannot be pressed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PanelTab {
    /// The emojis.
    Emoji,
    /// The pictures with movement.
    Gif,
    /// The stickers.
    Sticker,
}

/// Transient state for one peer. Each chat keeps a sender-keyed collection so
/// concurrent group participants do not overwrite one another.
pub(crate) struct TypingIndicator {
    pub(crate) sender_name: String,
    pub(crate) recording: bool,
    pub(crate) expires_at: Instant,
}

/// What the local search found.
#[derive(Clone, Debug, Default)]
pub(crate) struct SearchResults {
    pub(crate) contacts: Vec<ContactSearchResult>,
    pub(crate) groups: Vec<Chat>,
    pub(crate) messages: Vec<MessageSearchResult>,
}

/// One row of the list of results of the local search.
#[derive(Clone, Debug)]
pub(crate) enum SearchResultRow {
    Header {
        title: &'static str,
        count: usize,
    },
    Contact {
        result_index: usize,
        source_index: usize,
    },
    Group {
        result_index: usize,
        source_index: usize,
    },
    Message {
        result_index: usize,
        source_index: usize,
    },
}

/// What a result of the local search points at.
#[derive(Clone, Debug)]
pub(crate) enum SearchTarget {
    Contact(String),
    Group(Chat),
    Message {
        chat: Option<Chat>,
        chat_id: String,
        message_id: String,
    },
}

impl SearchResults {
    pub(crate) fn len(&self) -> usize {
        self.contacts.len() + self.groups.len() + self.messages.len()
    }

    pub(crate) fn target(&self, mut index: usize) -> Option<SearchTarget> {
        if let Some(contact) = self.contacts.get(index) {
            return Some(SearchTarget::Contact(contact.contact_jid.clone()));
        }
        index = index.checked_sub(self.contacts.len())?;
        if let Some(group) = self.groups.get(index) {
            return Some(SearchTarget::Group(group.clone()));
        }
        index = index.checked_sub(self.groups.len())?;
        self.messages
            .get(index)
            .map(|message| SearchTarget::Message {
                chat: message.chat.clone(),
                chat_id: message.chat_id.clone(),
                message_id: message.message_id.clone(),
            })
    }

    pub(crate) fn rows(&self) -> Vec<SearchResultRow> {
        let mut rows = Vec::with_capacity(self.len() + 3);
        if !self.contacts.is_empty() {
            rows.push(SearchResultRow::Header {
                title: "Contacts",
                count: self.contacts.len(),
            });
            rows.extend(self.contacts.iter().enumerate().map(|(result_index, _)| {
                SearchResultRow::Contact {
                    result_index,
                    source_index: result_index,
                }
            }));
        }
        if !self.groups.is_empty() {
            rows.push(SearchResultRow::Header {
                title: "Groups",
                count: self.groups.len(),
            });
            rows.extend(
                self.groups
                    .iter()
                    .enumerate()
                    .map(|(offset, _)| SearchResultRow::Group {
                        result_index: self.contacts.len() + offset,
                        source_index: offset,
                    }),
            );
        }
        if !self.messages.is_empty() {
            rows.push(SearchResultRow::Header {
                title: "Messages",
                count: self.messages.len(),
            });
            rows.extend(self.messages.iter().enumerate().map(|(offset, _)| {
                SearchResultRow::Message {
                    result_index: self.contacts.len() + self.groups.len() + offset,
                    source_index: offset,
                }
            }));
        }
        rows
    }

    pub(crate) fn row_index_for_result(&self, result_index: usize) -> Option<usize> {
        let mut row_start = 0;
        let mut result_start = 0;
        for count in [self.contacts.len(), self.groups.len(), self.messages.len()] {
            if count == 0 {
                continue;
            }
            row_start += 1; // section header
            if result_index < result_start + count {
                return Some(row_start + result_index - result_start);
            }
            row_start += count;
            result_start += count;
        }
        None
    }
}

impl ChatSwitcher {
    pub(crate) fn new(
        chat_ids: Vec<String>,
        reverse: bool,
        selected_is_first: bool,
    ) -> Option<Self> {
        // With the active chat at the front there must be something else to
        // switch to; without one, any single candidate is a valid target.
        let min_len = if selected_is_first { 2 } else { 1 };
        if chat_ids.len() < min_len {
            return None;
        }
        let highlighted = match (reverse, selected_is_first) {
            (true, _) => chat_ids.len() - 1,
            (false, true) => 1,
            (false, false) => 0,
        };
        Some(Self {
            chat_ids,
            highlighted,
        })
    }

    pub(crate) fn cycle(&mut self, reverse: bool) {
        if reverse {
            self.highlighted = self
                .highlighted
                .checked_sub(1)
                .unwrap_or(self.chat_ids.len() - 1);
        } else {
            self.highlighted = (self.highlighted + 1) % self.chat_ids.len();
        }
    }

    pub(crate) fn selected_chat_id(&self) -> Option<&str> {
        self.chat_ids.get(self.highlighted).map(String::as_str)
    }
}

/// Puts a conversation at the front of the recency history.
fn record_recent_chat(history: &mut Vec<String>, chat_id: &str) {
    history.retain(|existing| existing != chat_id);
    history.insert(0, chat_id.to_owned());
    history.truncate(MAX_RECENT_CHATS);
}

/// Follows a conversation that was merged into another one.
fn remap_recent_chat_ids(history: &mut Vec<String>, old_id: &str, new_id: &str) {
    for chat_id in history.iter_mut() {
        if chat_id == old_id {
            *chat_id = new_id.to_owned();
        }
    }
    let mut seen = HashSet::new();
    history.retain(|chat_id| seen.insert(chat_id.clone()));
    history.truncate(MAX_RECENT_CHATS);
}

/// Follows the draft of a conversation that was merged into another one.
fn remap_chat_draft(
    drafts: &mut HashMap<String, ChatDraft>,
    old_id: &str,
    new_id: &str,
    old_was_selected: bool,
) {
    let Some(old_draft) = drafts.remove(old_id) else {
        return;
    };
    if old_was_selected {
        drafts.insert(new_id.to_owned(), old_draft);
    } else {
        drafts.entry(new_id.to_owned()).or_insert(old_draft);
    }
}

impl ReactionDetails {
    fn new(chat_id: String, message: &whatfast_protocol::Message, emoji: String) -> Option<Self> {
        let reactions = reactions_for_emoji(message, &emoji);
        (!reactions.is_empty()).then(|| Self {
            chat_id,
            message_id: message.id.clone(),
            emoji,
            reactions: Rc::new(reactions),
        })
    }
}

/// The state of the application.
pub(crate) struct Whatfast {
    pub(crate) rpc: RpcClient,
    pub(crate) store: Store,
    pub(crate) composer: Entity<InputState>,
    /// The pictures that the user pasted or picked and that wait for the
    /// button of the send, the same way the reference shows them over the
    /// field of the message.
    pub(crate) pending_images: PendingImages,
    pub(crate) search_input: Entity<InputState>,
    pub(crate) search_query: String,
    pub(crate) search_results: SearchResults,
    pub(crate) search_generation: u64,
    pub(crate) search_highlighted: usize,
    pub(crate) search_error: Option<String>,
    pub(crate) search_scroll: VirtualListScrollHandle,
    pub(crate) search_smooth_scroll: SmoothScrollState,
    pub(crate) search_target_message_id: Option<String>,
    pub(crate) pending_search_scroll_id: Option<String>,
    pub(crate) pending_top_scroll_id: Option<String>,
    pub(crate) emoji_search: Entity<InputState>,
    pub(crate) emoji_query: String,
    pub(crate) emoji_category: EmojiCategory,
    pub(crate) emoji_results: Rc<Vec<&'static emojis::Emoji>>,
    pub(crate) emoji_target: Option<EmojiTarget>,
    pub(crate) reaction_details: Option<ReactionDetails>,
    pub(crate) image_viewer: Option<ImageViewer>,
    pub(crate) replying_to_message_id: Option<String>,
    pub(crate) chat_drafts: HashMap<String, ChatDraft>,
    pub(crate) recent_chat_ids: Vec<String>,
    pub(crate) chat_switcher: Option<ChatSwitcher>,
    pub(crate) chat_info: Option<ChatInfoState>,
    pub(crate) chat_context_menu: Option<ChatContextMenu>,
    pub(crate) chat_menu: Option<ChatMenu>,
    pub(crate) mention_picker: Option<MentionPicker>,
    pub(crate) mention_directories: HashMap<String, Vec<ChatParticipant>>,
    pub(crate) mention_directory_pending: HashSet<String>,
    pub(crate) typing_indicators: HashMap<String, HashMap<String, TypingIndicator>>,
    pub(crate) typing_signal: Option<(String, Instant)>,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) settings_open: bool,
    pub(crate) ui_scale: f32,
    /// The settings of the profile, as the file keeps them.
    pub(crate) settings: Settings,
    /// The air between rows and messages, mirrored for the hot path of the
    /// drawing.
    pub(crate) density: Density,
    /// Whether a plain Enter sends the message.
    pub(crate) enter_to_send: bool,
    /// Whether the chat list stays beside the conversation.
    pub(crate) split_view: bool,
    /// Whether the wallpaper and the smooth scrolling are off.
    pub(crate) battery_saver: bool,
    /// The theme in use: a builtin id or the id of a custom one.
    pub(crate) theme_id: String,
    /// The themes the user made.
    pub(crate) custom_themes: Vec<AppTheme>,
    /// The custom theme whose colors are being edited.
    pub(crate) editing_theme: Option<String>,
    /// The queue of the toasts of the system.
    pub(crate) notifier: Notifier,
    pub(crate) chat_scroll: UniformListScrollHandle,
    pub(crate) chat_smooth_scroll: SmoothScrollState,
    pub(crate) message_scroll: VirtualListScrollHandle,
    pub(crate) message_smooth_scroll: SmoothScrollState,
    pub(crate) chat_info_scroll: ScrollHandle,
    pub(crate) chat_info_smooth_scroll: SmoothScrollState,
    pub(crate) scroll_to_bottom_generation: Option<u64>,
    pub(crate) last_event_sequence: u64,
    pub(crate) pending_prepend_anchor: Option<(String, Point<Pixels>)>,
    pub(crate) newer_load_failed: bool,
    pub(crate) message_generation: u64,
    pub(crate) sync_complete_reloaded: bool,
    pub(crate) confirming_logout: bool,
    pub(crate) avatar_attempted: HashSet<String>,
    pub(crate) avatar_retries: HashMap<String, u8>,
    pub(crate) participant_avatar_attempted: HashSet<String>,
    pub(crate) image_download_attempted: HashSet<(String, String)>,
    pub(crate) image_failures: HashMap<(String, String), String>,
    pub(crate) sticker_preparing: bool,
    pub(crate) reaction_repair_attempted: HashSet<String>,
    pub(crate) latest_reaction_intents: HashMap<(String, String), String>,
    /// The generation of every vote of a poll that is in flight, so an old
    /// answer cannot overwrite a newer intent of the user.
    pub(crate) poll_vote_generations: HashMap<(String, String), u64>,
    /// The poll of every message with a vote in flight, kept to roll the
    /// optimistic change back and to protect it from stale snapshots.
    pub(crate) pending_poll_votes: HashMap<(String, String), (u64, whatfast_protocol::PollContent)>,
    /// The message of every deletion in flight, kept to roll the optimistic
    /// change back.
    pub(crate) pending_message_deletes: HashMap<(String, String), whatfast_protocol::Message>,
    /// The generation of every pin of a message that is in flight.
    pub(crate) pin_generations: HashMap<(String, String), u64>,
    /// The attachments whose download was already asked for.
    pub(crate) attachment_download_attempted: HashSet<(String, String)>,
    /// The attachments whose download failed.
    pub(crate) attachment_failures: HashMap<(String, String), String>,
    /// The attachment that the user asked to open and that is not in the
    /// disk yet: it goes to the program of the system when it arrives.
    pub(crate) attachment_open_requested: Option<(String, String)>,
    /// The attachment that the user asked to download: it is copied to the
    /// folder of the downloads when the file arrives, without opening it.
    pub(crate) attachment_save_requested: Option<(String, String)>,
    /// The width of the column of the chats, before the scale of the screen.
    pub(crate) sidebar_width: f32,
    /// Whether the column of the chats is hidden, so the conversation takes
    /// the whole window.
    pub(crate) sidebar_hidden: bool,
    /// Whether the border of the column of the chats is being dragged.
    pub(crate) sidebar_dragging: bool,
    /// The voice note that the window is playing, when there is one.
    pub(crate) voice: VoiceState,
    /// The thread of the sound of the voice notes.
    pub(crate) voice_audio: VoiceAudio,
    /// Whether the tick that moves the wave of the voice note is running.
    pub(crate) voice_tick: bool,
    /// The thread of the microphone that records the notes.
    pub(crate) recorder: VoiceRecorder,
    /// The note that is being recorded, when there is one.
    pub(crate) recording: Option<RecordingState>,
    /// Whether the tick that counts the recording is running.
    pub(crate) recording_tick: bool,
    /// Whether the health of the local database is being checked.
    pub(crate) integrity_checking: bool,
    /// Whether the library of stickers is being loaded.
    pub(crate) sticker_library_loading: bool,
    /// Whether the newsletters are being loaded.
    pub(crate) newsletters_loading: bool,
    /// The newsletter whose posts are open.
    pub(crate) selected_newsletter: Option<String>,
    /// Whether the view of the states is in front of the column of chats.
    ///
    /// The states are not conversations: they have their own view, the same
    /// way the reference draws them.
    pub(crate) states_open: bool,
    /// Whether the view of the channels is in front of the column of chats.
    ///
    /// The channels are not conversations either: they have their own view.
    pub(crate) channels_open: bool,
    /// Whether the view of the archived conversations is in front.
    pub(crate) archived_open: bool,
    /// When the statuses of the profile were read the last time.
    pub(crate) last_status_refresh: Instant,
    /// The column of icons at the left of the window.
    pub(crate) nav_rail: Entity<NavRail>,
    /// The strip of the open chats.
    pub(crate) tab_strip: Entity<TabStrip>,
    /// The editor of a picture, when it is open.
    pub(crate) media_editor: Option<Entity<MediaEditor>>,
    /// The panel of the newsletters.
    pub(crate) newsletter_panel: NewsletterPanel,
    /// The strip of the states of the contacts.
    pub(crate) status_strip: StatusStrip,
    /// The panel of the members of a group.
    pub(crate) member_panel: MemberPanel,
    /// The tray of the stickers.
    pub(crate) sticker_tray: StickerTray,
    /// Whether the tray of the stickers is open.
    pub(crate) sticker_tray_open: bool,
    /// The tab of the panel of the composer.
    pub(crate) panel_tab: PanelTab,
    /// The menu of a message, when it is open.
    pub(crate) message_menu: Option<Entity<MessageMenu>>,
    /// The dialog to forward a message.
    pub(crate) forward_picker: Option<Entity<ForwardPicker>>,
    /// The form of a poll.
    pub(crate) poll_composer: Option<Entity<PollComposer>>,
    /// The button of the clip of the composer.
    pub(crate) attachment_bar: Entity<AttachmentBar>,
    pub(crate) _subscriptions: Vec<Subscription>,
}

impl Whatfast {
    /// Builds the application: the inputs, the subscriptions, the thread that
    /// reads from the backend and the sweep of the requests that time out.
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut settings = settings::load();
        // The language of the file decides the first frame; an unknown code
        // (a hand-edited file, a removed language) falls back to English.
        if !i18n::set_language(&settings.language) {
            i18n::set_language(i18n::DEFAULT_LANGUAGE);
            settings.language = i18n::DEFAULT_LANGUAGE.to_string();
        }
        let ui_scale = normalized_ui_scale(settings.ui_scale);
        settings.ui_scale = ui_scale;
        // The custom themes are JSON in the settings file: they are
        // normalized on the way in, so a hand-edited theme cannot paint a
        // color that GPUI cannot parse.
        let custom_themes: Vec<AppTheme> = settings
            .custom_themes
            .iter()
            .filter_map(whatfast_core::theme::normalize_theme)
            .collect();
        tokens::apply_theme(
            &tokens::theme_by_id(&settings.theme_id, &custom_themes),
            ui_scale,
            cx,
        );
        apply_theme_scale(ui_scale, window, cx);
        let density = settings.density;
        let enter_to_send = settings.enter_to_send;
        let split_view = settings.split_view;
        let battery_saver = settings.battery_saver;
        let theme_id = settings.theme_id.clone();
        let fake = std::env::args().any(|arg| arg == "--fake-backend")
            || std::env::var_os("WHATFAST_FAKE").is_some();
        let (rpc, startup_error) = match RpcClient::start(fake) {
            Ok(rpc) => (rpc, None),
            Err(error) => (RpcClient::disconnected(), Some(error.to_string())),
        };
        let receiver = rpc.incoming();
        let composer = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(i18n::t("Type a message"))
                // Enter is decided in the subscription: the input has no
                // setter for this and the setting changes at runtime.
                .submit_on_enter(false)
                .clean_on_escape()
        });
        let emoji_search =
            cx.new(|cx| InputState::new(window, cx).placeholder(i18n::t("Search emoji")));
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                // port: the placeholder of the origin
                // (`tauri/src/lib/i18n.ts`, `sidebar.searchPlaceholder`).
                .placeholder(i18n::t("Search or start a new chat"))
                .clean_on_escape()
        });
        let focus_handle = cx.focus_handle();
        // GPUI only routes key events along the focused path. Anchor focus to
        // the app root from the start so shortcuts work before (and after) any
        // text input holds focus.
        focus_handle.focus(window, cx);
        // The column of icons and the strip of tabs report what the user
        // pressed; the app answers them. The weak entity of the app is what
        // the handlers use to get back to the state.
        let shell_view = cx.entity();
        let rail_view = shell_view.clone();
        let nav_rail = cx.new(|_| {
            NavRail::new(ui_scale).on_action(move |action, window, cx| {
                rail_view.update(cx, |this, cx| {
                    this.on_nav_action(action, window, cx);
                    cx.notify();
                });
            })
        });
        let tab_view = shell_view.clone();
        let tab_strip = cx.new(|cx| {
            TabStrip::new(cx, "whatfast", ui_scale).on_action(move |action, window, cx| {
                // The action runs at the end of the effect cycle: every
                // gesture of the strip ends in `sync_shell`, which updates the
                // strip itself, and GPUI refuses to update an entity inside
                // its own event handler. Answering the click right here closed
                // the app with "cannot update TabStrip while it is already
                // being updated".
                tab_view.update(cx, |this, cx| {
                    cx.defer_in(window, move |this, window, cx| {
                        this.on_tab_action(action, window, cx);
                        cx.notify();
                    });
                });
            })
        });
        let attachment_view = shell_view.clone();
        let attachment_bar = cx.new(|_| {
            AttachmentBar::new(ui_scale).on_action(move |action, window, cx| {
                attachment_view.update(cx, |this, cx| {
                    this.on_attachment_action(action, window, cx);
                    cx.notify();
                });
            })
        });
        let _subscriptions = vec![
            cx.subscribe_in(&composer, window, {
                let composer = composer.clone();
                move |this, _, event, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        let text = composer.read(cx).value().to_string();
                        let composing = !text.trim().is_empty();
                        this.update_active_draft_text(text);
                        this.refresh_mention_picker(cx);
                        this.signal_typing(composing);
                        cx.notify();
                    }
                    if let InputEvent::PressEnter {
                        secondary, shift, ..
                    } = event
                    {
                        // Ctrl+Enter always sends. Otherwise the setting
                        // decides: with Enter-to-send on, a plain Enter
                        // sends; with it off, the Enter already inserted a
                        // newline and only Ctrl+Enter sends.
                        if *secondary || (this.enter_to_send && !*shift) {
                            // The input inserts the newline before it emits
                            // the event (`submit_on_enter(false)`), so the
                            // newline of the submit is taken out before the
                            // text goes to the conversation.
                            let value = this.composer.read(cx).value().to_string();
                            if let Some(trimmed) = value.strip_suffix('\n') {
                                let trimmed = trimmed.to_string();
                                this.composer
                                    .update(cx, |input, cx| input.set_value(trimmed, window, cx));
                            }
                            // The pictures of the tray win over the text: the
                            // return key sends them with the text as their
                            // caption, the same way the button of the send
                            // does.
                            this.submit_composer(window, cx);
                        }
                    }
                }
            }),
            cx.subscribe_in(&emoji_search, window, {
                let emoji_search = emoji_search.clone();
                move |this, _, event, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.emoji_query = emoji_search.read(cx).value().to_string();
                        this.rebuild_emoji_results();
                        cx.notify();
                    }
                }
            }),
            cx.subscribe_in(&search_input, window, {
                let search_input = search_input.clone();
                move |this, _, event, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        let query = search_input.read(cx).value().to_string();
                        this.set_search_query(query, cx);
                    }
                }
            }),
        ];
        let view = cx.entity();
        cx.spawn_in(window, async move |_, window| {
            while let Ok(message) = receiver.recv().await {
                if window
                    .update(|window, cx| {
                        view.update(cx, |this, cx| {
                            this.handle_bridge(message, window, cx);
                            cx.notify();
                        })
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let timeout_view = cx.entity();
        cx.spawn_in(window, async move |_, window| {
            loop {
                Timer::after(TIMEOUT_SWEEP_INTERVAL).await;
                if window
                    .update(|window, cx| {
                        timeout_view.update(cx, |this, cx| {
                            this.expire_stalled_requests(window, cx);
                            cx.notify();
                        })
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let mut this = Self {
            rpc,
            store: Store::default(),
            composer,
            pending_images: PendingImages::default(),
            search_input,
            search_query: String::new(),
            search_results: SearchResults::default(),
            search_generation: 0,
            search_highlighted: 0,
            search_error: None,
            search_scroll: VirtualListScrollHandle::new(),
            search_smooth_scroll: SmoothScrollState::default(),
            search_target_message_id: None,
            pending_search_scroll_id: None,
            pending_top_scroll_id: None,
            emoji_search,
            emoji_query: String::new(),
            emoji_category: EmojiCategory::All,
            emoji_results: Rc::new(filter_emojis(EmojiCategory::All, "")),
            emoji_target: None,
            reaction_details: None,
            image_viewer: None,
            replying_to_message_id: None,
            chat_drafts: HashMap::new(),
            recent_chat_ids: Vec::new(),
            chat_switcher: None,
            chat_info: None,
            chat_context_menu: None,
            chat_menu: None,
            mention_picker: None,
            mention_directories: HashMap::new(),
            mention_directory_pending: HashSet::new(),
            typing_indicators: HashMap::new(),
            typing_signal: None,
            focus_handle,
            settings_open: false,
            ui_scale,
            settings,
            density,
            enter_to_send,
            split_view,
            battery_saver,
            theme_id,
            custom_themes,
            editing_theme: None,
            notifier: Notifier::new(),
            chat_scroll: UniformListScrollHandle::new(),
            chat_smooth_scroll: SmoothScrollState::default(),
            message_scroll: VirtualListScrollHandle::new(),
            message_smooth_scroll: SmoothScrollState::default(),
            chat_info_scroll: ScrollHandle::new(),
            chat_info_smooth_scroll: SmoothScrollState::default(),
            scroll_to_bottom_generation: None,
            last_event_sequence: 0,
            pending_prepend_anchor: None,
            newer_load_failed: false,
            message_generation: 0,
            sync_complete_reloaded: false,
            confirming_logout: false,
            avatar_attempted: HashSet::new(),
            avatar_retries: HashMap::new(),
            participant_avatar_attempted: HashSet::new(),
            image_download_attempted: HashSet::new(),
            image_failures: HashMap::new(),
            sticker_preparing: false,
            reaction_repair_attempted: HashSet::new(),
            latest_reaction_intents: HashMap::new(),
            poll_vote_generations: HashMap::new(),
            pending_poll_votes: HashMap::new(),
            pending_message_deletes: HashMap::new(),
            pin_generations: HashMap::new(),
            attachment_download_attempted: HashSet::new(),
            attachment_failures: HashMap::new(),
            attachment_open_requested: None,
            attachment_save_requested: None,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            sidebar_hidden: false,
            sidebar_dragging: false,
            voice: VoiceState::default(),
            voice_audio: VoiceAudio::new(),
            voice_tick: false,
            recorder: VoiceRecorder::new(),
            recording: None,
            recording_tick: false,
            integrity_checking: false,
            sticker_library_loading: false,
            newsletters_loading: false,
            selected_newsletter: None,
            states_open: false,
            channels_open: false,
            archived_open: false,
            last_status_refresh: Instant::now(),
            nav_rail,
            tab_strip,
            media_editor: None,
            newsletter_panel: NewsletterPanel::new(window, cx),
            status_strip: StatusStrip::new(window, cx),
            member_panel: MemberPanel::default(),
            sticker_tray: StickerTray::default(),
            sticker_tray_open: false,
            panel_tab: PanelTab::Emoji,
            message_menu: None,
            forward_picker: None,
            poll_composer: None,
            attachment_bar,
            _subscriptions,
        };
        if let Some(error) = startup_error {
            this.store.screen = Screen::Fatal;
            this.store.fatal_error = Some(format!("Could not start the local backend: {error}"));
            return this;
        }
        // The favorites belong to the profile, not to the account: they come
        // from the file of the profile and not from the sidecar.
        this.store
            .set_favorite_chat_ids(favorites::load_favorites());
        // The stars belong to the profile too: the linked device has no RPC
        // for the star of WhatsApp.
        this.store.set_starred_messages(starred::load_starred());
        this.request(
            rpc_request::Request::Hello(whatfast_protocol::HelloRequest {
                desktop_version: env!("CARGO_PKG_VERSION").into(),
                minimum_protocol_version: PROTOCOL_VERSION,
                maximum_protocol_version: PROTOCOL_VERSION,
            }),
            PendingRequest::Hello,
        );
        this
    }

    pub(crate) fn smooth_scroll_state_mut(
        &mut self,
        surface: ScrollSurface,
    ) -> &mut SmoothScrollState {
        match surface {
            ScrollSurface::ChatList => &mut self.chat_smooth_scroll,
            ScrollSurface::Search => &mut self.search_smooth_scroll,
            ScrollSurface::Messages => &mut self.message_smooth_scroll,
            ScrollSurface::ChatInfo => &mut self.chat_info_smooth_scroll,
        }
    }

    pub(crate) fn smooth_scroll_handle(&self, surface: ScrollSurface) -> ScrollHandle {
        match surface {
            ScrollSurface::ChatList => self.chat_scroll.0.borrow().base_handle.clone(),
            ScrollSurface::Search => self.search_scroll.base_handle().clone(),
            ScrollSurface::Messages => self.message_scroll.base_handle().clone(),
            ScrollSurface::ChatInfo => self.chat_info_scroll.clone(),
        }
    }

    pub(crate) fn stop_smooth_scroll(&mut self, surface: ScrollSurface) {
        self.smooth_scroll_state_mut(surface).finish();
    }

    pub(crate) fn handle_smooth_scroll_input(
        &mut self,
        surface: ScrollSurface,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ScrollDelta::Lines(delta) = event.delta else {
            // A touchpad gesture should take over immediately from any
            // mouse-wheel animation still in flight.
            self.stop_smooth_scroll(surface);
            return;
        };
        if self.battery_saver {
            // Without the inertia the wheel moves the list in one step: no
            // frame every 16 ms while the fingers are not on the wheel.
            let handle = self.smooth_scroll_handle(surface);
            let offset = handle.offset();
            let maximum = handle.max_offset().y;
            let distance = delta.y * 20.0 * self.ui_scale;
            let target_y = (offset.y + px(distance)).clamp(-maximum, px(0.));
            handle.set_offset(point(offset.x, target_y));
            cx.notify();
            return;
        }
        let Some(generation) = self
            .smooth_scroll_state_mut(surface)
            .push_wheel(delta.y, Instant::now())
        else {
            return;
        };

        let view = cx.entity();
        cx.spawn_in(window, async move |_, window| {
            let mut last_frame = Instant::now();
            loop {
                Timer::after(SMOOTH_SCROLL_FRAME).await;
                let now = Instant::now();
                let elapsed = now
                    .duration_since(last_frame)
                    .as_secs_f32()
                    .clamp(0.001, 0.05);
                last_frame = now;
                let keep_running = window
                    .update(|_, cx| {
                        view.update(cx, |this, cx| {
                            let Some(distance) = this
                                .smooth_scroll_state_mut(surface)
                                .advance(generation, elapsed)
                            else {
                                return false;
                            };
                            let handle = this.smooth_scroll_handle(surface);
                            let offset = handle.offset();
                            let maximum = handle.max_offset().y;
                            let target_y = (offset.y + px(distance)).clamp(-maximum, px(0.));
                            if maximum <= px(0.) || target_y == offset.y {
                                this.stop_smooth_scroll(surface);
                                return false;
                            }
                            handle.set_offset(point(offset.x, target_y));
                            cx.notify();
                            true
                        })
                    })
                    .unwrap_or(false);
                if !keep_running {
                    break;
                }
            }
        })
        .detach();
    }

    /// Sends a request to the backend, or shows the fatal screen when the
    /// request could not even be queued.
    pub(crate) fn request(&mut self, request: rpc_request::Request, pending: PendingRequest) {
        let sent = self.rpc.send(request, pending);
        report_failed_request(&mut self.store, sent);
    }

    /// Sweeps the requests that outlived their deadline and undoes what they
    /// had already changed locally.
    pub(crate) fn expire_stalled_requests(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Piggyback on the 1s sweep: typing indicators expire locally because
        // the peer's terminal "paused" update can be lost.
        let now = Instant::now();
        self.typing_indicators.retain(|_, indicators| {
            indicators.retain(|_, indicator| indicator.expires_at > now);
            !indicators.is_empty()
        });
        // port: the statuses arrive without a push event, so the client polls
        // the store every forty five seconds to keep them fresh.
        if self.store.connection == ConnectionState::Connected
            && self.last_status_refresh.elapsed() >= STATUS_REFRESH
        {
            self.last_status_refresh = Instant::now();
            self.load_statuses();
        }
        for (_, pending) in self.rpc.expire() {
            match pending {
                PendingRequest::Hello | PendingRequest::Auth => {
                    self.store.screen = Screen::Fatal;
                    self.store.fatal_error = Some(
                        "The local backend stopped responding during startup. Restart Whatfast to reconnect."
                            .into(),
                    );
                }
                PendingRequest::Avatar { chat_id } => {
                    self.avatar_attempted.remove(&chat_id);
                }
                PendingRequest::ParticipantAvatar { participant_id } => {
                    self.participant_avatar_attempted.remove(&participant_id);
                }
                PendingRequest::ChatInfo { chat_id } => {
                    if let Some(state) = self.chat_info.as_mut()
                        && state.chat_id == chat_id
                    {
                        state.loading = false;
                        state.error = Some("Loading chat info timed out. Try again.".into());
                    }
                }
                PendingRequest::MentionDirectory { chat_id } => {
                    self.mention_directory_pending.remove(&chat_id);
                }
                PendingRequest::MessageImage {
                    chat_id,
                    message_id,
                } => {
                    let sticker = self
                        .store
                        .message(&message_id)
                        .and_then(|message| message.content.as_ref())
                        .is_some_and(|content| {
                            matches!(content, whatfast_protocol::message::Content::Image(image) if image.sticker)
                        });
                    self.image_failures.insert(
                        (chat_id, message_id),
                        if sticker {
                            "Sticker download timed out · Click to retry".into()
                        } else {
                            "Photo download timed out · Click to retry".into()
                        },
                    );
                }
                PendingRequest::Search { generation, .. }
                    if generation == self.search_generation =>
                {
                    self.search_error = Some("Search timed out. Try again.".into());
                }
                PendingRequest::SendText {
                    chat_id,
                    draft_text,
                    reply_to_message_id,
                    mentions,
                } => {
                    self.restore_failed_draft(
                        &chat_id,
                        &draft_text,
                        reply_to_message_id,
                        &mentions,
                        window,
                        cx,
                    );
                    self.store.toast_error =
                        Some("Sending timed out. Your draft was restored; try again.".into());
                }
                PendingRequest::SendImage {
                    chat_id,
                    draft_text,
                    reply_to_message_id,
                } => {
                    self.restore_failed_draft(
                        &chat_id,
                        &draft_text,
                        reply_to_message_id,
                        &[],
                        window,
                        cx,
                    );
                    self.store.toast_error =
                        Some("Sending timed out. Your draft was restored; try again.".into());
                }
                PendingRequest::SendReaction {
                    chat_id,
                    message_id,
                    client_reaction_id,
                    ..
                } => {
                    self.clear_reaction_intent_if_current(
                        &chat_id,
                        &message_id,
                        &client_reaction_id,
                    );
                    self.store.toast_error = Some(i18n::t("Reaction timed out. Try again.").into());
                }
                PendingRequest::MarkRead {
                    chat_id,
                    previous_unread,
                } => {
                    self.store.restore_unread(&chat_id, previous_unread);
                    self.store.last_mark_read_id = None;
                    self.store.toast_error =
                        Some(i18n::t("Mark as read timed out. Try again.").into());
                }
                PendingRequest::Messages { prepend: true, .. } => {
                    self.pending_prepend_anchor = None;
                    self.store.toast_error =
                        Some(i18n::t("Loading messages timed out. Try again.").into());
                }
                PendingRequest::MessagesAfter { .. } => {
                    self.newer_load_failed = true;
                    self.store.toast_error =
                        Some("Loading newer messages timed out. Try again.".into());
                }
                PendingRequest::Logout => {
                    self.confirming_logout = false;
                    self.store.toast_error = Some(i18n::t("Logout timed out. Try again.").into());
                }
                PendingRequest::Search { .. } => {}
                // Best-effort presence broadcast; peers expire it on their own.
                PendingRequest::SetTyping => {}
                PendingRequest::SetChatPin {
                    chat_id,
                    previous_pinned,
                } => {
                    self.store.set_chat_pinned(&chat_id, previous_pinned);
                    self.store.toast_error =
                        Some(i18n::t("Pinning the chat timed out. Try again.").into());
                }
                PendingRequest::SetChatArchive {
                    chat_id,
                    previous_archived,
                } => {
                    self.store.set_chat_archived(&chat_id, previous_archived);
                    self.store.toast_error =
                        Some("Archiving the chat timed out. Try again.".into());
                }
                PendingRequest::SetChatMute {
                    chat_id,
                    previous_muted,
                    previous_mute_until_ms,
                } => {
                    self.store
                        .set_chat_muted(&chat_id, previous_muted, previous_mute_until_ms);
                    self.store.toast_error =
                        Some(i18n::t("Muting the chat timed out. Try again.").into());
                }
                PendingRequest::SetChatBlock {
                    chat_id,
                    previous_blocked,
                } => {
                    self.store.set_chat_blocked(&chat_id, previous_blocked);
                    self.store.toast_error =
                        Some(i18n::t("Blocking the chat timed out. Try again.").into());
                }
                PendingRequest::DeleteMessage {
                    chat_id,
                    message_id,
                } => {
                    if let Some(previous) = self
                        .pending_message_deletes
                        .remove(&(chat_id.clone(), message_id.clone()))
                    {
                        self.store.restore_message(previous);
                    }
                    self.store.toast_error =
                        Some("Deleting the message timed out. Try again.".into());
                }
                PendingRequest::VotePoll {
                    chat_id,
                    poll_message_id,
                    generation,
                } => {
                    self.rollback_poll_vote(&chat_id, &poll_message_id, generation);
                    self.store.toast_error = Some(i18n::t("The vote timed out. Try again.").into());
                }
                PendingRequest::EditMessage {
                    chat_id,
                    message_id,
                    draft_text,
                } => {
                    self.restore_failed_edit(&chat_id, &message_id, &draft_text, window, cx);
                    self.store.toast_error = Some(
                        "Editing the message timed out. Your draft was restored; try again.".into(),
                    );
                }
                PendingRequest::OlderHistory { chat_id } => {
                    report_failed_request(
                        &mut self.store,
                        self.rpc.request_history_coverage(chat_id.clone()),
                    );
                    self.store.toast_error =
                        Some("Asking for older history timed out. Try again.".into());
                }
                PendingRequest::Reconnect => {
                    self.store.connection = ConnectionState::Offline;
                    self.store.connection_detail = "Reconnect failed".into();
                    self.store.toast_error =
                        Some(i18n::t("Reconnect timed out. Try again.").into());
                }
                PendingRequest::IntegrityStatus | PendingRequest::RepairLocalCache => {
                    self.integrity_checking = false;
                    self.store.toast_error =
                        Some("Checking the local storage timed out. Try again.".into());
                }
                PendingRequest::Stickers => {
                    self.sticker_library_loading = false;
                    self.store.toast_error =
                        Some("Loading the stickers timed out. Try again.".into());
                }
                PendingRequest::Newsletters => {
                    self.newsletters_loading = false;
                    self.store.toast_error =
                        Some("Loading the newsletters timed out. Try again.".into());
                }
                PendingRequest::MessageAttachment {
                    chat_id,
                    message_id,
                } => {
                    let key = (chat_id.clone(), message_id.clone());
                    self.attachment_open_requested = None;
                    // The arrow of the download waits for this file: a
                    // download that never arrived must not save a later one.
                    forget_attachment_request(&mut self.attachment_save_requested, &key);
                    self.attachment_failures.insert(
                        key,
                        "Downloading the file timed out · Click to retry".into(),
                    );
                }
                _ => {
                    self.store.toast_error =
                        Some(i18n::t("Backend request timed out. Try again.").into());
                }
            }
        }
    }

    pub(crate) fn set_search_query(&mut self, query: String, cx: &mut Context<Self>) {
        self.search_query = query;
        self.search_generation = self.search_generation.wrapping_add(1);
        self.search_results = SearchResults::default();
        self.search_highlighted = 0;
        self.search_error = None;
        self.stop_smooth_scroll(ScrollSurface::Search);
        self.search_scroll.scroll_to_item(0, ScrollStrategy::Top);
        let trimmed = self.search_query.trim().to_owned();
        let generation = self.search_generation;
        if trimmed.chars().count() < 2 {
            cx.notify();
            return;
        }
        cx.spawn(async move |this, cx| {
            Timer::after(Duration::from_millis(150)).await;
            let _ = this.update(cx, |this, cx| {
                if this.search_generation != generation || this.search_query.trim() != trimmed {
                    return;
                }
                this.request(
                    rpc_request::Request::SearchLocal(whatfast_protocol::SearchLocalRequest {
                        query: trimmed.clone(),
                    }),
                    PendingRequest::Search {
                        query: trimmed,
                        generation,
                    },
                );
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.store.screen != Screen::Chats {
            return;
        }
        self.settings_open = false;
        self.chat_switcher = None;
        self.reaction_details = None;
        self.image_viewer = None;
        self.chat_info = None;
        self.search_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    pub(crate) fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_generation = self.search_generation.wrapping_add(1);
        self.search_query.clear();
        self.search_results = SearchResults::default();
        self.search_highlighted = 0;
        self.search_error = None;
        self.stop_smooth_scroll(ScrollSurface::Search);
        self.search_scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    pub(crate) fn move_search_selection(&mut self, reverse: bool) {
        let count = self.search_results.len();
        if count == 0 {
            self.search_highlighted = 0;
        } else if reverse {
            self.search_highlighted = self.search_highlighted.checked_sub(1).unwrap_or(count - 1);
        } else {
            self.search_highlighted = (self.search_highlighted + 1) % count;
        }
        if let Some(row_index) = self
            .search_results
            .row_index_for_result(self.search_highlighted)
        {
            self.stop_smooth_scroll(ScrollSurface::Search);
            self.search_scroll
                .scroll_to_item(row_index, ScrollStrategy::Center);
        }
    }

    pub(crate) fn activate_search_result(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.search_results.target(self.search_highlighted) else {
            return;
        };
        match target {
            SearchTarget::Contact(contact_jid) => {
                self.request(
                    rpc_request::Request::OpenContact(whatfast_protocol::OpenContactRequest {
                        contact_jid: contact_jid.clone(),
                    }),
                    PendingRequest::OpenContact,
                );
            }
            SearchTarget::Group(chat) => {
                let chat_id = chat.id.clone();
                self.store.upsert_chat(chat);
                self.clear_search(window, cx);
                self.open_chat(chat_id, window, cx);
            }
            SearchTarget::Message {
                chat,
                chat_id,
                message_id,
            } => {
                if let Some(chat) = chat {
                    self.store.upsert_chat(chat);
                }
                self.clear_search(window, cx);
                self.message_generation = self.message_generation.wrapping_add(1);
                self.store.select_chat(chat_id.clone());
                self.show_chat_in_the_column(&chat_id);
                record_recent_chat(&mut self.recent_chat_ids, &chat_id);
                let draft = self.chat_drafts.get(&chat_id).cloned().unwrap_or_default();
                self.replying_to_message_id = draft.reply_to_message_id;
                self.composer.update(cx, |input, cx| {
                    input.set_value(draft.text, window, cx);
                    input.focus(window, cx);
                });
                self.emoji_target = None;
                self.reaction_details = None;
                self.image_viewer = None;
                self.scroll_to_bottom_generation = None;
                self.pending_top_scroll_id = None;
                self.newer_load_failed = false;
                self.image_download_attempted.clear();
                self.image_failures.clear();
                self.search_target_message_id = Some(message_id.clone());
                self.request(
                    rpc_request::Request::ListMessagesAround(
                        whatfast_protocol::ListMessagesAroundRequest {
                            chat_id: chat_id.clone(),
                            message_id: message_id.clone(),
                        },
                    ),
                    PendingRequest::MessagesAround {
                        chat_id,
                        message_id,
                        generation: self.message_generation,
                    },
                );
            }
        }
        cx.notify();
    }

    pub(crate) fn handle_bridge(
        &mut self,
        message: BridgeMessage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.rpc.handle_incoming(message) {
            RpcIncoming::Response { pending, response } => {
                self.handle_response(pending, response, window, cx)
            }
            RpcIncoming::Event(event) => self.handle_event(event, window),
            RpcIncoming::Exited(error) => {
                self.store.connection = ConnectionState::Failed;
                self.store.connection_detail = error;
                self.store.toast_error =
                    Some("Backend stopped. Restart Whatfast to reconnect.".into());
            }
            RpcIncoming::Invalid => {
                self.store.toast_error = Some(i18n::t("Ignored an invalid backend envelope").into())
            }
            RpcIncoming::Ignore => {}
        }
        // La columna de iconos se habilita cuando el dispositivo queda
        // emparejado: sin esto se queda en `Pairing` para siempre y los
        // botones de los estados, los canales y los archivados no responden.
        self.sync_shell(cx);
    }

    pub(crate) fn handle_response(
        &mut self,
        pending: PendingRequest,
        response: RpcResponse,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(result) = response.result else {
            self.store.toast_error = Some(i18n::t("Backend returned an empty response").into());
            return;
        };
        if let rpc_response::Result::Error(error) = result {
            if let PendingRequest::Search { generation, .. } = &pending {
                if *generation == self.search_generation {
                    self.search_error = Some(error.message);
                }
                return;
            }
            if let PendingRequest::MessagesAround {
                chat_id,
                generation,
                ..
            } = &pending
                && *generation == self.message_generation
            {
                self.search_target_message_id = None;
                self.pending_search_scroll_id = None;
                self.load_selected_chat(chat_id.clone());
                self.store.toast_error = Some(format!("{}: {}", error.code, error.message));
                return;
            }
            if matches!(&pending, PendingRequest::MessagesAfter { .. }) {
                self.newer_load_failed = true;
            }
            match &pending {
                PendingRequest::SendText {
                    chat_id,
                    draft_text,
                    reply_to_message_id,
                    mentions,
                } => self.restore_failed_draft(
                    chat_id,
                    draft_text,
                    reply_to_message_id.clone(),
                    mentions,
                    window,
                    cx,
                ),
                PendingRequest::SendImage {
                    chat_id,
                    draft_text,
                    reply_to_message_id,
                } => self.restore_failed_draft(
                    chat_id,
                    draft_text,
                    reply_to_message_id.clone(),
                    &[],
                    window,
                    cx,
                ),
                _ => {}
            }
            if matches!(&pending, PendingRequest::RepairRecentReactions { .. })
                && error.code == "not_found"
            {
                return;
            }
            if let PendingRequest::RepairRecentReactions { chat_id } = &pending
                && error.retryable
            {
                self.schedule_reaction_repair_retry(chat_id.clone(), cx);
                return;
            }
            if let PendingRequest::SendReaction {
                chat_id,
                message_id,
                emoji,
                client_reaction_id,
                attempt,
            } = &pending
                && let Some(retry) = reaction_retry(error.retryable, *attempt)
            {
                let chat_id = chat_id.clone();
                let message_id = message_id.clone();
                let emoji = emoji.clone();
                let client_reaction_id = client_reaction_id.clone();
                cx.spawn(async move |this, cx| {
                    Timer::after(retry.delay).await;
                    let _ = this.update(cx, |this, _| {
                        if !this.reaction_intent_is_current(
                            &chat_id,
                            &message_id,
                            &client_reaction_id,
                        ) {
                            return;
                        }
                        this.send_reaction_request(
                            chat_id,
                            message_id,
                            emoji,
                            client_reaction_id,
                            retry.attempt,
                        );
                    });
                })
                .detach();
                return;
            }
            if let PendingRequest::SendReaction {
                chat_id,
                message_id,
                client_reaction_id,
                ..
            } = &pending
            {
                self.clear_reaction_intent_if_current(chat_id, message_id, client_reaction_id);
            }
            if matches!(&pending, PendingRequest::SetTyping) {
                return;
            }
            if let PendingRequest::ChatInfo { chat_id } = &pending {
                if let Some(state) = self.chat_info.as_mut()
                    && state.chat_id == *chat_id
                {
                    state.loading = false;
                    state.error = Some(error.message);
                }
                return;
            }
            if let PendingRequest::MentionDirectory { chat_id } = &pending {
                self.mention_directory_pending.remove(chat_id);
                return;
            }
            if let PendingRequest::Avatar { chat_id } = &pending {
                let retries = self
                    .avatar_retries
                    .get(chat_id)
                    .copied()
                    .unwrap_or_default();
                if let Some(retry) = avatar_retry(error.retryable, retries) {
                    self.avatar_retries.insert(chat_id.clone(), retry.attempt);
                    let retry_chat_id = chat_id.clone();
                    cx.spawn(async move |this, cx| {
                        Timer::after(retry.delay).await;
                        let _ = this.update(cx, |this, cx| {
                            this.avatar_attempted.remove(&retry_chat_id);
                            cx.notify();
                        });
                    })
                    .detach();
                }
                return;
            }
            if let PendingRequest::ParticipantAvatar { participant_id } = &pending {
                let retry_key = format!("participant:{participant_id}");
                let retries = self
                    .avatar_retries
                    .get(&retry_key)
                    .copied()
                    .unwrap_or_default();
                if let Some(retry) = avatar_retry(error.retryable, retries) {
                    self.avatar_retries.insert(retry_key, retry.attempt);
                    let retry_participant_id = participant_id.clone();
                    cx.spawn(async move |this, cx| {
                        Timer::after(retry.delay).await;
                        let _ = this.update(cx, |this, cx| {
                            this.participant_avatar_attempted
                                .remove(&retry_participant_id);
                            cx.notify();
                        });
                    })
                    .detach();
                }
                return;
            }
            if let PendingRequest::MessageImage {
                chat_id,
                message_id,
            } = &pending
            {
                // Keep the attempt latched so virtualization does not create a
                // retry storm. The row now exposes an explicit retry action.
                let sticker = self
                    .store
                    .message(message_id)
                    .and_then(|message| message.content.as_ref())
                    .is_some_and(|content| {
                        matches!(content, whatfast_protocol::message::Content::Image(image) if image.sticker)
                    });
                self.image_failures.insert(
                    (chat_id.clone(), message_id.clone()),
                    if sticker && error.retryable {
                        "Couldn't load sticker · Click to retry".into()
                    } else if sticker {
                        "Sticker is no longer available".into()
                    } else if error.retryable {
                        "Couldn't load photo · Click to retry".into()
                    } else {
                        "Photo is no longer available".into()
                    },
                );
                return;
            }
            if let PendingRequest::MarkRead {
                chat_id,
                previous_unread,
            } = &pending
            {
                self.store.restore_unread(chat_id, *previous_unread);
                self.store.last_mark_read_id = None;
            }
            // The changes below are optimistic: the client shows them before
            // the backend answers, so a failure has to put them back.
            if let PendingRequest::SetChatPin {
                chat_id,
                previous_pinned,
            } = &pending
            {
                self.store.set_chat_pinned(chat_id, *previous_pinned);
                self.store.toast_error = Some(format!("{}: {}", error.code, error.message));
                return;
            }
            if let PendingRequest::SetChatArchive {
                chat_id,
                previous_archived,
            } = &pending
            {
                self.store.set_chat_archived(chat_id, *previous_archived);
                self.store.toast_error = Some(format!("{}: {}", error.code, error.message));
                return;
            }
            if let PendingRequest::SetChatMute {
                chat_id,
                previous_muted,
                previous_mute_until_ms,
            } = &pending
            {
                self.store
                    .set_chat_muted(chat_id, *previous_muted, *previous_mute_until_ms);
                self.store.toast_error = Some(format!("{}: {}", error.code, error.message));
                return;
            }
            if let PendingRequest::SetChatBlock {
                chat_id,
                previous_blocked,
            } = &pending
            {
                self.store.set_chat_blocked(chat_id, *previous_blocked);
                self.store.toast_error = Some(format!("{}: {}", error.code, error.message));
                return;
            }
            if let PendingRequest::DeleteMessage {
                chat_id,
                message_id,
            } = &pending
            {
                if let Some(previous) = self
                    .pending_message_deletes
                    .remove(&(chat_id.clone(), message_id.clone()))
                {
                    self.store.restore_message(previous);
                }
                self.store.toast_error = Some(format!("{}: {}", error.code, error.message));
                return;
            }
            if let PendingRequest::VotePoll {
                chat_id,
                poll_message_id,
                generation,
            } = &pending
            {
                self.rollback_poll_vote(chat_id, poll_message_id, *generation);
                self.store.toast_error = Some(format!("{}: {}", error.code, error.message));
                return;
            }
            if let PendingRequest::EditMessage {
                chat_id,
                message_id,
                draft_text,
            } = &pending
            {
                self.restore_failed_edit(chat_id, message_id, draft_text, window, cx);
                self.store.toast_error = Some(format!("{}: {}", error.code, error.message));
                return;
            }
            if let PendingRequest::OlderHistory { chat_id } = &pending {
                // port: the origin reads the coverage again when the request
                // of more history fails, because the failure moves it.
                report_failed_request(
                    &mut self.store,
                    self.rpc.request_history_coverage(chat_id.clone()),
                );
                self.store.toast_error = Some(error.message.clone());
                return;
            }
            if let PendingRequest::Reconnect = &pending {
                self.store.connection = ConnectionState::Offline;
                self.store.connection_detail = "Reconnect failed".into();
                self.store.toast_error = Some(format!("{}: {}", error.code, error.message));
                return;
            }
            if let PendingRequest::IntegrityStatus | PendingRequest::RepairLocalCache = &pending {
                self.integrity_checking = false;
            }
            if let PendingRequest::Stickers = &pending {
                self.sticker_library_loading = false;
            }
            if let PendingRequest::Newsletters = &pending {
                self.newsletters_loading = false;
            }
            if let PendingRequest::MessageAttachment {
                chat_id,
                message_id,
            } = &pending
            {
                let key = (chat_id.clone(), message_id.clone());
                self.attachment_open_requested = None;
                // Same as the timeout: the save that was waiting for this
                // file is forgotten, so a later download does not save it.
                forget_attachment_request(&mut self.attachment_save_requested, &key);
                self.attachment_failures.insert(
                    key,
                    if error.retryable {
                        "Couldn't download the file · Click to retry".into()
                    } else {
                        "That file is no longer available".into()
                    },
                );
                return;
            }
            if matches!(&pending, PendingRequest::Logout) {
                self.confirming_logout = false;
                self.store.screen = Screen::Fatal;
                self.store.fatal_error = Some(format!(
                    "Logout stopped because local account data could not be safely cleared ({}: {}). Whatfast will not re-pair until this is resolved.",
                    error.code, error.message
                ));
                return;
            }
            if matches!(&pending, PendingRequest::Messages { prepend: true, .. }) {
                self.pending_prepend_anchor = None;
            }
            let message = format!("{}: {}", error.code, error.message);
            if matches!(pending, PendingRequest::Hello) {
                self.store.fatal_error = Some(message);
                self.store.screen = Screen::Fatal;
            } else {
                self.store.toast_error = Some(message);
            }
            return;
        }
        match (pending, result) {
            (PendingRequest::Hello, rpc_response::Result::Hello(hello)) => {
                if hello.protocol_version != PROTOCOL_VERSION {
                    self.store.screen = Screen::Fatal;
                    self.store.fatal_error = Some(format!(
                        "Protocol mismatch: desktop v{PROTOCOL_VERSION}, backend v{}",
                        hello.protocol_version
                    ));
                } else {
                    self.request(
                        rpc_request::Request::GetAuthState(
                            whatfast_protocol::GetAuthStateRequest {},
                        ),
                        PendingRequest::Auth,
                    );
                }
            }
            (PendingRequest::Auth, rpc_response::Result::AuthState(auth)) => {
                self.store.connection = auth.connection_state();
                if auth.paired {
                    self.store.screen = Screen::Syncing;
                    self.load_chats(String::new());
                    // port: the origin reads the status of the sync with the
                    // state of the account, in the same round trip.
                    self.load_sync_status();
                } else {
                    self.begin_pairing();
                }
            }
            (PendingRequest::Chats { cursor }, rpc_response::Result::ListChats(page)) => {
                let newest_chat_id = if cursor.is_empty() {
                    page.chats.first().map(|chat| chat.id.clone())
                } else {
                    None
                };
                self.store.replace_chat_page(
                    &cursor,
                    page.chats,
                    page.total_count,
                    page.next_cursor,
                );
                self.store.screen = Screen::Chats;
                // Repair the newest chat proactively so legacy reactions are
                // restored even before the user clicks back into it after an
                // upgrade. Other affected chats remain lazy-on-open.
                if let Some(chat_id) = newest_chat_id {
                    self.repair_recent_reactions(chat_id);
                }
            }
            (
                PendingRequest::Search { query, generation },
                rpc_response::Result::SearchLocal(results),
            ) if generation == self.search_generation && self.search_query.trim() == query => {
                self.search_results = SearchResults {
                    contacts: results.contacts,
                    groups: results.groups,
                    messages: results.messages,
                };
                self.search_highlighted = 0;
                self.search_error = None;
                self.search_scroll.scroll_to_item(0, ScrollStrategy::Top);
            }
            (PendingRequest::OpenContact, rpc_response::Result::OpenContact(opened)) => {
                if let Some(chat) = opened.chat {
                    let chat_id = chat.id.clone();
                    self.store.upsert_chat(chat);
                    self.clear_search(window, cx);
                    self.open_chat(chat_id, window, cx);
                }
            }
            (
                PendingRequest::MessagesAround {
                    chat_id,
                    message_id,
                    generation,
                },
                rpc_response::Result::ListMessagesAround(page),
            ) if self.store.selected_chat_id.as_deref() == Some(chat_id.as_str())
                && generation == self.message_generation
                && page.anchor_message_id == message_id =>
            {
                self.store
                    .replace_message_window(page.messages, page.has_older, page.has_newer);
                self.pending_search_scroll_id = Some(message_id.clone());
                self.search_target_message_id = Some(message_id.clone());
                cx.spawn(async move |this, cx| {
                    Timer::after(Duration::from_secs(3)).await;
                    let _ = this.update(cx, |this, cx| {
                        if this.search_target_message_id.as_deref() == Some(message_id.as_str()) {
                            this.search_target_message_id = None;
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
            (
                PendingRequest::OpenMessageWindow {
                    chat_id,
                    generation,
                },
                rpc_response::Result::OpenMessageWindow(page),
            ) if self.store.selected_chat_id.as_deref() == Some(chat_id.as_str())
                && generation == self.message_generation =>
            {
                self.store
                    .replace_message_window(page.messages, page.has_older, page.has_newer);
                if page.first_unread_message_id.is_empty() {
                    self.scroll_to_bottom_generation = Some(generation);
                } else {
                    self.pending_top_scroll_id = Some(page.first_unread_message_id);
                }
            }
            (
                PendingRequest::MessagesAfter {
                    chat_id,
                    generation,
                },
                rpc_response::Result::ListMessagesAfter(page),
            ) if self.store.selected_chat_id.as_deref() == Some(chat_id.as_str())
                && generation == self.message_generation =>
            {
                self.store
                    .append_newer_messages(page.messages, page.has_more);
            }
            (PendingRequest::Avatar { chat_id }, rpc_response::Result::GetChatAvatar(avatar))
                if avatar.chat_id == chat_id =>
            {
                self.store.set_chat_avatar(&chat_id, avatar.avatar_path);
                self.avatar_retries.remove(&chat_id);
                // El avatar es una foto local: si el backend vuelve a mandar
                // el chat sin ella, la app la pide de nuevo (el origen hace
                // lo mismo cuando el camino del avatar viene vacio).
                self.avatar_attempted.remove(&chat_id);
            }
            (
                PendingRequest::ParticipantAvatar { participant_id },
                rpc_response::Result::GetParticipantAvatar(avatar),
            ) if avatar.participant_id == participant_id => {
                self.store
                    .set_participant_avatar(participant_id.clone(), avatar.avatar_path);
                self.avatar_retries
                    .remove(&format!("participant:{participant_id}"));
                cx.spawn(async move |this, cx| {
                    Timer::after(PARTICIPANT_AVATAR_REFRESH).await;
                    let _ = this.update(cx, |this, cx| {
                        this.participant_avatar_attempted.remove(&participant_id);
                        cx.notify();
                    });
                })
                .detach();
            }
            (
                PendingRequest::Messages {
                    chat_id,
                    prepend,
                    generation,
                },
                rpc_response::Result::ListMessages(page),
            ) if self.store.selected_chat_id.as_deref() == Some(chat_id.as_str())
                && generation == self.message_generation =>
            {
                self.store.has_older_messages = page.has_more;
                if prepend {
                    self.store.prepend_messages(page.messages);
                } else {
                    for message in page.messages {
                        self.store.upsert_message(message);
                    }
                    self.scroll_to_bottom_generation = Some(generation);
                }
            }
            (PendingRequest::SendText { .. }, rpc_response::Result::SendText(sent)) => {
                if let Some(message) = sent.message {
                    self.store.upsert_message(message);
                    self.scroll_to_bottom_generation = Some(self.message_generation);
                }
            }
            (PendingRequest::SendImage { .. }, rpc_response::Result::SendImage(sent)) => {
                if let Some(message) = sent.message {
                    self.store.upsert_message(message);
                    self.scroll_to_bottom_generation = Some(self.message_generation);
                }
            }
            (PendingRequest::SetTyping, rpc_response::Result::SetTyping(_)) => {}
            (PendingRequest::ChatInfo { chat_id }, rpc_response::Result::GetChatInfo(info)) => {
                if let Some(chat) = info.chat.clone() {
                    self.store.upsert_chat(chat);
                }
                self.mention_directories
                    .insert(chat_id.clone(), info.participants.clone());
                if let Some(state) = self.chat_info.as_mut()
                    && state.chat_id == chat_id
                {
                    state.loading = false;
                    state.error = None;
                    state.info = Some(info);
                }
            }
            (
                PendingRequest::MentionDirectory { chat_id },
                rpc_response::Result::GetChatInfo(info),
            ) => {
                self.mention_directory_pending.remove(&chat_id);
                if let Some(chat) = info.chat.clone() {
                    self.store.upsert_chat(chat);
                }
                self.mention_directories.insert(chat_id, info.participants);
            }
            (PendingRequest::SendSticker, rpc_response::Result::SendSticker(sent)) => {
                if let Some(message) = sent.message {
                    let sent_to_selected_chat =
                        self.store.selected_chat_id.as_deref() == Some(message.chat_id.as_str());
                    self.store.upsert_message(message);
                    if sent_to_selected_chat {
                        self.scroll_to_bottom_generation = Some(self.message_generation);
                    }
                }
            }
            (
                PendingRequest::MessageImage {
                    chat_id,
                    message_id,
                },
                rpc_response::Result::GetMessageImage(image),
            ) if image.chat_id == chat_id && image.message_id == message_id => {
                let key = (chat_id.clone(), message_id.clone());
                if image.image_path.is_empty() {
                    let sticker = self
                        .store
                        .message(&message_id)
                        .and_then(|message| message.content.as_ref())
                        .is_some_and(|content| {
                            matches!(content, whatfast_protocol::message::Content::Image(image) if image.sticker)
                        });
                    let label = if sticker {
                        "Sticker is no longer available"
                    } else {
                        "Photo is no longer available"
                    };
                    self.image_failures.insert(key, label.into());
                } else {
                    self.image_failures.remove(&key);
                    self.store.set_message_image_paths(
                        &chat_id,
                        &message_id,
                        image.image_path,
                        image.thumbnail_path,
                    );
                }
            }
            (
                PendingRequest::SendReaction {
                    chat_id,
                    message_id,
                    client_reaction_id,
                    ..
                },
                rpc_response::Result::SendReaction(sent),
            ) => {
                if let Some(reaction) = sent.reaction
                    && reaction.chat_id == chat_id
                    && reaction.message_id == message_id
                {
                    self.store.apply_reaction(reaction, sent.removed);
                    self.refresh_reaction_details();
                }
                self.clear_reaction_intent_if_current(&chat_id, &message_id, &client_reaction_id);
            }
            (
                PendingRequest::RepairRecentReactions { chat_id },
                rpc_response::Result::RepairRecentReactions(repair),
            ) if repair.chat_id == chat_id => {
                // A peer history request can be accepted without a response
                // ever arriving (for example if the primary goes offline).
                // Keep a bounded fallback retry armed in both the requested
                // and rate-limited cases; completed jobs become a silent
                // not_found on the harmless later probe.
                self.schedule_reaction_repair_retry(chat_id, cx);
            }
            (PendingRequest::Logout, rpc_response::Result::Logout(_)) => {
                self.begin_pairing();
            }
            (PendingRequest::SyncStatus, rpc_response::Result::SyncStatus(status)) => {
                if let Some(status) = status.status {
                    self.store.apply_sync_status(status);
                }
            }
            (
                PendingRequest::OlderHistory { .. },
                rpc_response::Result::RequestOlderHistory(response),
            ) => {
                if let Some(coverage) = response.coverage {
                    self.store.set_history_coverage(coverage);
                }
            }
            (
                PendingRequest::HistoryCoverage { .. },
                rpc_response::Result::GetChatHistoryCoverage(response),
            ) => {
                if let Some(coverage) = response.coverage {
                    self.store.set_history_coverage(coverage);
                }
            }
            (
                PendingRequest::HistoryOverview,
                rpc_response::Result::GetHistoryOverview(response),
            ) => {
                if let Some(overview) = response.overview {
                    self.store.set_history_overview(overview);
                }
            }
            (
                PendingRequest::IntegrityStatus,
                rpc_response::Result::GetIntegrityStatus(response),
            ) => {
                self.integrity_checking = false;
                if let Some(status) = response.status {
                    self.store.set_integrity_status(status);
                }
            }
            (
                PendingRequest::RepairLocalCache,
                rpc_response::Result::RepairLocalCache(response),
            ) => {
                self.integrity_checking = false;
                if let Some(status) = response.status {
                    self.store.set_integrity_status(status);
                }
                // port: the origin reloads the chats and the open
                // conversations after the repair, because the cache changed.
                self.load_chats(String::new());
                if self.store.selected_chat_id.is_some() {
                    self.refresh_latest();
                }
            }
            (PendingRequest::Reconnect, rpc_response::Result::Reconnect(reconnect)) => {
                self.store.connection_detail = if reconnect.started {
                    "Connecting on your request".into()
                } else {
                    "Reconnect is already running or this device is not linked".into()
                };
            }
            (PendingRequest::SendAttachment { .. }, rpc_response::Result::SendAttachment(sent)) => {
                if let Some(message) = sent.message {
                    let sent_to_selected_chat =
                        self.store.selected_chat_id.as_deref() == Some(message.chat_id.as_str());
                    self.store.upsert_message(message);
                    if sent_to_selected_chat {
                        self.scroll_to_bottom_generation = Some(self.message_generation);
                    }
                }
            }
            (
                PendingRequest::MessageAttachment {
                    chat_id,
                    message_id,
                },
                rpc_response::Result::GetMessageAttachment(response),
            ) if response.chat_id == chat_id && response.message_id == message_id => {
                let key = (chat_id.clone(), message_id.clone());
                if response.local_path.is_empty() {
                    self.attachment_open_requested = None;
                    self.attachment_save_requested = None;
                    self.attachment_failures
                        .insert(key, "That file is no longer available".into());
                } else {
                    self.attachment_failures.remove(&key);
                    self.store.set_message_attachment_path(
                        &chat_id,
                        &message_id,
                        response.local_path.clone(),
                    );
                    // The user asked to open the file before it was in the
                    // disk: a voice note plays here, and any other file goes
                    // to the program of the system now.
                    if self.attachment_open_requested.as_ref() == Some(&key) {
                        self.attachment_open_requested = None;
                        let voice = self
                            .store
                            .message(&message_id)
                            .and_then(|message| match message.content.as_ref() {
                                Some(whatfast_protocol::message::Content::Attachment(
                                    attachment,
                                )) => Some(attachment.voice_note),
                                _ => None,
                            })
                            .unwrap_or(false);
                        if voice {
                            self.toggle_voice_note(
                                &chat_id,
                                &message_id,
                                &response.local_path,
                                window,
                                cx,
                            );
                        } else {
                            self.open_file_with_the_system(&response.local_path, cx);
                        }
                    }
                    // The user pressed the arrow of the download before the
                    // file was here: it goes to the folder now, without
                    // opening it.
                    if self.attachment_save_requested.as_ref() == Some(&key) {
                        self.attachment_save_requested = None;
                        let name =
                            self.store.message(&message_id).and_then(|message| {
                                match message.content.as_ref() {
                                    Some(whatfast_protocol::message::Content::Attachment(
                                        attachment,
                                    )) => (!attachment.file_name.trim().is_empty())
                                        .then(|| attachment.file_name.clone()),
                                    _ => None,
                                }
                            });
                        self.save_file_to_downloads(
                            PathBuf::from(&response.local_path),
                            name,
                            window,
                            cx,
                        );
                    }
                }
            }
            (PendingRequest::Stickers, rpc_response::Result::ListStickers(response)) => {
                self.sticker_library_loading = false;
                self.store.set_sticker_packs(response.packs);
            }
            (
                PendingRequest::SendStickerFromLibrary { .. },
                rpc_response::Result::SendStickerFromLibrary(sent),
            ) => {
                if let Some(message) = sent.message {
                    let sent_to_selected_chat =
                        self.store.selected_chat_id.as_deref() == Some(message.chat_id.as_str());
                    self.store.upsert_message(message);
                    if sent_to_selected_chat {
                        self.scroll_to_bottom_generation = Some(self.message_generation);
                    }
                }
            }
            (
                PendingRequest::ForwardMessage { target_chat_id, .. },
                rpc_response::Result::ForwardMessage(sent),
            ) => {
                if let Some(message) = sent.message {
                    let sent_to_selected_chat =
                        self.store.selected_chat_id.as_deref() == Some(target_chat_id.as_str());
                    self.store.upsert_message(message);
                    if sent_to_selected_chat {
                        self.scroll_to_bottom_generation = Some(self.message_generation);
                    }
                }
            }
            (PendingRequest::EditMessage { .. }, rpc_response::Result::EditMessage(edited)) => {
                if let Some(message) = edited.message {
                    self.store.upsert_message(message);
                    self.scroll_to_bottom_generation = Some(self.message_generation);
                }
            }
            (
                PendingRequest::DeleteMessage {
                    chat_id,
                    message_id,
                },
                rpc_response::Result::DeleteMessage(deleted),
            ) => {
                self.pending_message_deletes
                    .remove(&(chat_id.clone(), message_id.clone()));
                if let Some(message) = deleted.message {
                    self.store.upsert_message(message);
                }
            }
            (
                PendingRequest::SetMessagePin {
                    chat_id,
                    message_id,
                    generation,
                },
                rpc_response::Result::SetMessagePin(_),
            ) => {
                // port: the origin reloads the pinned messages when the pin
                // that is still the current one settles.
                let key = (chat_id.clone(), message_id.clone());
                if self.pin_generations.get(&key).copied() == Some(generation) {
                    self.load_pinned_messages(chat_id);
                }
            }
            (
                PendingRequest::PinnedMessages { chat_id },
                rpc_response::Result::ListPinnedMessages(response),
            ) => {
                self.store.set_pinned_messages(chat_id, response.pins);
            }
            (PendingRequest::CreatePoll { .. }, rpc_response::Result::CreatePoll(created)) => {
                if let Some(message) = created.message {
                    self.store.upsert_message(message);
                    self.scroll_to_bottom_generation = Some(self.message_generation);
                }
            }
            (
                PendingRequest::VotePoll {
                    chat_id,
                    poll_message_id,
                    generation,
                },
                rpc_response::Result::VotePoll(voted),
            ) => {
                let key = (chat_id.clone(), poll_message_id.clone());
                let current = self.poll_vote_generations.get(&key).copied() == Some(generation);
                if current {
                    self.pending_poll_votes.remove(&key);
                    if let Some(message) = voted.message {
                        self.store.upsert_message(message);
                    }
                }
            }
            (PendingRequest::Statuses, rpc_response::Result::ListStatuses(response)) => {
                self.store.set_statuses(response.chat_id, response.groups);
            }
            (
                PendingRequest::SubscribeNewsletter,
                rpc_response::Result::SubscribeNewsletter(response),
            ) => {
                if let Some(newsletter) = response.newsletter {
                    let jid = newsletter.jid.clone();
                    self.store.add_newsletter(newsletter);
                    self.select_newsletter(jid);
                }
            }
            (PendingRequest::Newsletters, rpc_response::Result::ListNewsletters(response)) => {
                self.newsletters_loading = false;
                self.store.set_newsletters(response.newsletters);
            }
            (
                PendingRequest::NewsletterMessages { jid },
                rpc_response::Result::ListNewsletterMessages(response),
            ) => {
                self.store.set_newsletter_messages(jid, response.messages);
            }
            (
                PendingRequest::UnsubscribeNewsletter { jid },
                rpc_response::Result::UnsubscribeNewsletter(_),
            ) => {
                if self.store.remove_newsletter(&jid)
                    && self.selected_newsletter.as_deref() == Some(&jid)
                {
                    self.selected_newsletter = None;
                    self.store.set_newsletter_messages(jid, Vec::new());
                }
            }
            (PendingRequest::SetChatPin { .. }, rpc_response::Result::SetChatPin(response)) => {
                if let Some(chat) = response.chat {
                    self.store.upsert_chat(chat);
                }
            }
            (
                PendingRequest::SetChatArchive { .. },
                rpc_response::Result::SetChatArchive(response),
            ) => {
                if let Some(chat) = response.chat {
                    self.store.upsert_chat(chat);
                }
            }
            (PendingRequest::SetChatMute { .. }, rpc_response::Result::SetChatMute(response)) => {
                if let Some(chat) = response.chat {
                    self.store.upsert_chat(chat);
                }
            }
            (PendingRequest::SetChatBlock { .. }, rpc_response::Result::SetChatBlock(response)) => {
                if let Some(chat) = response.chat {
                    self.store.upsert_chat(chat);
                }
            }
            (PendingRequest::Shutdown, rpc_response::Result::Shutdown(_)) => {
                self.store.toast_error =
                    Some(i18n::t("The local backend was asked to stop").into());
            }
            _ => {}
        }
    }

    /// Leaves a toast of the system when a message arrives in the background.
    ///
    /// The decision mirrors `tauri/src/lib/notification-policy.ts`: the
    /// profile enables the toasts, the chat is not muted, the message is not
    /// one of the user and neither the window nor the conversation is in
    /// front. The history that the first sync brings is not news: only
    /// recent messages are announced.
    fn notify_message(&self, message: &Message, window: &Window) {
        if !self.settings.notifications_enabled || !self.store.sync_complete || message.from_me {
            return;
        }
        if now_ms().saturating_sub(message.timestamp_ms) > NOTIFICATION_MAX_AGE_MS {
            return;
        }
        let Some(chat) = self.store.chat(&message.chat_id) else {
            return;
        };
        let chat_in_front =
            self.store.selected_chat_id.as_deref() == Some(message.chat_id.as_str());
        // port: `shouldNotify` is falsy based: a muted chat, or a chat that
        // the user is looking at, gets no toast.
        if chat.muted || (window.is_window_active() && chat_in_front) {
            return;
        }
        self.notifier.show(Note {
            title: notification_title(chat, message),
            body: notification_body(message, self.settings.notification_previews),
        });
    }

    pub(crate) fn handle_event(&mut self, event: BackendEvent, window: &mut Window) {
        if event.sequence == 0 || event.sequence <= self.last_event_sequence {
            return;
        }
        if self.last_event_sequence != 0
            && event.sequence > self.last_event_sequence.saturating_add(1)
        {
            self.store.toast_error = Some(format!(
                "Backend event gap ({} → {}); refreshing local state",
                self.last_event_sequence, event.sequence
            ));
            self.load_chats(String::new());
            if self.store.selected_chat_id.is_some() {
                self.refresh_latest();
            }
        }
        self.last_event_sequence = event.sequence;
        match event.event {
            Some(backend_event::Event::ConnectionChanged(connection)) => {
                let state = connection.state();
                self.store.connection = state;
                self.store.connection_detail = connection.detail;
                if state == ConnectionState::LoggedOut {
                    if self.rpc.logout_pending() {
                        self.store.connection_detail =
                            "Logged out; securely clearing local account data…".into();
                    } else {
                        self.begin_pairing();
                    }
                } else if state == ConnectionState::Connected {
                    if self.store.screen == Screen::Pairing {
                        self.request(
                            rpc_request::Request::GetAuthState(
                                whatfast_protocol::GetAuthStateRequest {},
                            ),
                            PendingRequest::Auth,
                        );
                    }
                    if let Some(chat_id) = self.store.selected_chat_id.clone() {
                        self.repair_recent_reactions(chat_id);
                    }
                }
            }
            Some(backend_event::Event::PairingQr(qr)) => {
                self.store.qr_code = Some(qr.code);
                self.store.qr_expires_at_ms = qr.expires_at_ms;
                self.store.screen = Screen::Pairing;
            }
            Some(backend_event::Event::SyncProgress(sync)) => {
                self.store.apply_sync_progress(
                    sync.chats_processed,
                    sync.messages_processed,
                    sync.complete,
                );

                // WhatsApp may deliver a large initial history in many chunks.
                // Do not lock the user behind a modal syncing screen while
                // already-persisted chats are usable. Refresh the newest chat
                // page after each chunk and let the remaining import continue
                // in the local backend.
                self.store.screen = Screen::Chats;
                if sync.complete {
                    self.store.connection_detail.clear();
                    if !self.sync_complete_reloaded {
                        self.sync_complete_reloaded = true;
                        self.load_chats(String::new());
                    }
                } else {
                    self.store.connection_detail = format!(
                        "Importing older history… {} chats · {} messages",
                        self.store.sync_chats, self.store.sync_messages
                    );
                    self.load_chats(String::new());
                }
            }
            // The status of the sync and the coverage of the history are
            // domain state: the client keeps them even when no panel shows
            // them yet.
            Some(backend_event::Event::SyncStatusChanged(changed)) => {
                if let Some(status) = changed.status {
                    self.store.apply_sync_status(status);
                }
            }
            Some(backend_event::Event::HistoryCoverageChanged(changed)) => {
                if let Some(coverage) = changed.coverage {
                    self.store.set_history_coverage(coverage);
                }
            }
            Some(backend_event::Event::ChatUpserted(upsert)) => {
                if let Some(chat) = upsert.chat {
                    self.store.upsert_chat(chat);
                }
            }
            Some(backend_event::Event::ChatMerged(merge)) => {
                let old_was_selected =
                    self.store.selected_chat_id.as_deref() == Some(merge.old_chat_id.as_str());
                let selected = self
                    .store
                    .merge_chat_id(&merge.old_chat_id, &merge.new_chat_id);
                remap_recent_chat_ids(
                    &mut self.recent_chat_ids,
                    &merge.old_chat_id,
                    &merge.new_chat_id,
                );
                if let Some(switcher) = self.chat_switcher.as_mut() {
                    remap_recent_chat_ids(
                        &mut switcher.chat_ids,
                        &merge.old_chat_id,
                        &merge.new_chat_id,
                    );
                    switcher.highlighted = switcher
                        .highlighted
                        .min(switcher.chat_ids.len().saturating_sub(1));
                }
                remap_chat_draft(
                    &mut self.chat_drafts,
                    &merge.old_chat_id,
                    &merge.new_chat_id,
                    old_was_selected,
                );
                self.avatar_attempted.remove(&merge.old_chat_id);
                self.reaction_repair_attempted.remove(&merge.old_chat_id);
                self.latest_reaction_intents
                    .retain(|(chat_id, _), _| chat_id != &merge.old_chat_id);
                self.load_chats(String::new());
                if selected {
                    self.load_selected_chat(merge.new_chat_id);
                }
            }
            Some(backend_event::Event::MessageUpserted(upsert)) => {
                if let Some(message) = upsert.message {
                    // A message that arrives while the window is in the
                    // background leaves a toast, if the profile asks for it.
                    self.notify_message(&message, window);
                    // The typed-out message arrived; drop the indicator now
                    // rather than waiting for the paused update or the TTL.
                    if !message.from_me
                        && let Some(indicators) = self.typing_indicators.get_mut(&message.chat_id)
                    {
                        indicators.remove(&message.sender_id);
                        if indicators.is_empty() {
                            self.typing_indicators.remove(&message.chat_id);
                        }
                    }
                    if self.store.has_newer_messages
                        && self.store.selected_chat_id.as_deref() == Some(message.chat_id.as_str())
                    {
                        self.store.newer_activity = true;
                    } else {
                        let mut message = message;
                        // A vote that is in flight owns the poll of the
                        // message until its own answer arrives.
                        self.preserve_pending_poll_intent(&mut message);
                        self.store.upsert_message(message);
                        self.refresh_reaction_details();
                    }
                }
            }
            Some(backend_event::Event::TypingChanged(update)) => {
                let sender_key = if update.sender_id.is_empty() {
                    update.sender_name.clone()
                } else {
                    update.sender_id.clone()
                };
                if update.typing {
                    self.typing_indicators
                        .entry(update.chat_id)
                        .or_default()
                        .insert(
                            sender_key,
                            TypingIndicator {
                                sender_name: update.sender_name,
                                recording: update.recording,
                                expires_at: Instant::now() + TYPING_INDICATOR_TTL,
                            },
                        );
                } else if let Some(indicators) = self.typing_indicators.get_mut(&update.chat_id) {
                    indicators.remove(&sender_key);
                    if indicators.is_empty() {
                        self.typing_indicators.remove(&update.chat_id);
                    }
                }
            }
            Some(backend_event::Event::ReceiptUpdated(receipt)) => {
                self.store.update_receipt(receipt)
            }
            Some(backend_event::Event::ReactionUpdated(update)) => {
                if let Some(reaction) = update.reaction {
                    self.store.apply_reaction(reaction, update.removed);
                    self.refresh_reaction_details();
                }
            }
            Some(backend_event::Event::RecentReactionsRepaired(repair))
                if self.store.selected_chat_id.as_deref() == Some(repair.chat_id.as_str()) =>
            {
                self.refresh_latest();
            }
            Some(backend_event::Event::RecentReactionsRepaired(_)) => {}
            // The sticker tray lives only in the Tauri desktop shell; this
            // legacy GPUI reference only keeps the library.
            Some(backend_event::Event::StickersChanged(_)) => {
                // port: the origin asks for the library again when it needs
                // a fresh view. Here the library is only reloaded when it was
                // already open, so a change does not wake the sidecar.
                if !self.store.sticker_packs().is_empty() {
                    self.load_stickers();
                }
            }
            // Pinned-message browsing lives in the Tauri desktop shell; this
            // client keeps the list of the pinned messages.
            Some(backend_event::Event::PinnedMessagesChanged(changed)) => {
                if self.store.selected_chat_id.as_deref() == Some(changed.chat_id.as_str()) {
                    self.load_pinned_messages(changed.chat_id.clone());
                }
            }
            Some(backend_event::Event::Problem(problem)) => {
                if problem.fatal {
                    self.store.screen = Screen::Fatal;
                    self.store.fatal_error = Some(problem.message);
                } else {
                    self.store.toast_error = Some(problem.message);
                }
            }
            None => {}
        }
    }

    pub(crate) fn load_chats(&mut self, cursor: String) {
        if self.rpc.pending_requests().any(|pending| {
            matches!(pending, PendingRequest::Chats { cursor: pending_cursor } if pending_cursor == &cursor)
        }) {
            return;
        }
        self.request(
            rpc_request::Request::ListChats(whatfast_protocol::ListChatsRequest {
                cursor: cursor.clone(),
                limit: CHAT_PAGE_SIZE,
            }),
            PendingRequest::Chats { cursor },
        );
    }

    pub(crate) fn load_avatar(&mut self, chat_id: String) {
        if self.avatar_attempted.contains(&chat_id) || self.rpc.pending_media_count() >= 4 {
            return;
        }
        self.avatar_attempted.insert(chat_id.clone());
        self.request(
            rpc_request::Request::GetChatAvatar(whatfast_protocol::GetChatAvatarRequest {
                chat_id: chat_id.clone(),
            }),
            PendingRequest::Avatar { chat_id },
        );
    }

    pub(crate) fn load_participant_avatar(&mut self, participant_id: String) {
        if participant_id.is_empty()
            || self.participant_avatar_attempted.contains(&participant_id)
            || self.rpc.pending_media_count() >= 4
        {
            return;
        }
        self.participant_avatar_attempted
            .insert(participant_id.clone());
        self.request(
            rpc_request::Request::GetParticipantAvatar(
                whatfast_protocol::GetParticipantAvatarRequest {
                    participant_id: participant_id.clone(),
                },
            ),
            PendingRequest::ParticipantAvatar { participant_id },
        );
    }

    pub(crate) fn load_message_image(&mut self, chat_id: String, message_id: String) {
        let key = (chat_id.clone(), message_id.clone());
        if self.image_download_attempted.contains(&key) || self.rpc.pending_media_count() >= 4 {
            return;
        }
        self.image_download_attempted.insert(key);
        self.request(
            rpc_request::Request::GetMessageImage(whatfast_protocol::GetMessageImageRequest {
                chat_id: chat_id.clone(),
                message_id: message_id.clone(),
            }),
            PendingRequest::MessageImage {
                chat_id,
                message_id,
            },
        );
    }

    pub(crate) fn retry_message_image(&mut self, chat_id: String, message_id: String) {
        let key = (chat_id.clone(), message_id.clone());
        self.image_failures.remove(&key);
        self.image_download_attempted.remove(&key);
        self.store
            .set_message_image_paths(&chat_id, &message_id, String::new(), String::new());
        self.image_viewer = None;
        self.load_message_image(chat_id, message_id);
    }

    pub(crate) fn open_image_viewer(
        &mut self,
        chat_id: String,
        message_id: String,
        path: String,
        caption: String,
        sticker: bool,
    ) {
        self.image_viewer = Some(ImageViewer {
            chat_id,
            message_id,
            path,
            caption,
            sticker,
        });
        self.emoji_target = None;
        self.reaction_details = None;
    }

    pub(crate) fn repair_recent_reactions(&mut self, chat_id: String) {
        if self.store.connection != ConnectionState::Connected
            || !self.reaction_repair_attempted.insert(chat_id.clone())
        {
            return;
        }
        self.request(
            rpc_request::Request::RepairRecentReactions(
                whatfast_protocol::RepairRecentReactionsRequest {
                    chat_id: chat_id.clone(),
                },
            ),
            PendingRequest::RepairRecentReactions { chat_id },
        );
    }

    pub(crate) fn schedule_reaction_repair_retry(
        &mut self,
        chat_id: String,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            Timer::after(REACTION_REPAIR_RETRY).await;
            let _ = this.update(cx, |this, _| {
                this.reaction_repair_attempted.remove(&chat_id);
                if this.store.selected_chat_id.as_deref() == Some(chat_id.as_str()) {
                    this.repair_recent_reactions(chat_id);
                }
            });
        })
        .detach();
    }

    pub(crate) fn rebuild_emoji_results(&mut self) {
        self.emoji_results = Rc::new(filter_emojis(self.emoji_category, &self.emoji_query));
    }

    pub(crate) fn set_emoji_category(&mut self, category: EmojiCategory) {
        self.emoji_category = category;
        self.rebuild_emoji_results();
    }

    pub(crate) fn toggle_composer_emoji(&mut self) {
        // El panel del compositor: la pestana de los emojis.
        if matches!(self.emoji_target, Some(EmojiTarget::Composer))
            && self.panel_tab == PanelTab::Emoji
        {
            self.emoji_target = None;
        } else {
            self.panel_tab = PanelTab::Emoji;
            self.sticker_tray_open = false;
            self.emoji_target = Some(EmojiTarget::Composer);
        }
    }

    /// Opens the panel of the composer in one of its tabs.
    pub(crate) fn set_panel_tab(&mut self, tab: PanelTab, cx: &mut Context<Self>) {
        self.panel_tab = tab;
        match tab {
            PanelTab::Emoji => {
                self.sticker_tray_open = false;
                if self.emoji_target.is_none() {
                    self.emoji_target = Some(EmojiTarget::Composer);
                }
            }
            PanelTab::Sticker => {
                // The picker of the reactions is not the panel of the
                // composer: it stays open.
                if !matches!(self.emoji_target, Some(EmojiTarget::Reaction { .. })) {
                    self.emoji_target = None;
                }
                self.sticker_tray_open = true;
                self.load_stickers();
            }
            // The app has no pictures with movement: the tab is drawn but it
            // cannot be pressed.
            PanelTab::Gif => {}
        }
        cx.notify();
    }

    pub(crate) fn open_reaction_picker(&mut self, chat_id: String, message_id: String) {
        self.reaction_details = None;
        self.emoji_target = Some(EmojiTarget::Reaction {
            chat_id,
            message_id,
        });
    }

    pub(crate) fn update_active_draft_text(&mut self, text: String) {
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            return;
        };
        let draft = self.chat_drafts.entry(chat_id.clone()).or_default();
        draft.text = text;
        if draft.is_empty() {
            self.chat_drafts.remove(&chat_id);
        }
    }

    pub(crate) fn update_active_draft_reply(&mut self, reply_to_message_id: Option<String>) {
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            return;
        };
        let draft = self.chat_drafts.entry(chat_id.clone()).or_default();
        draft.reply_to_message_id = reply_to_message_id;
        if draft.is_empty() {
            self.chat_drafts.remove(&chat_id);
        }
    }

    pub(crate) fn clear_active_draft(&mut self) {
        if let Some(chat_id) = self.store.selected_chat_id.as_ref() {
            self.chat_drafts.remove(chat_id);
        }
        self.replying_to_message_id = None;
    }

    /// Re-evaluates the composer text around the cursor and opens or closes
    /// the @ mention picker accordingly. Only group chats can tag members.
    pub(crate) fn refresh_mention_picker(&mut self, cx: &mut Context<Self>) {
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            self.mention_picker = None;
            return;
        };
        let is_group = self
            .store
            .chat(&chat_id)
            .is_some_and(|chat| chat.kind() == whatfast_protocol::ChatKind::Group);
        if !is_group {
            self.mention_picker = None;
            return;
        }
        let state = self.composer.read(cx);
        let text = state.value().to_string();
        let cursor = state.cursor().min(text.len());
        let Some(token_start) = mention_token_start(&text, cursor) else {
            self.mention_picker = None;
            return;
        };
        let query = text[token_start + 1..cursor].to_string();
        let highlighted = match &self.mention_picker {
            Some(picker) if picker.chat_id == chat_id && picker.token_start == token_start => {
                picker.highlighted
            }
            _ => 0,
        };
        self.mention_picker = Some(MentionPicker {
            chat_id: chat_id.clone(),
            token_start,
            query,
            highlighted,
        });
        self.ensure_mention_directory(chat_id);
    }

    pub(crate) fn ensure_mention_directory(&mut self, chat_id: String) {
        if self.mention_directories.contains_key(&chat_id)
            || self.mention_directory_pending.contains(&chat_id)
            || self.store.connection != ConnectionState::Connected
        {
            return;
        }
        self.mention_directory_pending.insert(chat_id.clone());
        self.request(
            rpc_request::Request::GetChatInfo(whatfast_protocol::GetChatInfoRequest {
                chat_id: chat_id.clone(),
            }),
            PendingRequest::MentionDirectory { chat_id },
        );
    }

    pub(crate) fn mention_matches(&self) -> Vec<ChatParticipant> {
        let Some(picker) = &self.mention_picker else {
            return Vec::new();
        };
        let Some(participants) = self.mention_directories.get(&picker.chat_id) else {
            return Vec::new();
        };
        let query = picker.query.to_lowercase();
        let digits = query.trim_start_matches('+');
        participants
            .iter()
            .filter(|participant| !participant.is_me)
            .filter(|participant| {
                query.is_empty()
                    || participant.display_name.to_lowercase().contains(&query)
                    || (!digits.is_empty()
                        && participant
                            .phone_number
                            .trim_start_matches('+')
                            .contains(digits))
            })
            .take(MAX_MENTION_ROWS)
            .cloned()
            .collect()
    }

    pub(crate) fn mention_popup_visible(&self) -> bool {
        let Some(picker) = &self.mention_picker else {
            return false;
        };
        if self.store.selected_chat_id.as_deref() != Some(picker.chat_id.as_str()) {
            return false;
        }
        self.mention_directory_pending.contains(&picker.chat_id)
            || !self.mention_matches().is_empty()
    }

    /// Replaces the in-progress `@query` token with `@Name ` and records the
    /// participant so send_text can attach the real WhatsApp mention.
    pub(crate) fn apply_mention(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(picker) = self.mention_picker.clone() else {
            return;
        };
        let Some(participant) = self.mention_matches().get(index).cloned() else {
            return;
        };
        let name = mention_display_name(&participant);
        let state = self.composer.read(cx);
        let text = state.value().to_string();
        let cursor = state.cursor().min(text.len());
        self.mention_picker = None;
        // The cursor may have moved since the picker opened (e.g. arrow keys
        // don't emit Change). Only rewrite when the same token is still there.
        if mention_token_start(&text, cursor) != Some(picker.token_start) {
            return;
        }
        let new_text = format!(
            "{}@{} {}",
            &text[..picker.token_start],
            name,
            text[cursor..].trim_start()
        );
        let draft = self.chat_drafts.entry(picker.chat_id.clone()).or_default();
        if !draft
            .mentions
            .iter()
            .any(|entry| entry.jid == participant.participant_id)
        {
            draft.mentions.push(MentionEntry {
                display_name: name,
                jid: participant.participant_id.clone(),
            });
        }
        self.composer.update(cx, |input, cx| {
            input.set_value(new_text, window, cx);
            input.focus(window, cx);
        });
    }

    pub(crate) fn restore_failed_draft(
        &mut self,
        chat_id: &str,
        draft_text: &str,
        reply_to_message_id: Option<String>,
        mentions: &[(String, String)],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let failed_draft = ChatDraft {
            text: draft_text.to_owned(),
            reply_to_message_id,
            mentions: mentions
                .iter()
                .map(|(display_name, jid)| MentionEntry {
                    display_name: display_name.clone(),
                    jid: jid.clone(),
                })
                .collect(),
            editing_message_id: None,
        };
        self.restore_draft(chat_id, failed_draft, window, cx);
    }

    /// Puts a draft back into the composer, when the message it carries
    /// could not be sent.
    fn restore_draft(
        &mut self,
        chat_id: &str,
        failed_draft: ChatDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let should_restore = self
            .chat_drafts
            .get(chat_id)
            .is_none_or(ChatDraft::is_empty);
        if !should_restore {
            return;
        }
        self.chat_drafts
            .insert(chat_id.to_owned(), failed_draft.clone());
        if self.store.selected_chat_id.as_deref() == Some(chat_id)
            && self.composer.read(cx).value().is_empty()
        {
            self.replying_to_message_id = failed_draft.reply_to_message_id;
            self.composer.update(cx, |input, cx| {
                input.set_value(failed_draft.text, window, cx)
            });
        }
    }

    pub(crate) fn start_reply(&mut self, message_id: String) {
        if self.store.message(&message_id).is_none() {
            return;
        }
        self.replying_to_message_id = Some(message_id.clone());
        self.update_active_draft_reply(Some(message_id));
        self.emoji_target = None;
        self.reaction_details = None;
    }

    pub(crate) fn cancel_reply(&mut self) {
        self.replying_to_message_id = None;
        self.update_active_draft_reply(None);
    }

    pub(crate) fn scroll_to_message(&mut self, message_id: &str) {
        if let Some(index) = self
            .store
            .messages
            .iter()
            .position(|message| message.id == message_id)
        {
            self.stop_smooth_scroll(ScrollSurface::Messages);
            self.message_scroll
                .scroll_to_item(index, ScrollStrategy::Center);
        }
    }

    pub(crate) fn open_reaction_details(
        &mut self,
        chat_id: String,
        message_id: String,
        emoji: String,
    ) {
        if self.store.selected_chat_id.as_deref() != Some(chat_id.as_str()) {
            return;
        }
        let Some(message) = self.store.message(&message_id) else {
            return;
        };
        let Some(details) = ReactionDetails::new(chat_id, message, emoji) else {
            return;
        };
        self.emoji_target = None;
        self.reaction_details = Some(details);
    }

    pub(crate) fn refresh_reaction_details(&mut self) {
        let Some(details) = self.reaction_details.as_ref() else {
            return;
        };
        if self.store.selected_chat_id.as_deref() != Some(details.chat_id.as_str()) {
            self.reaction_details = None;
            return;
        }
        let Some(message) = self.store.message(&details.message_id) else {
            self.reaction_details = None;
            return;
        };
        let reactions = reactions_for_emoji(message, &details.emoji);
        if reactions.is_empty() {
            self.reaction_details = None;
        } else if let Some(details) = self.reaction_details.as_mut() {
            details.reactions = Rc::new(reactions);
        }
    }

    pub(crate) fn choose_emoji(
        &mut self,
        emoji: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.emoji_target.clone() {
            Some(EmojiTarget::Composer) => {
                self.composer
                    .update(cx, |input, cx| input.insert(emoji, window, cx));
            }
            Some(EmojiTarget::Reaction {
                chat_id,
                message_id,
            }) => {
                self.send_reaction_request(
                    chat_id,
                    message_id,
                    emoji.to_string(),
                    Uuid::new_v4().to_string(),
                    0,
                );
                self.emoji_target = None;
            }
            None => {}
        }
    }

    /// El chip de una reacción: saca la mía o suma la del emoji del chip.
    ///
    /// port: el cliente de WhatsApp quita la reacción propia al tocar el
    /// chip y suma la propia al tocar el chip de otro. La lista de quienes
    /// reaccionaron se abre con el botón derecho del mouse.
    pub(crate) fn click_reaction_chip(
        &mut self,
        chat_id: String,
        message_id: String,
        emoji: String,
        from_me: bool,
        cx: &mut Context<Self>,
    ) {
        let emoji = if from_me { String::new() } else { emoji };
        self.send_reaction_request(chat_id, message_id, emoji, Uuid::new_v4().to_string(), 0);
        self.reaction_details = None;
        cx.notify();
    }

    pub(crate) fn remove_reaction(&mut self) {
        let Some(EmojiTarget::Reaction {
            chat_id,
            message_id,
        }) = self.emoji_target.clone()
        else {
            return;
        };
        self.send_reaction_request(
            chat_id,
            message_id,
            String::new(),
            Uuid::new_v4().to_string(),
            0,
        );
        self.emoji_target = None;
    }

    pub(crate) fn send_reaction_request(
        &mut self,
        chat_id: String,
        message_id: String,
        emoji: String,
        client_reaction_id: String,
        attempt: u8,
    ) {
        let intent_key = (chat_id.clone(), message_id.clone());
        if attempt == 0 {
            self.latest_reaction_intents
                .insert(intent_key, client_reaction_id.clone());
        } else if !self.reaction_intent_is_current(&chat_id, &message_id, &client_reaction_id) {
            return;
        }
        self.request(
            rpc_request::Request::SendReaction(whatfast_protocol::SendReactionRequest {
                chat_id: chat_id.clone(),
                message_id: message_id.clone(),
                emoji: emoji.clone(),
                client_reaction_id: client_reaction_id.clone(),
            }),
            PendingRequest::SendReaction {
                chat_id,
                message_id,
                emoji,
                client_reaction_id,
                attempt,
            },
        );
    }

    pub(crate) fn reaction_intent_is_current(
        &self,
        chat_id: &str,
        message_id: &str,
        client_reaction_id: &str,
    ) -> bool {
        self.latest_reaction_intents
            .get(&(chat_id.to_owned(), message_id.to_owned()))
            .is_some_and(|latest| latest == client_reaction_id)
    }

    pub(crate) fn clear_reaction_intent_if_current(
        &mut self,
        chat_id: &str,
        message_id: &str,
        client_reaction_id: &str,
    ) {
        let key = (chat_id.to_owned(), message_id.to_owned());
        if self
            .latest_reaction_intents
            .get(&key)
            .is_some_and(|latest| latest == client_reaction_id)
        {
            self.latest_reaction_intents.remove(&key);
        }
    }

    /// The theme in use, as the settings ask.
    pub(crate) fn active_theme(&self) -> AppTheme {
        tokens::theme_by_id(&self.theme_id, &self.custom_themes)
    }

    /// Copies the fields of the interface into the settings and stores them.
    pub(crate) fn save_settings(&mut self) {
        self.settings.ui_scale = self.ui_scale;
        self.settings.density = self.density;
        self.settings.enter_to_send = self.enter_to_send;
        self.settings.split_view = self.split_view;
        self.settings.battery_saver = self.battery_saver;
        self.settings.theme_id = self.theme_id.clone();
        self.settings.custom_themes = self
            .custom_themes
            .iter()
            .filter_map(|theme| serde_json::to_value(theme).ok())
            .collect();
        if let Err(error) = settings::save(&self.settings) {
            self.store.toast_error = Some(format!(
                "{}: {error}",
                i18n::t("Could not save the settings")
            ));
        }
    }

    /// Paints the active theme and measures the messages again.
    pub(crate) fn apply_active_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let theme = self.active_theme();
        let light = theme.appearance == ThemeAppearance::Light;
        Theme::change(
            if light {
                ThemeMode::Light
            } else {
                ThemeMode::Dark
            },
            Some(window),
            cx,
        );
        tokens::apply_theme(&theme, self.ui_scale, cx);
        apply_theme_scale(self.ui_scale, window, cx);
        self.store.invalidate_all_message_heights();
        cx.notify();
    }

    /// Chooses one theme of the catalog.
    pub(crate) fn select_theme(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.theme_id = id;
        self.apply_active_theme(window, cx);
        self.save_settings();
    }

    pub(crate) fn toggle_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The quick button walks between the two palettes of the app; the
        // settings panel is the place for the rest of the catalog.
        let light = self.active_theme().appearance == ThemeAppearance::Light;
        let id = if light {
            "whatsapp-dark"
        } else {
            "whatsapp-light"
        };
        self.select_theme(id.to_string(), window, cx);
    }

    pub(crate) fn set_ui_scale(&mut self, scale: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.ui_scale = normalized_ui_scale(scale);
        apply_theme_scale(self.ui_scale, window, cx);
        self.store.invalidate_all_message_heights();
        self.save_settings();
        cx.notify();
    }

    pub(crate) fn adjust_ui_scale(
        &mut self,
        delta: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_ui_scale(self.ui_scale + delta, window, cx);
    }

    /// Chooses the language of the interface.
    pub(crate) fn set_language(
        &mut self,
        code: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !i18n::set_language(&code) {
            return;
        }
        self.settings.language = code;
        self.refresh_placeholders(window, cx);
        self.save_settings();
        cx.notify();
    }

    /// Rewrites the placeholders after a change of language.
    ///
    /// The placeholder is a value of the input, so it does not follow the
    /// global language on its own.
    pub(crate) fn refresh_placeholders(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.update(cx, |input, cx| {
            input.set_placeholder(i18n::t("Type a message"), window, cx)
        });
        self.emoji_search.update(cx, |input, cx| {
            input.set_placeholder(i18n::t("Search emoji"), window, cx)
        });
        self.search_input.update(cx, |input, cx| {
            input.set_placeholder(i18n::t("Search or start a new chat"), window, cx)
        });
    }

    /// The air between the rows of the list and the messages.
    pub(crate) fn set_density(&mut self, density: Density, cx: &mut Context<Self>) {
        if self.density == density {
            return;
        }
        self.density = density;
        self.store.invalidate_all_message_heights();
        self.save_settings();
        cx.notify();
    }

    /// Whether a plain Enter sends the message.
    pub(crate) fn set_enter_to_send(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.enter_to_send == enabled {
            return;
        }
        self.enter_to_send = enabled;
        self.save_settings();
        cx.notify();
    }

    /// Whether a message in the background shows a toast of the system.
    pub(crate) fn set_notifications(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.settings.notifications_enabled == enabled {
            return;
        }
        self.settings.notifications_enabled = enabled;
        self.save_settings();
        cx.notify();
    }

    /// Whether the toast includes the text of the message.
    pub(crate) fn set_notification_previews(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.settings.notification_previews == enabled {
            return;
        }
        self.settings.notification_previews = enabled;
        self.save_settings();
        cx.notify();
    }

    /// Whether the chat list stays beside the conversation on narrow windows.
    pub(crate) fn set_split_view(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.split_view == enabled {
            return;
        }
        self.split_view = enabled;
        self.save_settings();
        cx.notify();
    }

    /// Whether the wallpaper and the smooth scroll are off.
    pub(crate) fn set_battery_saver(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.battery_saver == enabled {
            return;
        }
        self.battery_saver = enabled;
        if enabled {
            self.stop_smooth_scroll(ScrollSurface::Messages);
            self.stop_smooth_scroll(ScrollSurface::ChatList);
        }
        self.save_settings();
        cx.notify();
    }

    /// Sets the directory of "Save to folder"; `None` asks every time.
    pub(crate) fn set_download_dir(&mut self, directory: Option<PathBuf>, cx: &mut Context<Self>) {
        self.settings.download_dir = directory
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.save_settings();
        cx.notify();
    }

    /// Asks the user for the directory of the downloads.
    pub(crate) fn choose_download_dir(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(i18n::t("Choose the folder for the downloads").into()),
        });
        let view = cx.entity();
        cx.spawn_in(window, async move |_, window| {
            let paths = selected.await.ok()?.ok()??;
            let path = paths.into_iter().next()?;
            window
                .update(|_window, cx| {
                    view.update(cx, |this, cx| {
                        this.set_download_dir(Some(path), cx);
                        cx.notify();
                    })
                })
                .ok()
        })
        .detach();
    }

    /// Makes a custom theme from the one in use, so the user can paint it.
    pub(crate) fn create_custom_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut theme = self.active_theme();
        let count = self.custom_themes.len() + 1;
        let number = count.to_string();
        theme.id = format!("custom-{}", Uuid::new_v4());
        theme.name = i18n::t_args("My theme {count}", &[("count", &number)]);
        // A theme of the user is not builtin: the catalog can edit or delete
        // it, and the builtin ones stay read-only.
        theme.builtin = None;
        let id = theme.id.clone();
        self.custom_themes.push(theme);
        self.theme_id = id.clone();
        self.editing_theme = Some(id);
        self.apply_active_theme(window, cx);
        self.save_settings();
    }

    /// Deletes one custom theme.
    pub(crate) fn delete_custom_theme(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.custom_themes.retain(|theme| theme.id != id);
        if self.theme_id == id {
            // Back to the palette of the app, which always exists.
            self.theme_id.clear();
        }
        if self.editing_theme.as_deref() == Some(id) {
            self.editing_theme = None;
        }
        self.apply_active_theme(window, cx);
        self.save_settings();
    }

    /// Paints the accent of a custom theme with one color of the palette.
    pub(crate) fn set_theme_accent(
        &mut self,
        id: &str,
        color: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let hex = format!("#{color:06x}");
        if let Some(theme) = self.custom_themes.iter_mut().find(|theme| theme.id == id) {
            theme.tokens.set(ThemeToken::Accent, &hex);
            theme
                .tokens
                .set(ThemeToken::AccentHover, &shade_color(color, 0.82));
            theme
                .tokens
                .set(ThemeToken::AccentSoft, &rgba_color(color, 0.18));
            theme
                .tokens
                .set(ThemeToken::AccentFg, accent_foreground(color));
            theme.tokens.set(ThemeToken::QuoteBar, &hex);
        }
        if self.theme_id == id {
            self.apply_active_theme(window, cx);
        }
        self.save_settings();
    }

    /// Changes the filter of the column of chats.
    ///
    /// port: `setFilter` of the origin (`tauri/src/state/app.ts`): the filter
    /// is part of the state of the sidebar, and the channels are asked for
    /// when the filter of the channels is picked.
    pub(crate) fn set_chat_filter(
        &mut self,
        filter: ChatFilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.store.chat_filter() == filter {
            return;
        }
        self.store.set_chat_filter(filter);
        self.message_generation = self.message_generation.wrapping_add(1);
        self.store.selected_chat_id = None;
        self.emoji_target = None;
        self.reaction_details = None;
        self.image_viewer = None;
        self.replying_to_message_id = None;
        self.chat_switcher = None;
        self.chat_info = None;
        // This unmounts the composer (and emoji popup); if either held focus
        // the root would fall off the key dispatch path, deadening shortcuts.
        // The sidebar search input is the only input that survives the toggle.
        if !self
            .search_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
        {
            self.focus_handle.focus(window, cx);
        }
    }

    /// Marks a conversation as a favorite, or takes the mark away.
    ///
    /// port: `toggleFavorite` of the origin (`tauri/src/state/app.ts`): the
    /// mark belongs to the profile, so it is written to the file of the
    /// profile and not to the sidecar.
    pub(crate) fn toggle_favorite(&mut self, chat_id: String, cx: &mut Context<Self>) {
        self.store.toggle_favorite(&chat_id);
        if let Err(error) = favorites::save_favorites(self.store.favorite_chat_ids()) {
            self.store.toast_error = Some(format!("Could not save the favorites: {error}"));
        }
        cx.notify();
    }

    /// Opens the menu of a row of the list of conversations.
    ///
    /// port: the `contextmenu` of the origin opens the menu of the
    /// conversation under the pointer, where the pointer is.
    pub(crate) fn open_chat_context_menu(
        &mut self,
        chat_id: String,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.chat_context_menu = Some(ChatContextMenu { chat_id, position });
        cx.notify();
    }

    /// Closes the menu of a row of the list of conversations.
    pub(crate) fn close_chat_context_menu(&mut self, cx: &mut Context<Self>) {
        self.chat_context_menu = None;
        cx.notify();
    }

    /// Opens the menu of the options of a conversation.
    pub(crate) fn open_chat_menu(
        &mut self,
        chat_id: String,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.chat_menu = Some(ChatMenu { chat_id, position });
        cx.notify();
    }

    /// Closes the menu of the options of a conversation.
    pub(crate) fn close_chat_menu(&mut self, cx: &mut Context<Self>) {
        self.chat_menu = None;
        cx.notify();
    }

    /// Runs one action of the menu of the options of a conversation.
    pub(crate) fn run_chat_menu_action(
        &mut self,
        action: ChatMenuAction,
        chat_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.chat_menu = None;
        match action {
            ChatMenuAction::Info => {
                self.open_chat_info(chat_id);
            }
            ChatMenuAction::Search => {
                self.focus_search(window, cx);
            }
            ChatMenuAction::Mute => {
                let muted = self.store.chat(&chat_id).is_some_and(|chat| !chat.muted);
                self.set_chat_muted(chat_id, muted, None);
            }
            ChatMenuAction::Favorite => self.toggle_favorite(chat_id, cx),
            ChatMenuAction::Block => {
                let blocked = self.store.chat(&chat_id).is_some_and(|chat| !chat.blocked);
                self.set_chat_blocked(chat_id, blocked);
            }
        }
        cx.notify();
    }

    /// Marks a conversation as unread.
    ///
    /// port: `markChatUnread` of the origin (`tauri/src/state/app.ts`): the
    /// count of unread messages goes up by one.
    pub(crate) fn mark_chat_unread(&mut self, chat_id: String, cx: &mut Context<Self>) {
        self.store.mark_chat_unread(&chat_id);
        cx.notify();
    }

    /// Marks a conversation as read.
    ///
    /// port: `markChatRead` of the origin: the conversation that is open is
    /// marked read through its last incoming message, and any other one
    /// loses its count of unread messages.
    pub(crate) fn mark_chat_read(&mut self, chat_id: String, cx: &mut Context<Self>) {
        if self.store.selected_chat_id.as_deref() == Some(chat_id.as_str()) {
            self.mark_read();
            return;
        }
        self.store.mark_chat_read_locally(&chat_id);
        cx.notify();
    }

    /// Runs one action of the menu of a row of the list of conversations.
    pub(crate) fn run_chat_context_action(
        &mut self,
        action: ChatContextAction,
        chat_id: String,
        cx: &mut Context<Self>,
    ) {
        self.chat_context_menu = None;
        match action {
            ChatContextAction::Pin => {
                let pinned = self.store.chat(&chat_id).is_some_and(|chat| !chat.pinned);
                self.set_chat_pinned(chat_id, pinned);
            }
            ChatContextAction::Mute => {
                let muted = self.store.chat(&chat_id).is_some_and(|chat| !chat.muted);
                self.set_chat_muted(chat_id, muted, None);
            }
            ChatContextAction::Archive => {
                let archived = self.store.chat(&chat_id).is_some_and(|chat| !chat.archived);
                self.set_chat_archived(chat_id, archived);
            }
            ChatContextAction::Favorite => self.toggle_favorite(chat_id, cx),
            ChatContextAction::MarkRead => self.mark_chat_read(chat_id, cx),
            ChatContextAction::MarkUnread => self.mark_chat_unread(chat_id, cx),
            ChatContextAction::Block => {
                let blocked = self.store.chat(&chat_id).is_some_and(|chat| !chat.blocked);
                self.set_chat_blocked(chat_id, blocked);
            }
        }
        cx.notify();
    }

    /// Leaves the column in the filter that shows the conversation.
    ///
    /// An archived conversation only shows up under the filter of the
    /// archived, so the column moves there when the user opens one.
    fn show_chat_in_the_column(&mut self, chat_id: &str) {
        let filter = if self.store.chat(chat_id).is_some_and(|chat| chat.archived) {
            ChatFilter::Archived
        } else {
            ChatFilter::All
        };
        if self.store.chat_filter() != filter {
            self.store.set_chat_filter(filter);
        }
        // An archived conversation only shows in the view of the archived: the
        // column moves there so the user sees where the chat is.
        self.archived_open = filter == ChatFilter::Archived;
        self.states_open = false;
        self.channels_open = false;
    }

    pub(crate) fn recent_chat_candidates(&self) -> Vec<String> {
        let mut candidates = self
            .recent_chat_ids
            .iter()
            .filter(|chat_id| self.store.chat(chat_id).is_some())
            .cloned()
            .collect::<Vec<_>>();
        // The recency history only exists within a session, so seed from the
        // visible chat list (already recency-ordered) until enough history has
        // accumulated to cycle.
        if candidates.len() < 2 {
            for chat_id in self.store.chat_ids(self.store.chat_filter()).iter() {
                if !candidates.contains(chat_id) {
                    candidates.push(chat_id.clone());
                }
                if candidates.len() >= MAX_RECENT_CHATS {
                    break;
                }
            }
        }
        if let Some(selected_chat_id) = self.store.selected_chat_id.as_deref() {
            if let Some(selected_index) = candidates
                .iter()
                .position(|chat_id| chat_id == selected_chat_id)
            {
                candidates.swap(0, selected_index);
            } else if self.store.chat(selected_chat_id).is_some() {
                candidates.insert(0, selected_chat_id.to_owned());
            }
        }
        candidates.truncate(MAX_RECENT_CHATS);
        candidates
    }

    /// The "typing…" line for a chat, or None when nobody is typing there.
    /// `group` picks the phrasing that names the member.
    pub(crate) fn typing_label(&self, chat_id: &str, group: bool) -> Option<String> {
        format_typing_label(self.typing_indicators.get(chat_id)?, group, Instant::now())
    }

    /// Broadcasts the local composing state, re-signalling at most once per
    /// TYPING_RESIGNAL window while the draft keeps changing.
    pub(crate) fn signal_typing(&mut self, composing: bool) {
        if self.store.connection != ConnectionState::Connected {
            return;
        }
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            return;
        };
        match (&self.typing_signal, composing) {
            (Some((signaled, at)), true)
                if *signaled == chat_id && at.elapsed() < TYPING_RESIGNAL =>
            {
                return;
            }
            (None, false) => return,
            (Some((signaled, _)), false) if *signaled != chat_id => {
                self.typing_signal = None;
                return;
            }
            _ => {}
        }
        self.typing_signal = composing.then(|| (chat_id.clone(), Instant::now()));
        self.request(
            rpc_request::Request::SetTyping(whatfast_protocol::SetTypingRequest {
                chat_id,
                typing: composing,
            }),
            PendingRequest::SetTyping,
        );
    }

    pub(crate) fn open_chat_info(&mut self, chat_id: String) {
        self.settings_open = false;
        self.emoji_target = None;
        self.reaction_details = None;
        self.image_viewer = None;
        self.chat_switcher = None;
        self.stop_smooth_scroll(ScrollSurface::ChatInfo);
        self.chat_info_scroll.set_offset(point(px(0.), px(0.)));
        self.chat_info = Some(ChatInfoState {
            chat_id: chat_id.clone(),
            loading: true,
            error: None,
            info: None,
        });
        self.request(
            rpc_request::Request::GetChatInfo(whatfast_protocol::GetChatInfoRequest {
                chat_id: chat_id.clone(),
            }),
            PendingRequest::ChatInfo { chat_id },
        );
    }

    pub(crate) fn cycle_recent_chat(
        &mut self,
        reverse: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.store.screen != Screen::Chats {
            return;
        }
        if let Some(switcher) = self.chat_switcher.as_mut() {
            switcher.cycle(reverse);
            return;
        }
        let candidates = self.recent_chat_candidates();
        let selected_is_first = self
            .store
            .selected_chat_id
            .as_deref()
            .is_some_and(|selected| candidates.first().is_some_and(|first| first == selected));
        let Some(switcher) = ChatSwitcher::new(candidates, reverse, selected_is_first) else {
            return;
        };
        self.emoji_target = None;
        self.reaction_details = None;
        self.image_viewer = None;
        self.chat_info = None;
        self.settings_open = false;
        self.chat_switcher = Some(switcher);
        // The commit-on-ctrl-release and escape handlers live on the root div,
        // which only sees events while focus is somewhere inside it. Opening
        // the switcher may unmount the focused input (emoji search), so anchor
        // focus to the root for the switcher's lifetime.
        self.focus_handle.focus(window, cx);
    }

    /// The handler of the switcher actions that the window may not deliver.
    ///
    /// port: the origin registered the same handler globally, because the
    /// root of the window never sees the key events when the focus is
    /// elsewhere.
    pub(crate) fn cycle_recent_chat_from_global(
        this: &WeakEntity<Self>,
        window: AnyWindowHandle,
        reverse: bool,
        cx: &mut App,
    ) {
        let Some(this) = this.upgrade() else {
            return;
        };
        // Global action handlers fire while the window is mid-dispatch, so
        // defer until its lease is released before updating it again.
        cx.defer(move |cx| {
            window
                .update(cx, |_, window, cx| {
                    this.update(cx, |app, cx| {
                        app.cycle_recent_chat(reverse, window, cx);
                        cx.notify();
                    });
                })
                .ok();
        });
    }

    pub(crate) fn cancel_chat_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chat_switcher = None;
        if self.store.selected_chat_id.is_some() {
            self.composer
                .update(cx, |input, cx| input.focus(window, cx));
        }
    }

    pub(crate) fn commit_chat_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected_chat_id = self
            .chat_switcher
            .take()
            .and_then(|switcher| switcher.selected_chat_id().map(str::to_owned));
        if let Some(chat_id) = selected_chat_id {
            self.open_chat(chat_id, window, cx);
        }
    }

    pub(crate) fn commit_chat_switcher_to(
        &mut self,
        chat_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.chat_switcher = None;
        self.open_chat(chat_id, window, cx);
    }

    pub(crate) fn open_chat(
        &mut self,
        chat_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_chat_with(chat_id, true, window, cx);
    }

    /// Selects a chat that is already open in a tab.
    ///
    /// The strip keeps the order the user gave it: the recency history is not
    /// rewritten, so a click on a tab does not move it to the front.
    fn select_open_tab(&mut self, chat_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.open_chat_with(chat_id, false, window, cx);
    }

    fn open_chat_with(
        &mut self,
        chat_id: String,
        record_recent: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.store.chat(&chat_id).is_none() {
            return;
        }
        // Volver a tocar la conversacion que ya esta abierta no vacia la
        // bandeja: solo se vacia cuando el usuario se va a otra.
        let switching_chat = self.store.selected_chat_id.as_deref() != Some(chat_id.as_str());
        self.show_chat_in_the_column(&chat_id);
        if record_recent {
            record_recent_chat(&mut self.recent_chat_ids, &chat_id);
        }
        self.load_selected_chat(chat_id.clone());
        // The pictures of the tray belong to the conversation that was open:
        // the reference drops them when the user moves to another chat.
        if switching_chat {
            self.pending_images.clear();
            // The note belongs to the conversation where it started: leaving
            // the chat drops it, the same way the reference does. One that is
            // already being coded is kept and sent to its own conversation.
            if self
                .recording
                .as_ref()
                .is_some_and(|state| !state.finishing)
            {
                self.discard_voice_recording();
            }
        }
        let draft = self.chat_drafts.get(&chat_id).cloned().unwrap_or_default();
        self.replying_to_message_id = draft.reply_to_message_id;
        self.composer.update(cx, |input, cx| {
            input.set_value(draft.text, window, cx);
            input.focus(window, cx);
        });
        self.refresh_composer_placeholder(window, cx);
        // The strip of the open chats follows the recency history.
        self.sync_shell(cx);
    }

    pub(crate) fn load_selected_chat(&mut self, chat_id: String) {
        if let Some((signaled_chat_id, _)) = self.typing_signal.take()
            && signaled_chat_id != chat_id
        {
            self.request(
                rpc_request::Request::SetTyping(whatfast_protocol::SetTypingRequest {
                    chat_id: signaled_chat_id,
                    typing: false,
                }),
                PendingRequest::SetTyping,
            );
        }
        self.message_generation = self.message_generation.wrapping_add(1);
        self.stop_smooth_scroll(ScrollSurface::Messages);
        self.store.select_chat(chat_id.clone());
        self.emoji_target = None;
        self.reaction_details = None;
        self.image_viewer = None;
        self.chat_info = None;
        self.mention_picker = None;
        self.scroll_to_bottom_generation = None;
        self.search_target_message_id = None;
        self.pending_search_scroll_id = None;
        self.pending_top_scroll_id = None;
        self.newer_load_failed = false;
        self.image_download_attempted.clear();
        self.image_failures.clear();
        self.request(
            rpc_request::Request::OpenMessageWindow(whatfast_protocol::OpenMessageWindowRequest {
                chat_id: chat_id.clone(),
            }),
            PendingRequest::OpenMessageWindow {
                chat_id: chat_id.clone(),
                generation: self.message_generation,
            },
        );
        // port: the origin reads the coverage and the pinned messages of
        // every conversation it opens.
        self.load_history_coverage(chat_id.clone());
        self.load_pinned_messages(chat_id.clone());
        self.repair_recent_reactions(chat_id);
    }

    pub(crate) fn load_older_messages(&mut self) {
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            return;
        };
        if self.rpc.pending_requests().any(|pending| {
            matches!(pending, PendingRequest::Messages {
                chat_id: pending_chat,
                prepend: true,
                generation,
            } if pending_chat == &chat_id && *generation == self.message_generation)
        }) {
            return;
        }
        let Some(first) = self.store.messages.first() else {
            return;
        };
        self.pending_prepend_anchor = Some((first.id.clone(), self.message_scroll.offset()));
        self.request(
            rpc_request::Request::ListMessages(whatfast_protocol::ListMessagesRequest {
                chat_id: chat_id.clone(),
                before_timestamp_ms: first.timestamp_ms,
                before_message_id: first.id.clone(),
                limit: MESSAGE_PAGE_SIZE,
            }),
            PendingRequest::Messages {
                chat_id,
                prepend: true,
                generation: self.message_generation,
            },
        );
    }

    pub(crate) fn load_newer_messages(&mut self) {
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            return;
        };
        if self.newer_load_failed
            || (!self.store.has_newer_messages && !self.store.newer_activity)
            || self.rpc.pending_requests().any(|pending| {
                matches!(pending, PendingRequest::MessagesAfter {
                    chat_id: pending_chat,
                    generation,
                } if pending_chat == &chat_id && *generation == self.message_generation)
            })
        {
            return;
        }
        let Some(last) = self.store.messages.last() else {
            return;
        };
        let after_timestamp_ms = last.timestamp_ms;
        let after_message_id = last.id.clone();
        // Any event arriving after this point sets newer_activity again, so a
        // response can distinguish activity included in its query from a race.
        self.store.newer_activity = false;
        self.request(
            rpc_request::Request::ListMessagesAfter(whatfast_protocol::ListMessagesAfterRequest {
                chat_id: chat_id.clone(),
                after_timestamp_ms,
                after_message_id,
                limit: MESSAGE_PAGE_SIZE,
            }),
            PendingRequest::MessagesAfter {
                chat_id,
                generation: self.message_generation,
            },
        );
    }

    pub(crate) fn refresh_latest(&mut self) {
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            return;
        };
        self.load_selected_chat(chat_id);
    }

    pub(crate) fn begin_pairing(&mut self) {
        if self.store.screen == Screen::Pairing
            && self
                .rpc
                .pending_requests()
                .any(|pending| matches!(pending, PendingRequest::Pairing))
        {
            return;
        }
        self.store.clear_account_state();
        self.rpc.clear_pending();
        self.store.screen = Screen::Pairing;
        self.store.connection = ConnectionState::Pairing;
        self.message_generation = self.message_generation.wrapping_add(1);
        self.pending_prepend_anchor = None;
        self.sync_complete_reloaded = false;
        self.last_event_sequence = 0;
        self.confirming_logout = false;
        self.avatar_attempted.clear();
        self.avatar_retries.clear();
        self.participant_avatar_attempted.clear();
        self.image_download_attempted.clear();
        self.image_failures.clear();
        self.reaction_repair_attempted.clear();
        self.latest_reaction_intents.clear();
        self.poll_vote_generations.clear();
        self.pending_poll_votes.clear();
        self.pending_message_deletes.clear();
        self.pin_generations.clear();
        self.attachment_download_attempted.clear();
        self.attachment_failures.clear();
        self.attachment_open_requested = None;
        self.attachment_save_requested = None;
        // The state stays until the thread answers: a note that was already
        // being coded is deleted when its file comes out, never sent.
        self.discard_voice_recording();
        self.integrity_checking = false;
        self.sticker_library_loading = false;
        self.newsletters_loading = false;
        self.selected_newsletter = None;
        self.emoji_target = None;
        self.reaction_details = None;
        self.image_viewer = None;
        self.replying_to_message_id = None;
        self.chat_drafts.clear();
        self.recent_chat_ids.clear();
        self.chat_switcher = None;
        self.chat_info = None;
        self.store.set_chat_filter(ChatFilter::All);
        self.request(
            rpc_request::Request::StartPairing(whatfast_protocol::StartPairingRequest {}),
            PendingRequest::Pairing,
        );
    }

    pub(crate) fn request_logout(&mut self) {
        if self.rpc.logout_pending() {
            return;
        }
        self.request(
            rpc_request::Request::Logout(whatfast_protocol::LogoutRequest {}),
            PendingRequest::Logout,
        );
    }

    pub(crate) fn send_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft_text = self.composer.read(cx).value().to_string();
        let text = draft_text.trim().to_string();
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            self.store.toast_error = Some(i18n::t("Choose a conversation before sending").into());
            return;
        };
        if self.store.connection != ConnectionState::Connected {
            self.store.toast_error = Some(i18n::t("Reconnect to WhatsApp before sending").into());
            return;
        }
        if let Err(error) = validate_text_message(&text) {
            self.store.toast_error = Some(error);
            return;
        }
        self.store.toast_error = None;
        let reply_to_message_id = self.replying_to_message_id.clone();
        let editing_message_id = self
            .chat_drafts
            .get(&chat_id)
            .and_then(|draft| draft.editing_message_id.clone());
        if let Some(message_id) = editing_message_id {
            // port: the origin sends an edit instead of a text when the draft
            // carries the id of the message being edited.
            self.mention_picker = None;
            self.typing_signal = None;
            self.clear_active_draft();
            self.composer
                .update(cx, |input, cx| input.set_value("", window, cx));
            report_failed_request(
                &mut self.store,
                self.rpc.request_edit_message(chat_id, message_id, text),
            );
            return;
        }
        let draft_mentions = self
            .chat_drafts
            .get(&chat_id)
            .map(|draft| draft.mentions.clone())
            .unwrap_or_default();
        let (wire_text, mentioned_jids) = encode_mentions(&text, &draft_mentions);
        self.mention_picker = None;
        // Delivering the message clears the composing indicator for peers.
        self.typing_signal = None;
        self.clear_active_draft();
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        let client_message_id = Uuid::new_v4().to_string();
        self.request(
            rpc_request::Request::SendText(whatfast_protocol::SendTextRequest {
                client_message_id: client_message_id.clone(),
                chat_id,
                text: wire_text,
                reply_to_message_id: reply_to_message_id.clone().unwrap_or_default(),
                mentioned_jids,
                reply_to_chat_id: String::new(),
            }),
            PendingRequest::SendText {
                // port: the origin re-read the selected conversation here
                // instead of reusing the one it had just moved into the
                // request. The two are the same, but the quirk is kept.
                chat_id: self.store.selected_chat_id.clone().unwrap_or_default(),
                draft_text,
                reply_to_message_id,
                mentions: draft_mentions
                    .iter()
                    .map(|entry| (entry.display_name.clone(), entry.jid.clone()))
                    .collect(),
            },
        );
    }

    /// Asks the user for one or more pictures and stages them in the tray of
    /// the composer.
    ///
    /// The pictures do not go out here: they wait for the button of the send,
    /// the same way the reference shows them over the field of the message.
    pub(crate) fn choose_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.store.connection != ConnectionState::Connected
            || self.store.selected_chat_id.is_none()
        {
            return;
        }
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Choose photos to send".into()),
        });
        let view = cx.entity();
        cx.spawn_in(window, async move |_, window| {
            let paths = selected.await.ok()?.ok()??;
            window
                .update(|window, cx| {
                    view.update(cx, |this, cx| {
                        this.stage_images(paths, window, cx);
                        cx.notify();
                    })
                })
                .ok()
        })
        .detach();
    }

    /// Puts one or more pictures in the tray of the composer, in order.
    ///
    /// The first picture that the tray refuses stops the batch: the user sees
    /// why and the pictures that fit stay staged.
    pub(crate) fn stage_images(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.store.selected_chat_id.is_none() {
            self.store.toast_error = Some(i18n::t("Choose a conversation before sending").into());
            return;
        }
        if self.store.connection != ConnectionState::Connected {
            self.store.toast_error = Some(i18n::t("Reconnect to WhatsApp before sending").into());
            return;
        }
        // The toast of the batch is the first picture that the tray refused:
        // the ones that fit stay staged.
        let mut failure = None;
        for path in paths {
            if let Err(error) = self.pending_images.stage(path) {
                failure = Some(error);
                break;
            }
        }
        self.store.toast_error = failure;
        self.refresh_composer_placeholder(window, cx);
    }

    /// Puts the message of the composer out: the pictures of the tray when
    /// there are any, the text otherwise.
    pub(crate) fn submit_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_images.is_empty() {
            self.send_text(window, cx);
        } else {
            self.send_pending_images(window, cx);
        }
    }

    /// Sends the pictures of the tray, with the text of the composer as the
    /// caption of the first one.
    ///
    /// port: the reference sends the pictures of a batch as one message with
    /// one caption. The protocol of the sidecar sends one picture per call and
    /// carries the caption inside each one, so the caption travels with the
    /// first picture and the rest go without it.
    pub(crate) fn send_pending_images(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            self.store.toast_error = Some(i18n::t("Choose a conversation before sending").into());
            return;
        };
        if self.store.connection != ConnectionState::Connected {
            self.store.toast_error = Some(i18n::t("Reconnect to WhatsApp before sending").into());
            return;
        }
        let draft_text = self.composer.read(cx).value().to_string();
        let caption = draft_text.trim().to_string();
        // Every picture is checked before the first one goes out: a refused
        // picture leaves the whole tray in place.
        for image in self.pending_images.images() {
            if let Err(error) = validate_image_message(&image.path, &caption) {
                self.store.toast_error = Some(error);
                return;
            }
        }
        let pending = self.pending_images.take_all();
        self.store.toast_error = None;
        let reply_to_message_id = self.replying_to_message_id.clone();
        self.mention_picker = None;
        // Delivering the message clears the composing indicator for peers.
        self.typing_signal = None;
        self.clear_active_draft();
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.refresh_composer_placeholder(window, cx);
        for (index, image) in pending.into_iter().enumerate() {
            let caption = if index == 0 {
                caption.clone()
            } else {
                String::new()
            };
            let reply_to_message_id = if index == 0 {
                reply_to_message_id.clone()
            } else {
                None
            };
            self.request(
                rpc_request::Request::SendImage(whatfast_protocol::SendImageRequest {
                    client_message_id: Uuid::new_v4().to_string(),
                    chat_id: chat_id.clone(),
                    image_path: image.path.to_string_lossy().into_owned(),
                    caption,
                    reply_to_message_id: reply_to_message_id.clone().unwrap_or_default(),
                    reply_to_chat_id: String::new(),
                }),
                PendingRequest::SendImage {
                    // port: the same quirk as in `send_text`.
                    chat_id: self.store.selected_chat_id.clone().unwrap_or_default(),
                    draft_text: draft_text.clone(),
                    reply_to_message_id,
                },
            );
        }
    }

    /// Puts the options of one picture of the tray in front.
    pub(crate) fn select_pending_image(&mut self, index: usize, cx: &mut Context<Self>) {
        self.pending_images.select(index);
        cx.notify();
    }

    /// Drops one picture of the tray.
    pub(crate) fn remove_pending_image(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_images.remove(index);
        self.refresh_composer_placeholder(window, cx);
    }

    /// Opens the editor over one picture of the tray: the crop and the
    /// rotation, the same options of the reference.
    pub(crate) fn edit_pending_image(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self
            .pending_images
            .get(index)
            .map(|image| image.path.clone())
        else {
            return;
        };
        self.open_media_editor(path, window, cx);
    }

    /// The field of the message says what the next message is: the text or
    /// the caption of the pictures of the tray.
    pub(crate) fn refresh_composer_placeholder(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let placeholder = if self.pending_images.is_empty() {
            i18n::t("Type a message")
        } else {
            i18n::t("Add a caption…")
        };
        self.composer.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx)
        });
    }

    pub(crate) fn choose_sticker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            return;
        };
        if self.store.connection != ConnectionState::Connected || self.sticker_preparing {
            return;
        }
        let reply_to_message_id = self.replying_to_message_id.clone();
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose an image to turn into a sticker".into()),
        });
        let view = cx.entity();
        cx.spawn_in(window, async move |_, window| {
            let path = selected.await.ok()?.ok()??.into_iter().next()?;
            window
                .update(|_, cx| {
                    view.update(cx, |this, cx| {
                        this.sticker_preparing = true;
                        this.store.toast_error = None;
                        cx.notify();
                    })
                })
                .ok()?;
            let prepared = smol::unblock(move || sticker::prepare(&path)).await;
            window
                .update(|_, cx| {
                    view.update(cx, |this, cx| {
                        this.sticker_preparing = false;
                        match prepared {
                            Ok(prepared) => this.send_sticker(
                                prepared,
                                chat_id.clone(),
                                reply_to_message_id.clone(),
                            ),
                            Err(error) => this.store.toast_error = Some(error),
                        }
                        cx.notify();
                    })
                })
                .ok()
        })
        .detach();
    }

    pub(crate) fn send_sticker(
        &mut self,
        prepared: sticker::PreparedSticker,
        chat_id: String,
        reply_to_message_id: Option<String>,
    ) {
        if self.store.connection != ConnectionState::Connected {
            self.store.toast_error = Some(i18n::t("Reconnect to WhatsApp before sending").into());
            return;
        }
        self.store.toast_error = None;
        self.request(
            rpc_request::Request::SendSticker(whatfast_protocol::SendStickerRequest {
                client_message_id: Uuid::new_v4().to_string(),
                chat_id: chat_id.clone(),
                webp_data: prepared.webp_data,
                reply_to_message_id: reply_to_message_id.clone().unwrap_or_default(),
                reply_to_chat_id: String::new(),
            }),
            PendingRequest::SendSticker,
        );
        if let Some(draft) = self.chat_drafts.get_mut(&chat_id) {
            draft.reply_to_message_id = None;
            if draft.is_empty() {
                self.chat_drafts.remove(&chat_id);
            }
        }
        if self.store.selected_chat_id.as_deref() == Some(chat_id.as_str())
            && self.replying_to_message_id == reply_to_message_id
        {
            self.replying_to_message_id = None;
        }
    }

    pub(crate) fn mark_read(&mut self) {
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            return;
        };
        let Some(last) = self
            .store
            .messages
            .iter()
            .rev()
            .find(|message| !message.from_me)
        else {
            return;
        };
        let through_message_id = last.id.clone();
        if self.store.last_mark_read_id.as_deref() == Some(through_message_id.as_str()) {
            return;
        }
        self.store.last_mark_read_id = Some(through_message_id.clone());
        let previous_unread = self.store.mark_selected_chat_read_locally().unwrap_or(0);
        self.request(
            rpc_request::Request::MarkRead(whatfast_protocol::MarkReadRequest {
                chat_id,
                through_message_id,
            }),
            PendingRequest::MarkRead {
                // port: the same quirk as in `send_text`.
                chat_id: self.store.selected_chat_id.clone().unwrap_or_default(),
                previous_unread,
            },
        );
    }

    // ---------------------------------------------------------------------
    // The sync, the coverage of the history and the health of the cache
    // ---------------------------------------------------------------------

    /// Asks the sidecar for the progress of the synchronization.
    pub(crate) fn load_sync_status(&mut self) {
        report_failed_request(&mut self.store, self.rpc.request_sync_status());
    }

    /// Asks WhatsApp for more history of one conversation.
    pub(crate) fn request_older_history(&mut self, chat_id: String) {
        report_failed_request(&mut self.store, self.rpc.request_older_history(chat_id));
    }

    /// Asks the sidecar for the coverage of the history of one conversation.
    pub(crate) fn load_history_coverage(&mut self, chat_id: String) {
        report_failed_request(&mut self.store, self.rpc.request_history_coverage(chat_id));
    }

    /// Asks the sidecar for the summary of the history of the account.
    pub(crate) fn load_history_overview(&mut self) {
        report_failed_request(&mut self.store, self.rpc.request_history_overview());
    }

    /// Asks the sidecar for the health of the local database.
    pub(crate) fn load_integrity_status(&mut self) {
        self.integrity_checking = true;
        report_failed_request(&mut self.store, self.rpc.request_integrity_status());
    }

    /// Repairs the local database of the sidecar.
    pub(crate) fn repair_local_cache(&mut self) {
        self.integrity_checking = true;
        report_failed_request(&mut self.store, self.rpc.request_repair_local_cache());
    }

    /// Asks the sidecar to reconnect to WhatsApp.
    ///
    /// port: the origin refuses to reconnect when the account is already
    /// connected, and it shows the state of the connection while it waits.
    pub(crate) fn reconnect(&mut self) {
        if self.store.connection == ConnectionState::Connected
            || self.store.connection == ConnectionState::Connecting
        {
            return;
        }
        self.store.connection = ConnectionState::Connecting;
        self.store.connection_detail = "Connecting on your request".into();
        report_failed_request(&mut self.store, self.rpc.request_reconnect());
    }

    /// Asks the sidecar to stop.
    pub(crate) fn shutdown(&mut self) {
        report_failed_request(&mut self.store, self.rpc.request_shutdown());
    }

    // ---------------------------------------------------------------------
    // The attachments
    // ---------------------------------------------------------------------

    /// Sends a file that is not an image.
    pub(crate) fn send_attachment(
        &mut self,
        path: PathBuf,
        kind: AttachmentKind,
        caption: String,
        voice_note: bool,
    ) {
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            self.store.toast_error = Some(i18n::t("Choose a conversation before sending").into());
            return;
        };
        self.send_attachment_to(chat_id, path, kind, caption, voice_note);
    }

    /// Sends the note that the microphone finished coding to the conversation
    /// where it started, not to the one that happens to be open now.
    fn send_voice_note(&mut self, chat_id: String, path: PathBuf) {
        self.send_attachment_to(chat_id, path, AttachmentKind::Audio, String::new(), true);
    }

    fn send_attachment_to(
        &mut self,
        chat_id: String,
        path: PathBuf,
        kind: AttachmentKind,
        caption: String,
        voice_note: bool,
    ) {
        if self.store.connection != ConnectionState::Connected {
            self.store.toast_error = Some(i18n::t("Reconnect to WhatsApp before sending").into());
            return;
        }
        self.store.toast_error = None;
        report_failed_request(
            &mut self.store,
            self.rpc.request_send_attachment(OutgoingAttachment {
                client_message_id: Uuid::new_v4().to_string(),
                chat_id,
                file_path: path.to_string_lossy().into_owned(),
                kind,
                caption,
                reply_to_message_id: String::new(),
                voice_note,
            }),
        );
    }

    // ---------------------------------------------------------------------
    // The voice notes that the composer records
    // ---------------------------------------------------------------------

    /// Opens the microphone and starts keeping the note.
    pub(crate) fn start_voice_recording(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.recording.is_some() {
            return;
        }
        if self.store.connection != ConnectionState::Connected {
            self.store.toast_error = Some(i18n::t("Reconnect to WhatsApp before sending").into());
            cx.notify();
            return;
        }
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            self.store.toast_error = Some(i18n::t("Choose a conversation before sending").into());
            cx.notify();
            return;
        };
        self.store.toast_error = None;
        self.recorder.start();
        self.recording = Some(RecordingState {
            started: Instant::now(),
            finishing: false,
            chat_id,
            discarded: false,
        });
        self.start_recording_tick(window, cx);
        cx.notify();
    }

    /// Closes the microphone and codes the note.
    pub(crate) fn stop_voice_recording(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.recording.as_mut() else {
            return;
        };
        if state.finishing {
            return;
        }
        state.finishing = true;
        self.recorder.stop();
        cx.notify();
    }

    /// Throws the note away without sending it.
    pub(crate) fn cancel_voice_recording(&mut self, cx: &mut Context<Self>) {
        self.discard_voice_recording();
        cx.notify();
    }

    /// Marks the note as thrown away and closes the microphone. The state
    /// stays until the thread answers, so a `Finished` that was already on
    /// its way still finds the file and deletes it instead of sending it.
    fn discard_voice_recording(&mut self) {
        let Some(state) = self.recording.as_mut() else {
            return;
        };
        if state.discarded {
            return;
        }
        state.discarded = true;
        self.recorder.cancel();
    }

    /// Reads what the thread of the microphone says and counts the clock.
    pub(crate) fn tick_recording(&mut self, cx: &mut Context<Self>) {
        while let Some(event) = self.recorder.try_recv() {
            match event {
                RecorderEvent::Started => {}
                RecorderEvent::Finished { path, duration } => {
                    let state = self.recording.take();
                    let target = note_target(state.as_ref()).map(str::to_owned);
                    match target {
                        Some(chat_id) if duration >= MIN_DURATION_SECONDS => {
                            self.send_voice_note(chat_id, path);
                        }
                        Some(_) => {
                            let _ = std::fs::remove_file(&path);
                            self.store.toast_error =
                                Some(i18n::t("The voice note is too short").into());
                        }
                        // The note was thrown away while it was being coded:
                        // the file is removed and nothing is sent.
                        None => {
                            let _ = std::fs::remove_file(&path);
                        }
                    }
                }
                RecorderEvent::Failed(message) => {
                    let discarded = self.recording.as_ref().is_some_and(|state| state.discarded);
                    self.recording = None;
                    if !discarded {
                        self.store.toast_error = Some(message);
                    }
                }
                RecorderEvent::Cancelled => {
                    self.recording = None;
                }
            }
        }
        // The note of the reference stops on its own at fifteen minutes: the
        // client keeps five, because the samples wait in memory.
        let too_long = self.recording.as_ref().is_some_and(|state| {
            !state.finishing && !state.discarded && state.elapsed_seconds() >= MAX_DURATION_SECONDS
        });
        if too_long {
            self.stop_voice_recording(cx);
        }
        cx.notify();
    }

    /// Starts the tick that counts the recording, once per window.
    fn start_recording_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.recording_tick {
            return;
        }
        self.recording_tick = true;
        let view = cx.entity().downgrade();
        cx.spawn_in(window, async move |_, window| {
            loop {
                Timer::after(Duration::from_millis(100)).await;
                let recording = window.update(|_, cx| {
                    view.update(cx, |this, cx| {
                        this.tick_recording(cx);
                        let recording = this.recording.is_some();
                        if !recording {
                            this.recording_tick = false;
                        }
                        recording
                    })
                });
                match recording {
                    Ok(Ok(true)) => {}
                    _ => break,
                }
            }
        })
        .detach();
    }

    /// Pega la imagen del portapapeles en la bandeja del compositor.
    ///
    /// La foto no sale sola: queda esperando el boton de mandar, con la cruz
    /// para sacarla y el lapiz para recortarla, como en la referencia.
    ///
    /// Devuelve `true` cuando el portapapeles tenia una imagen: el texto
    /// sigue el camino normal del campo de escritura.
    pub(crate) fn paste_image_from_clipboard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let Some(image) = item.entries.iter().find_map(|entry| match entry {
            gpui::ClipboardEntry::Image(image) => Some(image.clone()),
            _ => None,
        }) else {
            return false;
        };
        if image.bytes.len() as u64 > MAX_IMAGE_BYTES {
            self.store.toast_error = Some(i18n::t("The pasted image is too big").into());
            return true;
        }
        // El portapapeles de Windows entrega el mapa de bits como BMP, que
        // WhatsApp no manda como foto: las demas formas se pasan a PNG.
        let (bytes, extension) = match image.format {
            gpui::ImageFormat::Png => (image.bytes.clone(), "png"),
            gpui::ImageFormat::Jpeg => (image.bytes.clone(), "jpg"),
            gpui::ImageFormat::Gif => (image.bytes.clone(), "gif"),
            gpui::ImageFormat::Webp => (image.bytes.clone(), "webp"),
            _ => {
                let Ok(decoded) = image::load_from_memory(&image.bytes) else {
                    self.store.toast_error =
                        Some(i18n::t("The pasted image could not be read").into());
                    return true;
                };
                let mut png = Vec::new();
                if decoded
                    .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                    .is_err()
                {
                    self.store.toast_error =
                        Some(i18n::t("The pasted image could not be read").into());
                    return true;
                }
                (png, "png")
            }
        };
        // El archivo que pide el backend tiene que existir en el disco: la
        // imagen del portapapeles se guarda en la carpeta temporal.
        let directory = std::env::temp_dir().join("whatfast-pegado");
        if std::fs::create_dir_all(&directory).is_err() {
            self.store.toast_error = Some(i18n::t("The pasted image could not be saved").into());
            return true;
        }
        let path = directory.join(format!("pegado-{}.{}", Uuid::new_v4(), extension));
        if std::fs::write(&path, &bytes).is_err() {
            self.store.toast_error = Some(i18n::t("The pasted image could not be saved").into());
            return true;
        }
        self.stage_images(vec![path], window, cx);
        true
    }

    /// Asks the user for a file and sends it to the open conversation.
    pub(crate) fn choose_attachment(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        kind: AttachmentKind,
    ) {
        if self.store.connection != ConnectionState::Connected
            || self.store.selected_chat_id.is_none()
        {
            return;
        }
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose a file to send".into()),
        });
        let view = cx.entity();
        cx.spawn_in(window, async move |_, window| {
            let path = selected.await.ok()?.ok()??.into_iter().next()?;
            window
                .update(|_, cx| {
                    view.update(cx, |this, cx| {
                        this.send_attachment(path, kind, String::new(), false);
                        cx.notify();
                    })
                })
                .ok()
        })
        .detach();
    }

    /// Downloads the file of one attachment into the local cache.
    pub(crate) fn load_message_attachment(&mut self, chat_id: String, message_id: String) {
        let key = (chat_id.clone(), message_id.clone());
        if self.attachment_download_attempted.contains(&key) || self.rpc.pending_media_count() >= 4
        {
            return;
        }
        self.attachment_download_attempted.insert(key);
        report_failed_request(
            &mut self.store,
            self.rpc.request_message_attachment(chat_id, message_id),
        );
    }

    /// Downloads the file of one attachment again, after a failure.
    pub(crate) fn retry_message_attachment(&mut self, chat_id: String, message_id: String) {
        let key = (chat_id.clone(), message_id.clone());
        self.attachment_failures.remove(&key);
        self.attachment_download_attempted.remove(&key);
        self.store
            .set_message_attachment_path(&chat_id, &message_id, String::new());
        self.load_message_attachment(chat_id, message_id);
    }

    /// Opens the file of one attachment.
    ///
    /// The file that is already in the disk goes to the program of the
    /// system, and the one that is not there is asked for to the sidecar: it
    /// goes to the program of the system when it arrives.
    ///
    /// port: `openAttachment` of `tauri/src/state/app.ts` opens the file
    /// that is in the disk and downloads the one that is not there.
    pub(crate) fn open_message_attachment(
        &mut self,
        chat_id: String,
        message_id: String,
        cx: &mut Context<Self>,
    ) {
        let key = (chat_id.clone(), message_id.clone());
        // A download that failed before is asked again.
        if self.attachment_failures.remove(&key).is_some() {
            self.attachment_download_attempted.remove(&key);
        }
        let Some(path) =
            self.store
                .message(&message_id)
                .and_then(|message| match message.content.as_ref() {
                    Some(whatfast_protocol::message::Content::Attachment(attachment)) => {
                        Some(attachment.local_path.clone())
                    }
                    _ => None,
                })
        else {
            // The message has no attachment: there is nothing to open.
            return;
        };
        if !path.is_empty() && Path::new(&path).is_file() {
            self.open_file_with_the_system(&path, cx);
            return;
        }
        // The file is not in the disk: the path that the message had is
        // stale, and the file is asked for again.
        self.store
            .set_message_attachment_path(&chat_id, &message_id, String::new());
        self.attachment_open_requested = Some(key);
        self.load_message_attachment(chat_id, message_id);
    }

    /// Opens one file with the program of the system.
    ///
    /// port: the menu of the clip of the composer opens the file that the
    /// user chose with the same url (`file:///C:/...`).
    pub(crate) fn open_file_with_the_system(&mut self, path: &str, cx: &mut Context<Self>) {
        let url = format!("file:///{}", path.replace('\\', "/"));
        cx.open_url(&url);
    }

    /// Copies an attachment that is already on the disk into the folder of the
    /// downloads.
    ///
    /// When the settings do not name a folder, the system asks for one. The
    /// copy never overwrites a file: a number goes before the extension.
    pub(crate) fn save_message_attachment(
        &mut self,
        chat_id: String,
        message_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(attachment) =
            self.store
                .message(&message_id)
                .and_then(|message| match message.content.as_ref() {
                    Some(whatfast_protocol::message::Content::Attachment(attachment)) => {
                        Some(attachment.clone())
                    }
                    _ => None,
                })
        else {
            self.store.toast_error = Some(i18n::t("That file is no longer available").into());
            cx.notify();
            return;
        };
        if !attachment.local_path.is_empty() && Path::new(&attachment.local_path).is_file() {
            let name =
                (!attachment.file_name.trim().is_empty()).then(|| attachment.file_name.clone());
            self.save_file_to_downloads(PathBuf::from(&attachment.local_path), name, window, cx);
            return;
        }
        // The file is not in the disk yet: the arrow asks for the download
        // and the copy happens when the file arrives, without opening it.
        let key = (chat_id.clone(), message_id.clone());
        self.store
            .set_message_attachment_path(&chat_id, &message_id, String::new());
        self.attachment_failures.remove(&key);
        self.attachment_download_attempted.remove(&key);
        self.attachment_save_requested = Some(key);
        self.load_message_attachment(chat_id, message_id);
        cx.notify();
    }

    /// Saves one picture of the viewer into the folder of the downloads.
    pub(crate) fn save_image_to_downloads(
        &mut self,
        path: String,
        caption: String,
        sticker: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if path.is_empty() || !Path::new(&path).is_file() {
            self.store.toast_error = Some(i18n::t("That file is no longer available").into());
            cx.notify();
            return;
        }
        let stem = {
            let caption = caption.trim();
            if caption.is_empty() {
                if sticker {
                    "sticker".to_string()
                } else {
                    "photo".to_string()
                }
            } else {
                caption.chars().take(60).collect::<String>()
            }
        };
        self.save_file_to_downloads(PathBuf::from(path), Some(stem), window, cx);
    }

    /// Copies one file into the folder of the downloads, asking for it when
    /// the settings do not name one.
    fn save_file_to_downloads(
        &mut self,
        source: PathBuf,
        suggested_name: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let configured = self.settings.download_dir.trim().to_string();
        if !configured.is_empty() {
            self.copy_attachment_to(&source, Path::new(&configured), suggested_name, window, cx);
            return;
        }
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(i18n::t("Choose the folder for the downloads").into()),
        });
        let view = cx.entity();
        cx.spawn_in(window, async move |_, window| {
            let paths = selected.await.ok()?.ok()??;
            let directory = paths.into_iter().next()?;
            window
                .update(|window, cx| {
                    view.update(cx, |this, cx| {
                        this.copy_attachment_to(
                            &source,
                            &directory,
                            suggested_name.clone(),
                            window,
                            cx,
                        );
                        cx.notify();
                    })
                })
                .ok()
        })
        .detach();
    }

    /// Copies the file, with a number when the name is taken.
    ///
    /// The copy runs on the executor of the background: a video of a few
    /// hundred megabytes takes long enough to freeze the window if the bytes
    /// travelled in the thread of the interface.
    fn copy_attachment_to(
        &mut self,
        source: &Path,
        directory: &Path,
        suggested_name: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The name of the profile wins: the cache of the backend names the
        // files by hash and the user expects the name of the document, or the
        // caption of the picture. A name without an extension takes the one
        // of the file that is copied.
        let mut name = suggested_name
            .as_deref()
            .map(sanitize_file_name)
            .filter(|name| !name.is_empty())
            .or_else(|| {
                source
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            });
        if let Some(name) = name.as_mut()
            && Path::new(name.as_str()).extension().is_none()
            && let Some(extension) = source.extension()
            && !extension.is_empty()
        {
            name.push('.');
            name.push_str(&extension.to_string_lossy());
        }
        let Some(name) = name else {
            self.store.toast_error = Some(i18n::t("The file could not be saved").into());
            return;
        };
        let stem = Path::new(&name)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| name.clone());
        let extension = Path::new(&name)
            .extension()
            .map(|extension| format!(".{}", extension.to_string_lossy()))
            .unwrap_or_default();
        let source = source.to_path_buf();
        let directory = directory.to_path_buf();
        let copy = cx.background_spawn(async move {
            // Three tries cover the double click: the second copy finds the
            // name taken by the first one and only changes the number.
            let mut counter = 1;
            let mut last_error = None;
            for _ in 0..3 {
                let mut destination = directory.join(&name);
                while destination.exists() {
                    destination = directory.join(format!("{stem} ({counter}){extension}"));
                    counter += 1;
                }
                // A copy that dies halfway leaves a `.part` file, never half
                // of the destination: the rename is the instant that makes
                // the file visible in the folder.
                let partial = directory.join(format!(".whatfast-{}.part", uuid::Uuid::new_v4()));
                let result = std::fs::copy(&source, &partial)
                    .and_then(|_| std::fs::rename(&partial, &destination));
                if result.is_err() {
                    let _ = std::fs::remove_file(&partial);
                }
                match result {
                    Ok(()) => return Ok(destination),
                    // Another copy of the same file won the name: the loop
                    // picks the next one.
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        last_error = Some(error);
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(last_error.unwrap_or_else(|| std::io::Error::other("the name stayed taken")))
        });
        let view = cx.entity().downgrade();
        cx.spawn_in(window, async move |_, window| {
            let result = copy.await;
            window
                .update(|_window, cx| {
                    view.update(cx, |this, cx| {
                        match result {
                            Ok(destination) => {
                                this.store.toast_error = None;
                                this.store.toast_info = Some(i18n::t_args(
                                    "Saved to {path}",
                                    &[("path", &destination.display().to_string())],
                                ));
                                this.clear_toast_info_later(cx);
                            }
                            Err(error) => {
                                this.store.toast_error = Some(format!(
                                    "{}: {error}",
                                    i18n::t("The file could not be saved")
                                ));
                            }
                        }
                        cx.notify();
                    })
                    .ok()
                })
                .ok()
        })
        .detach();
    }

    /// Takes the green notice out after a moment.
    fn clear_toast_info_later(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            Timer::after(Duration::from_secs(4)).await;
            let _ = this.update(cx, |this, cx| {
                this.store.toast_info = None;
                cx.notify();
            });
        })
        .detach();
    }

    // ---------------------------------------------------------------------
    // The library of stickers
    // ---------------------------------------------------------------------

    /// Asks the sidecar for the library of stickers.
    pub(crate) fn load_stickers(&mut self) {
        self.sticker_library_loading = true;
        report_failed_request(&mut self.store, self.rpc.request_stickers());
    }

    /// Sends one sticker of the library to the open conversation.
    pub(crate) fn send_sticker_from_library(&mut self, sticker_id: String) {
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            self.store.toast_error = Some(i18n::t("Choose a conversation before sending").into());
            return;
        };
        if self.store.connection != ConnectionState::Connected {
            self.store.toast_error = Some(i18n::t("Reconnect to WhatsApp before sending").into());
            return;
        }
        if sticker_id.trim().is_empty() {
            self.store.toast_error = Some(i18n::t("That sticker is no longer available").into());
            return;
        }
        self.store.toast_error = None;
        report_failed_request(
            &mut self.store,
            self.rpc.request_send_sticker_from_library(
                Uuid::new_v4().to_string(),
                chat_id,
                sticker_id,
            ),
        );
    }

    // ---------------------------------------------------------------------
    // The messages: forward, edit and delete
    // ---------------------------------------------------------------------

    /// Sends one message to another conversation.
    pub(crate) fn forward_message(
        &mut self,
        source_chat_id: String,
        message_id: String,
        target_chat_id: String,
    ) {
        if self.store.connection != ConnectionState::Connected {
            self.store.toast_error = Some(i18n::t("Reconnect to WhatsApp before sending").into());
            return;
        }
        self.store.toast_error = None;
        report_failed_request(
            &mut self.store,
            self.rpc.request_forward_message(
                Uuid::new_v4().to_string(),
                source_chat_id,
                message_id,
                target_chat_id,
            ),
        );
    }

    /// Puts the text of one message in the composer to edit it.
    ///
    /// port: the origin does the same: the draft carries the id of the
    /// message being edited and the composer sends an edit instead of a text.
    pub(crate) fn start_edit(
        &mut self,
        message_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            return;
        };
        let Some(message) = self.store.message(&message_id) else {
            return;
        };
        if !message.from_me || message.revoked {
            return;
        }
        let Some(whatfast_protocol::message::Content::Text(text)) = message.content.as_ref() else {
            return;
        };
        let text = text.text.clone();
        let draft = self.chat_drafts.entry(chat_id.clone()).or_default();
        draft.text = text.clone();
        draft.reply_to_message_id = None;
        draft.mentions.clear();
        draft.editing_message_id = Some(message_id);
        self.replying_to_message_id = None;
        self.composer
            .update(cx, |input, cx| input.set_value(text, window, cx));
    }

    /// Drops the edit of a message and empties the composer.
    ///
    /// port: the origin empties the whole draft when the edit is cancelled.
    pub(crate) fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            return;
        };
        self.chat_drafts.remove(&chat_id);
        self.replying_to_message_id = None;
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    /// Deletes one message of the open conversation.
    ///
    /// port: the origin marks the message as revoked before the answer
    /// arrives, and puts it back when the backend refuses.
    pub(crate) fn delete_message(&mut self, message_id: String, delete_for_everyone: bool) {
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            return;
        };
        let Some(message) = self.store.message(&message_id).cloned() else {
            return;
        };
        if delete_for_everyone && !message.from_me {
            self.store.toast_error =
                Some("Only your own messages can be deleted for everyone".into());
            return;
        }
        let Some(previous) = self.store.revoke_message(&chat_id, &message_id) else {
            return;
        };
        self.pending_message_deletes
            .insert((chat_id.clone(), message_id.clone()), previous);
        report_failed_request(
            &mut self.store,
            self.rpc
                .request_delete_message(chat_id, message_id, delete_for_everyone),
        );
    }

    /// Pins or unpins one message of the open conversation.
    pub(crate) fn set_message_pin(&mut self, message_id: String, pinned: bool) {
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            return;
        };
        let key = (chat_id.clone(), message_id.clone());
        let generation = {
            let entry = self.pin_generations.entry(key).or_insert(0);
            *entry = entry.wrapping_add(1);
            *entry
        };
        report_failed_request(
            &mut self.store,
            self.rpc
                .request_set_message_pin(chat_id, message_id, pinned, generation),
        );
    }

    /// Asks the sidecar for the pinned messages of one conversation.
    pub(crate) fn load_pinned_messages(&mut self, chat_id: String) {
        report_failed_request(&mut self.store, self.rpc.request_pinned_messages(chat_id));
    }

    // ---------------------------------------------------------------------
    // The polls
    // ---------------------------------------------------------------------

    /// Creates one poll in the open conversation.
    pub(crate) fn create_poll(
        &mut self,
        question: String,
        options: Vec<String>,
        selectable_options_count: u32,
    ) {
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            self.store.toast_error = Some(i18n::t("Choose a conversation before sending").into());
            return;
        };
        if self.store.connection != ConnectionState::Connected {
            self.store.toast_error = Some(i18n::t("Reconnect to WhatsApp before sending").into());
            return;
        }
        self.store.toast_error = None;
        report_failed_request(
            &mut self.store,
            self.rpc.request_create_poll(
                Uuid::new_v4().to_string(),
                chat_id,
                question,
                options,
                selectable_options_count,
            ),
        );
    }

    /// Votes one poll of the open conversation.
    ///
    /// port: the origin reduces the vote on the message right away and keeps
    /// the generation of the vote, so an old answer does not overwrite a
    /// newer intent of the user.
    pub(crate) fn vote_poll(&mut self, poll_message_id: String, selected_options: Vec<String>) {
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            return;
        };
        let key = (chat_id.clone(), poll_message_id.clone());
        let generation = {
            let entry = self.poll_vote_generations.entry(key.clone()).or_insert(0);
            *entry = entry.wrapping_add(1);
            *entry
        };
        let Some(previous) =
            self.store
                .apply_poll_vote(&chat_id, &poll_message_id, &selected_options)
        else {
            return;
        };
        self.pending_poll_votes.insert(key, (generation, previous));
        report_failed_request(
            &mut self.store,
            self.rpc
                .request_vote_poll(chat_id, poll_message_id, selected_options, generation),
        );
    }

    /// Puts the poll of one message back where it was, after a failed vote.
    fn rollback_poll_vote(&mut self, chat_id: &str, poll_message_id: &str, generation: u64) {
        let key = (chat_id.to_owned(), poll_message_id.to_owned());
        if self.poll_vote_generations.get(&key).copied() != Some(generation) {
            // A newer vote is in flight: its intent is the one that counts.
            return;
        }
        let Some((_, previous)) = self.pending_poll_votes.remove(&key) else {
            return;
        };
        self.store
            .restore_poll_content(chat_id, poll_message_id, previous);
    }

    /// Puts the pending vote of one message back into a snapshot of the
    /// poll, so an unsolicited snapshot does not overwrite the intent of the
    /// user.
    ///
    /// port: `preservePendingPollIntent` in `tauri/src/lib/polls.ts`.
    fn preserve_pending_poll_intent(&self, message: &mut whatfast_protocol::Message) {
        let key = (message.chat_id.clone(), message.id.clone());
        let Some((_, pending)) = self.pending_poll_votes.get(&key) else {
            return;
        };
        if !matches!(
            message.content,
            Some(whatfast_protocol::message::Content::Poll(_))
        ) {
            return;
        }
        message.content = Some(whatfast_protocol::message::Content::Poll(pending.clone()));
    }

    // ---------------------------------------------------------------------
    // The statuses and the newsletters
    // ---------------------------------------------------------------------

    /// Asks the sidecar for the statuses of the profile.
    pub(crate) fn load_statuses(&mut self) {
        report_failed_request(&mut self.store, self.rpc.request_statuses());
    }

    /// Asks the sidecar for the newsletters of the account.
    pub(crate) fn load_newsletters(&mut self) {
        self.newsletters_loading = true;
        report_failed_request(&mut self.store, self.rpc.request_newsletters());
    }

    /// Subscribes to one newsletter with its invite key.
    pub(crate) fn subscribe_newsletter(&mut self, invite_key: String) {
        let key = invite_key.trim().to_owned();
        if key.is_empty() {
            return;
        }
        report_failed_request(&mut self.store, self.rpc.request_subscribe_newsletter(key));
    }

    /// Opens one newsletter and asks for its posts.
    pub(crate) fn select_newsletter(&mut self, jid: String) {
        self.selected_newsletter = Some(jid.clone());
        self.store.set_newsletter_messages(jid.clone(), Vec::new());
        report_failed_request(
            &mut self.store,
            self.rpc
                .request_newsletter_messages(jid, NEWSLETTER_PAGE_SIZE),
        );
    }

    /// Drops one newsletter.
    pub(crate) fn unsubscribe_newsletter(&mut self, jid: String) {
        report_failed_request(
            &mut self.store,
            self.rpc.request_unsubscribe_newsletter(jid),
        );
    }

    // ---------------------------------------------------------------------
    // The flags of a conversation
    // ---------------------------------------------------------------------

    /// Pins or unpins one conversation.
    pub(crate) fn set_chat_pinned(&mut self, chat_id: String, pinned: bool) {
        let Some(chat) = self.store.chat(&chat_id) else {
            return;
        };
        let previous_pinned = chat.pinned;
        self.store.set_chat_pinned(&chat_id, pinned);
        report_failed_request(
            &mut self.store,
            self.rpc
                .request_set_chat_pin(chat_id, pinned, previous_pinned),
        );
    }

    /// Archives or unarchives one conversation.
    pub(crate) fn set_chat_archived(&mut self, chat_id: String, archived: bool) {
        let Some(chat) = self.store.chat(&chat_id) else {
            return;
        };
        let previous_archived = chat.archived;
        self.store.set_chat_archived(&chat_id, archived);
        report_failed_request(
            &mut self.store,
            self.rpc
                .request_set_chat_archive(chat_id, archived, previous_archived),
        );
    }

    /// Mutes or unmutes one conversation.
    ///
    /// port: the origin mutes a conversation for a while, and the moment the
    /// mute ends is computed with the clock of the machine.
    pub(crate) fn set_chat_muted(
        &mut self,
        chat_id: String,
        muted: bool,
        duration_ms: Option<i64>,
    ) {
        let Some(chat) = self.store.chat(&chat_id) else {
            return;
        };
        let previous_muted = chat.muted;
        let previous_mute_until_ms = chat.mute_until_ms;
        let mute_until_ms = if muted {
            match duration_ms {
                Some(duration) if duration > 0 => now_ms().saturating_add(duration),
                _ => MUTE_UNTIL_INDEFINITE,
            }
        } else {
            0
        };
        self.store.set_chat_muted(&chat_id, muted, mute_until_ms);
        report_failed_request(
            &mut self.store,
            self.rpc.request_set_chat_mute(
                chat_id,
                mute_until_ms,
                previous_muted,
                previous_mute_until_ms,
            ),
        );
    }

    /// Blocks or unblocks one conversation.
    pub(crate) fn set_chat_blocked(&mut self, chat_id: String, blocked: bool) {
        let Some(chat) = self.store.chat(&chat_id) else {
            return;
        };
        let previous_blocked = chat.blocked;
        self.store.set_chat_blocked(&chat_id, blocked);
        report_failed_request(
            &mut self.store,
            self.rpc
                .request_set_chat_block(chat_id, blocked, previous_blocked),
        );
    }

    /// Puts the draft of an edit back where it was, after a failed edit.
    fn restore_failed_edit(
        &mut self,
        chat_id: &str,
        message_id: &str,
        draft_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.restore_draft(
            chat_id,
            ChatDraft {
                text: draft_text.to_owned(),
                reply_to_message_id: None,
                mentions: Vec::new(),
                editing_message_id: Some(message_id.to_owned()),
            },
            window,
            cx,
        );
    }

    // -----------------------------------------------------------------------
    // The shell: the column of icons and the strip of the open chats
    // -----------------------------------------------------------------------

    /// Answers a button of the column of icons.
    pub(crate) fn on_nav_action(
        &mut self,
        action: NavAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            NavAction::Chats => {
                self.store.set_chat_filter(ChatFilter::All);
                self.states_open = false;
                self.channels_open = false;
                self.archived_open = false;
                self.settings_open = false;
            }
            NavAction::Search => {
                self.states_open = false;
                self.channels_open = false;
                self.archived_open = false;
                self.settings_open = false;
                self.focus_search(window, cx);
            }
            NavAction::States => {
                // The states are their own view: they are not conversations.
                self.states_open = true;
                self.channels_open = false;
                self.archived_open = false;
                self.settings_open = false;
                self.load_statuses();
            }
            NavAction::Channels => {
                // The channels are their own view too.
                self.states_open = false;
                self.channels_open = true;
                self.archived_open = false;
                self.settings_open = false;
                self.load_newsletters();
            }
            NavAction::Archived => {
                // The archived conversations are their own view.
                self.states_open = false;
                self.channels_open = false;
                self.archived_open = true;
                self.settings_open = false;
                self.store.set_chat_filter(ChatFilter::Archived);
            }
            NavAction::Privacy | NavAction::Settings => {
                self.settings_open = true;
            }
        }
        self.sync_shell(cx);
    }

    /// Opens the view of the states.
    ///
    /// The states are their own view: the keyboard reaches them the same way
    /// the column of icons does.
    pub(crate) fn open_states(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.on_nav_action(NavAction::States, window, cx);
    }

    /// Opens the view of the channels.
    pub(crate) fn open_channels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.on_nav_action(NavAction::Channels, window, cx);
    }

    /// Opens the view of the archived conversations.
    pub(crate) fn open_archived(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.on_nav_action(NavAction::Archived, window, cx);
    }

    /// Answers a gesture of the strip of the open chats.
    pub(crate) fn on_tab_action(
        &mut self,
        action: TabAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            TabAction::Select { chat_id } => {
                self.select_open_tab(chat_id, window, cx);
            }
            TabAction::Close {
                chat_id,
                next_chat_id,
                ..
            } => {
                self.recent_chat_ids.retain(|id| id != &chat_id);
                // The closed tab gives the conversation to its neighbour: the
                // same rule the strip already used to pick the next one.
                if self.store.selected_chat_id.as_deref() == Some(chat_id.as_str())
                    && let Some(next_chat_id) = next_chat_id
                {
                    self.select_open_tab(next_chat_id, window, cx);
                }
            }
            TabAction::Reorder { chat_id, index, .. } => {
                self.move_recent_chat(&chat_id, index);
            }
            // The native client has one pane: there is nowhere to move a tab to.
            TabAction::MoveToPane { .. } => {}
            TabAction::Add => {
                self.focus_search(window, cx);
            }
            TabAction::Unhandled | TabAction::Handled | TabAction::Boundary { .. } => {}
        }
        self.sync_shell(cx);
    }

    /// Moves a chat of the recency history to another place.
    fn move_recent_chat(&mut self, chat_id: &str, index: usize) {
        let Some(from) = self.recent_chat_ids.iter().position(|id| id == chat_id) else {
            return;
        };
        let chat_id = self.recent_chat_ids.remove(from);
        let index = index.min(self.recent_chat_ids.len());
        self.recent_chat_ids.insert(index, chat_id);
    }

    /// Keeps the column of icons and the strip of tabs in sync with the state
    /// of the app.
    pub(crate) fn sync_shell(&mut self, cx: &mut Context<Self>) {
        let link = match self.store.screen {
            Screen::Pairing => LinkState::Pairing,
            _ => LinkState::Linked,
        };
        let settings_open = self.settings_open;
        let states_open = self.states_open;
        let channels_open = self.channels_open;
        let archived_open = self.archived_open;
        self.nav_rail.update(cx, |rail, _| {
            rail.set_link(link);
            rail.set_sidebar_open(true);
            rail.set_settings_open(settings_open);
            rail.set_states_open(states_open);
            rail.set_channels_open(channels_open);
            rail.set_archived_open(archived_open);
        });
        let entries: Vec<TabEntry> = self
            .recent_chat_ids
            .iter()
            .filter_map(|chat_id| self.store.chat(chat_id))
            .map(TabEntry::from_chat)
            .collect();
        let selected = self.store.selected_chat_id.clone().unwrap_or_default();
        let active = if entries.iter().any(|entry| entry.chat_id == selected) {
            selected
        } else {
            String::new()
        };
        self.tab_strip.update(cx, |strip, _| {
            strip.set_tabs(entries, active);
        });
    }

    // -----------------------------------------------------------------------
    // The editor of a picture
    // -----------------------------------------------------------------------

    /// Opens the editor of a picture over the conversation.
    pub(crate) fn open_media_editor(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.media_editor.is_some() {
            return;
        }
        let editor = cx.new(|cx| MediaEditor::new(&path, cx));
        let subscription = cx.subscribe_in(&editor, window, move |this, _, event, window, cx| {
            match event {
                MediaEditorEvent::Applied(edit) => {
                    this.media_editor = None;
                    this.stage_edited_picture(edit.clone(), window, cx);
                }
                MediaEditorEvent::Cancelled => {
                    this.media_editor = None;
                }
            }
            cx.notify();
        });
        self._subscriptions.push(subscription);
        self.media_editor = Some(editor);
        self.sync_shell(cx);
    }

    /// Writes the picture the user edited and puts it back in the tray, in
    /// the place of the picture it came from.
    ///
    /// The edited picture does not go out on its own: the tray keeps it until
    /// the user presses send.
    fn stage_edited_picture(
        &mut self,
        edit: crate::media_editor::MediaEdit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.store.selected_chat_id.is_none() {
            return;
        }
        let source = edit.path.clone();
        let plan = edit.plan;
        let extension = source
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("png")
            .to_owned();
        let destination = whatfast_model::paths::data_dir()
            .join("edited")
            .join(format!("{}.{extension}", uuid::Uuid::new_v4()));
        let parent = destination
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf();
        // Decoding, rotating, cropping and encoding a twelve-megapixel photo
        // takes hundreds of milliseconds: the pixels travel in the background
        // so the window keeps painting while the edit is written.
        let task_source = source.clone();
        let edited = cx.background_spawn(async move {
            std::fs::create_dir_all(parent)
                .map_err(|error| error.to_string())
                .and_then(|()| {
                    crate::media_editor::apply_edit(&task_source, &plan, &destination)
                })?;
            Ok::<std::path::PathBuf, String>(destination)
        });
        let view = cx.entity().downgrade();
        cx.spawn_in(window, async move |_, window| {
            let prepared = edited.await;
            window
                .update(|window, cx| {
                    view.update(cx, |this, cx| {
                        match prepared {
                            Ok(destination) => {
                                // The editor was opened over a picture of the
                                // tray: the edited one takes its place, with
                                // the options and the caption already chosen.
                                if !this.pending_images.replace(&source, destination.clone()) {
                                    this.stage_images(vec![destination], window, cx);
                                } else {
                                    this.refresh_composer_placeholder(window, cx);
                                }
                            }
                            Err(error) => this.store.toast_error = Some(error),
                        }
                        cx.notify();
                    })
                    .ok()
                })
                .ok()
        })
        .detach();
    }

    // -----------------------------------------------------------------------
    // The actions on the messages
    // -----------------------------------------------------------------------

    /// Opens the menu of a message.
    ///
    /// `position` is where the pointer of the right click was, in pixels of the
    /// window: the menu opens there and not in the corner of the layout.
    pub(crate) fn open_message_menu(
        &mut self,
        message_id: String,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            return;
        };
        let Some(message) = self.store.message(&message_id).cloned() else {
            return;
        };
        let pinned = self.store.message_is_pinned(&chat_id, &message_id);
        let starred = self.store.message_is_starred(&chat_id, &message_id);
        let scale = self.ui_scale;
        let view = cx.entity();
        let menu = cx.new(|_| {
            MessageMenu::new(chat_id, &message, pinned, starred, now_ms(), scale)
                .at(position)
                .on_action(move |action, window, cx| {
                    view.update(cx, |this, cx| {
                        this.on_message_action(action, window, cx);
                        cx.notify();
                    });
                })
        });
        self.message_menu = Some(menu);
    }

    /// Answers an action of the menu of a message.
    fn on_message_action(
        &mut self,
        action: MessageAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = self.message_menu.clone() else {
            return;
        };
        let chat_id = menu.read(cx).chat_id().to_owned();
        let message_id = menu.read(cx).message_id().to_owned();
        let text = menu.read(cx).text().to_owned();
        self.message_menu = None;
        match action {
            MessageAction::Reply => self.start_reply(message_id),
            MessageAction::Forward => self.open_forward_picker(chat_id, message_id, window, cx),
            MessageAction::Edit => self.start_edit(message_id, window, cx),
            MessageAction::Delete => self.delete_message(message_id, false),
            MessageAction::DeleteForEveryone => self.delete_message(message_id, true),
            MessageAction::Copy => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
            }
            MessageAction::Pin => self.set_message_pin(message_id, true),
            MessageAction::Unpin => self.set_message_pin(message_id, false),
            MessageAction::Star => self.set_message_star(chat_id, message_id, true, cx),
            MessageAction::Unstar => self.set_message_star(chat_id, message_id, false, cx),
        }
    }

    /// Stars a message of this profile, or takes the star off it.
    ///
    /// WhatsApp stars through the app-state of the account and the linked
    /// device does not expose that action: the star is kept in the file of
    /// the profile, like the favorites of the conversations.
    pub(crate) fn set_message_star(
        &mut self,
        chat_id: String,
        message_id: String,
        wanted: bool,
        cx: &mut Context<Self>,
    ) {
        if self.store.message_is_starred(&chat_id, &message_id) != wanted {
            self.store.toggle_starred_message(&chat_id, &message_id);
        }
        if let Err(error) = starred::save_starred(self.store.starred_messages()) {
            self.store.toast_error = Some(format!(
                "{}: {error}",
                i18n::t("The starred messages could not be saved")
            ));
        }
        cx.notify();
    }

    /// Opens the dialog to send a message to another conversation.
    pub(crate) fn open_forward_picker(
        &mut self,
        chat_id: String,
        message_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity();
        let picker = cx.new(|cx| {
            ForwardPicker::new(chat_id, message_id, window, cx).on_action(
                move |action, window, cx| {
                    view.update(cx, |this, cx| {
                        this.on_forward_action(action, window, cx);
                        cx.notify();
                    });
                },
            )
        });
        let chats: Vec<Chat> = self
            .store
            .chat_ids(ChatFilter::All)
            .iter()
            .filter_map(|chat_id| self.store.chat(chat_id).cloned())
            .collect();
        picker.update(cx, |picker, _| {
            picker.set_chats(chats);
        });
        self.forward_picker = Some(picker);
    }

    /// Answers the dialog of the forward.
    fn on_forward_action(
        &mut self,
        action: ForwardAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(picker) = self.forward_picker.clone() else {
            return;
        };
        let source_chat_id = picker.read(cx).source_chat_id().to_owned();
        let message_id = picker.read(cx).message_id().to_owned();
        self.forward_picker = None;
        match action {
            ForwardAction::Forward(targets) => {
                for target in targets {
                    self.forward_message(source_chat_id.clone(), message_id.clone(), target);
                }
            }
            ForwardAction::Cancel => {}
        }
        let _ = window;
    }

    /// Opens the form of a poll.
    pub(crate) fn open_poll_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.store.selected_chat_id.is_none() {
            self.store.toast_error = Some(i18n::t("Choose a conversation before sending").into());
            return;
        }
        let view = cx.entity();
        let composer = cx.new(|cx| {
            PollComposer::new(window, cx).on_action(move |action, window, cx| {
                view.update(cx, |this, cx| {
                    this.on_poll_action(action, window, cx);
                    cx.notify();
                });
            })
        });
        self.poll_composer = Some(composer);
    }

    /// Answers the form of the poll.
    fn on_poll_action(&mut self, action: PollAction, window: &mut Window, cx: &mut Context<Self>) {
        self.poll_composer = None;
        match action {
            PollAction::Create {
                question,
                options,
                selectable_options_count,
            } => {
                self.create_poll(question, options, selectable_options_count);
            }
            PollAction::Cancel => {}
        }
        let _ = (window, cx);
    }

    /// Answers the button of the clip.
    fn on_attachment_action(
        &mut self,
        action: AttachmentAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            AttachmentAction::Send { path, kind } => {
                self.send_attachment(path, kind, String::new(), false);
            }
            AttachmentAction::Open { path } => {
                let url = format!("file:///{}", path.to_string_lossy().replace('\\', "/"));
                cx.open_url(&url);
            }
            AttachmentAction::Saved { path } => {
                self.store.toast_error = None;
                let _ = path;
            }
            AttachmentAction::SaveFailed { message } => {
                self.store.toast_error = Some(message);
            }
            AttachmentAction::Photo => {
                self.choose_image(window, cx);
            }
            AttachmentAction::Poll => {
                self.open_poll_composer(window, cx);
            }
            AttachmentAction::Sticker => {
                self.toggle_sticker_tray(cx);
            }
            AttachmentAction::MakeSticker => {
                self.choose_sticker(window, cx);
            }
        }
        let _ = window;
    }

    /// Opens or closes the tray of the stickers.
    pub(crate) fn toggle_sticker_tray(&mut self, cx: &mut Context<Self>) {
        // El panel del compositor: la pestana de los stickers.
        if self.sticker_tray_open && self.panel_tab == PanelTab::Sticker {
            self.sticker_tray_open = false;
        } else {
            self.panel_tab = PanelTab::Sticker;
            if !matches!(self.emoji_target, Some(EmojiTarget::Reaction { .. })) {
                self.emoji_target = None;
            }
            self.sticker_tray_open = true;
            self.load_stickers();
        }
        cx.notify();
    }
}

/// Reports a request that could not even be queued: the sidecar is gone.
///
/// port: the origin showed the fatal screen when the bridge could not send a
/// request. The helper is a free function because the call borrows two
/// fields of the application at the same time: the store it writes and the
/// client it asks.
fn report_failed_request(store: &mut Store, request: anyhow::Result<u64>) {
    if let Err(error) = request {
        store.screen = Screen::Fatal;
        store.fatal_error = Some(error.to_string());
    }
}

/// The colors of the accent that the custom themes offer.
pub(crate) const ACCENT_SWATCHES: [u32; 10] = [
    0x25d366, 0x00a884, 0x0a84ff, 0x7c4dff, 0xe91e63, 0xff5722, 0xf59e0b, 0x10b981, 0x06b6d4,
    0x64748b,
];

/// One color of the palette, darkened by a factor: the hover of the accent.
fn shade_color(color: u32, factor: f32) -> String {
    let red = (((color >> 16) & 0xff) as f32 * factor).round() as u32;
    let green = (((color >> 8) & 0xff) as f32 * factor).round() as u32;
    let blue = ((color & 0xff) as f32 * factor).round() as u32;
    format!("#{red:02x}{green:02x}{blue:02x}")
}

/// One color of the palette, with an alpha: the soft fill of the accent.
fn rgba_color(color: u32, alpha: f32) -> String {
    format!(
        "rgba({}, {}, {}, {alpha})",
        (color >> 16) & 0xff,
        (color >> 8) & 0xff,
        color & 0xff
    )
}

/// The color of the text that sits on top of the accent.
fn accent_foreground(color: u32) -> &'static str {
    let red = ((color >> 16) & 0xff) as f32 / 255.0;
    let green = ((color >> 8) & 0xff) as f32 / 255.0;
    let blue = (color & 0xff) as f32 / 255.0;
    let luminance = 0.2126 * red + 0.7152 * green + 0.0722 * blue;
    if luminance > 0.55 {
        "#000000"
    } else {
        "#ffffff"
    }
}

/// The current instant, in milliseconds of the epoch.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

/// How old a message can be and still be announced: the history of the sync
/// is older than this, and a notification for it would be noise.
const NOTIFICATION_MAX_AGE_MS: i64 = 60_000;

/// The title of the toast: the chat, the sender or the phone number.
///
/// port: `notificationTitle` of `tauri/src/lib/notification-policy.ts`.
fn notification_title(chat: &Chat, message: &Message) -> String {
    for candidate in [
        chat.title.as_str(),
        message.sender_name.as_str(),
        chat.phone_number.as_str(),
    ] {
        if !candidate.is_empty() {
            return candidate.to_string();
        }
    }
    i18n::t("New WhatsApp message").to_string()
}

/// The body of the toast: the text of the message or the kind of content.
///
/// port: `notificationBody` of `tauri/src/lib/notification-policy.ts`, with
/// the labels translated and the same cut at 180 characters.
fn notification_body(message: &Message, show_preview: bool) -> String {
    if !show_preview {
        return i18n::t("New message").to_string();
    }
    let mut body = i18n::t("Message").to_string();
    if message.revoked {
        body = i18n::t("This message was deleted").to_string();
    } else if let Some(content) = message.content.as_ref() {
        use whatfast_protocol::message::Content;
        body = match content {
            Content::Text(text) => text.text.clone(),
            Content::Image(image) => {
                if !image.caption.is_empty() {
                    image.caption.clone()
                } else if image.sticker {
                    i18n::t("Sticker").to_string()
                } else {
                    i18n::t("Photo").to_string()
                }
            }
            Content::Attachment(attachment) => {
                for candidate in [
                    attachment.caption.as_str(),
                    attachment.file_name.as_str(),
                    attachment.kind.as_str(),
                ] {
                    if !candidate.is_empty() {
                        return candidate.to_string();
                    }
                }
                i18n::t("Attachment").to_string()
            }
            Content::Contacts(contacts) => {
                if contacts.contacts.len() == 1 {
                    contacts
                        .contacts
                        .first()
                        .map(|contact| contact.display_name.clone())
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| i18n::t("Contact").to_string())
                } else {
                    i18n::t_args(
                        "{count} contacts",
                        &[("count", &contacts.contacts.len().to_string())],
                    )
                }
            }
            Content::Location(location) => {
                if !location.name.is_empty() {
                    location.name.clone()
                } else if !location.address.is_empty() {
                    location.address.clone()
                } else {
                    i18n::t("Location").to_string()
                }
            }
            Content::Unsupported(unsupported) => {
                if !unsupported.fallback_text.is_empty() {
                    unsupported.fallback_text.clone()
                } else if !unsupported.type_name.is_empty() {
                    unsupported.type_name.clone()
                } else {
                    i18n::t("Message").to_string()
                }
            }
            Content::Poll(_) => i18n::t("Message").to_string(),
        };
    }
    if body.chars().count() > 180 {
        body = body.chars().take(179).collect::<String>() + "\u{2026}";
    }
    body
}

/// The scale the profile stored, or the default one.
fn load_ui_scale() -> f32 {
    settings::load_ui_scale()
        .filter(|value| value.is_finite())
        .map(normalized_ui_scale)
        .unwrap_or(1.0)
}

/// Stores the scale of the interface in the profile.
fn save_ui_scale(scale: f32) -> std::io::Result<()> {
    settings::save_ui_scale(scale)
}

/// Applies the scale of the interface to the theme of the window.
fn apply_theme_scale(scale: f32, window: &mut Window, cx: &mut App) {
    crate::tokens::apply_theme_scale(scale, window, cx);
}

/// Forgets the save that was waiting for one attachment, when its download
/// failed: the next download of the same file must open it, not save it.
fn forget_attachment_request(request: &mut Option<(String, String)>, key: &(String, String)) {
    if request.as_ref() == Some(key) {
        *request = None;
    }
}

/// The name of a file that a download writes: the characters that the file
/// systems of the three platforms refuse are replaced, and the spaces around
/// the name go away.
fn sanitize_file_name(name: &str) -> String {
    let cleaned = name
        .trim()
        .chars()
        .map(|character| match character {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            character if character.is_control() => '_',
            character => character,
        })
        .collect::<String>();
    // Windows no crea un archivo con un punto o un espacio al final: el
    // nombre se guarda sin ellos.
    let cleaned = cleaned.trim_end_matches([' ', '.']);
    if cleaned.is_empty() {
        return String::new();
    }
    // `CON`, `NUL` o `COM1` nombran un dispositivo en Windows, con cualquier
    // extension: el nombre reservado no se puede usar como archivo.
    let stem = cleaned
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0');
    if reserved {
        format!("_{cleaned}")
    } else {
        cleaned.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The instant of the clock is what the client uses to say until when a
    /// conversation stays muted.
    #[test]
    fn the_instant_of_the_clock_is_in_milliseconds_of_the_epoch() {
        let now = now_ms();
        assert!(now > 1_700_000_000_000, "the clock is not in milliseconds");
        assert!(now < 10_000_000_000_000, "the clock is not in milliseconds");
    }

    /// A request that could not even be queued leaves the client without a
    /// sidecar: the fatal screen is the only honest answer.
    #[test]
    fn a_request_that_could_not_be_queued_shows_the_fatal_screen() {
        let mut store = Store::default();
        report_failed_request(&mut store, Err(anyhow::anyhow!("the sidecar is gone")));

        assert_eq!(store.screen, Screen::Fatal);
        assert_eq!(store.fatal_error.as_deref(), Some("the sidecar is gone"));
    }

    /// A request that was queued leaves the screen alone.
    #[test]
    fn a_request_that_was_queued_leaves_the_screen_alone() {
        let mut store = Store::default();
        report_failed_request(&mut store, Ok(7));

        assert_eq!(store.screen, Screen::Starting);
        assert!(store.fatal_error.is_none());
    }

    /// A note goes to the conversation where it started, even when the user
    /// moved to another chat while the encoder was finishing; one that was
    /// thrown away never gets a destination.
    #[test]
    fn a_note_goes_to_the_conversation_where_it_started() {
        let state = RecordingState {
            started: std::time::Instant::now(),
            finishing: true,
            chat_id: "chat-a".into(),
            discarded: false,
        };
        assert_eq!(note_target(Some(&state)), Some("chat-a"));
        assert_eq!(note_target(None), None);

        let thrown_away = RecordingState {
            discarded: true,
            chat_id: "chat-b".into(),
            ..state
        };
        assert_eq!(note_target(Some(&thrown_away)), None);
    }

    /// A download that failed must not leave a save waiting: the next time
    /// the file arrives, it opens instead of going to the downloads folder.
    #[test]
    fn a_failed_download_forgets_the_save_that_was_waiting() {
        let key = ("chat".to_string(), "message".to_string());
        let mut waiting = Some(key.clone());
        forget_attachment_request(&mut waiting, &key);
        assert_eq!(waiting, None);

        let mut another = Some(("chat".to_string(), "other".to_string()));
        forget_attachment_request(&mut another, &key);
        assert!(another.is_some());
    }

    /// The name of a download cannot carry the characters that the file
    /// systems refuse, and it cannot be empty either.
    #[test]
    fn the_name_of_a_download_is_safe_for_the_disk() {
        assert_eq!(sanitize_file_name("  agenda.pdf  "), "agenda.pdf");
        assert_eq!(
            sanitize_file_name("a/b\\c:d*e?f\"g<h>i|j.pdf"),
            "a_b_c_d_e_f_g_h_i_j.pdf"
        );
        assert_eq!(sanitize_file_name("line\nbreak.txt"), "line_break.txt");
        assert_eq!(sanitize_file_name("   "), "");
        assert_eq!(sanitize_file_name("informe..."), "informe");
        assert_eq!(sanitize_file_name("notas .txt"), "notas .txt");
        assert_eq!(sanitize_file_name("CON.txt"), "_CON.txt");
        assert_eq!(sanitize_file_name("nul"), "_nul");
        assert_eq!(sanitize_file_name("LPT9.tar.gz"), "_LPT9.tar.gz");
        assert_eq!(sanitize_file_name("COM0.txt"), "COM0.txt");
        assert_eq!(sanitize_file_name("console.txt"), "console.txt");
    }

    /// A draft that edits a message is not empty: the message it edits is
    /// still the one the user is working on.
    #[test]
    fn a_draft_that_edits_a_message_is_not_empty() {
        let draft = ChatDraft {
            editing_message_id: Some("message".into()),
            ..Default::default()
        };
        assert!(!draft.is_empty());
        assert!(
            ChatDraft::default().is_empty(),
            "a draft with nothing in it is empty"
        );
    }
}
