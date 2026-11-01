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
    collections::{HashMap, HashSet, VecDeque},
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
    ActiveTheme as _, Theme, ThemeMode, VirtualListScrollHandle,
    input::{InputEvent, InputState},
    slider::{SliderEvent, SliderState},
};
use smol::Timer;
use std::sync::Arc;
use uuid::Uuid;
use whatfast_core::theme::{Theme as AppTheme, ThemeAppearance, ThemeToken};
use whatfast_i18n as i18n;
use whatfast_model::{
    emoji_picker::{EmojiCategory, filtered as filter_emojis},
    favorites,
    model::{CHAT_PAGE_SIZE, ChatFilter, MESSAGE_PAGE_SIZE, Screen, Store, reactions_for_emoji},
    settings::{self, ChatBackground, Density, Settings},
    starred, sticker,
};
use whatfast_protocol::{
    AttachmentKind, BackendEvent, Chat, ChatParticipant, ConnectionState, ContactSearchResult,
    Message, MessageSearchResult, RpcResponse, backend_event, rpc_request, rpc_response,
};

pub(crate) use crate::agents_panel::AgentsPanel;
use crate::audio::{VoiceAudio, VoiceState};
use crate::automations_panel::AutomationsPanel;
use crate::composer_attachments::PendingImages;
use crate::media_editor::{MediaEditor, MediaEditorEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum MediaEditorTarget {
    #[default]
    Chat,
    Status,
}
use crate::members::MemberPanel;
use crate::message_actions::{
    AttachmentAction, AttachmentBar, ForwardAction, ForwardPicker, MessageAction, MessageMenu,
    PollAction, PollComposer,
};
use crate::navrail::{LinkState, NavAction, NavRail};
use crate::newsletters::NewsletterPanel;
use crate::notifications::{Note, Notifier};
use crate::quick_ask::QuickAsk;
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
use crate::video::{VideoEngine, VideoKey, VideoPlayback};

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
        ToggleArchived,
        ToggleAgents,
        FocusSearch,
        OpenQuickAsk,
        EscapeAction,
        ScrollToBottomAction,
        ScrollToTopAction,
        PageUpMessages,
        PageDownMessages,
        SelectPreviousChat,
        SelectNextChat,
        OpenSettingsAction,
        FocusComposerAction
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
/// The rows the field of the message shows: one while it is short, up to six
/// so a long message stays readable while it is written.
pub(crate) const COMPOSER_MIN_ROWS: usize = 1;
pub(crate) const COMPOSER_MAX_ROWS: usize = 6;

/// Cuantas veces se vuelve a preguntar el estado de la cuenta antes de
/// mostrar la pantalla de vinculacion.
///
/// El sidecar abre la sesion despues del saludo y, mientras no la tiene,
/// contesta que no hay dispositivo vinculado. Con 8 preguntas cada 200 ms se
/// cubre ese arranque sin que el usuario vea el QR.
pub(crate) const AUTH_PROBE_ATTEMPTS: u8 = 8;
pub(crate) const AUTH_PROBE_DELAY: Duration = Duration::from_millis(200);

/// The surface a scroll gesture happened on.
#[derive(Clone, Copy)]
pub(crate) enum ScrollSurface {
    ChatList,
    Search,
    Messages,
    ChatInfo,
    Settings,
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

/// Una foto de la conversacion que el visor puede mostrar: la que esta
/// abierta y las vecinas que ofrece el carrusel de miniaturas.
#[derive(Clone, Debug)]
pub(crate) struct ViewerItem {
    pub(crate) message_id: String,
    pub(crate) path: String,
    pub(crate) caption: String,
    pub(crate) sticker: bool,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) from_me: bool,
    /// The name of the sender of the picture, already resolved: the viewer
    /// never guesses it from the jid.
    pub(crate) sender_name: String,
    pub(crate) sender_avatar: String,
    pub(crate) timestamp_ms: i64,
}

/// The image the user is looking at, full size.
#[derive(Clone, Debug)]
pub(crate) struct ImageViewer {
    pub(crate) chat_id: String,
    pub(crate) message_id: String,
    pub(crate) path: String,
    pub(crate) caption: String,
    pub(crate) sticker: bool,
    /// The picture's own pixel size when the message carried it; zero when
    /// it did not, and the viewer then reads the file header instead.
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Every picture of the conversation, in the order of the messages: the
    /// arrows and the strip of thumbnails walk this list.
    pub(crate) gallery: Vec<ViewerItem>,
    /// The position of the picture on screen inside `gallery`.
    pub(crate) index: usize,
    /// The zoom of the picture over its natural fit.
    pub(crate) zoom: f32,
}

impl ImageViewer {
    /// The picture on screen.
    pub(crate) fn current(&self) -> Option<&ViewerItem> {
        self.gallery.get(self.index)
    }

    /// Shows another picture of the gallery and copies its data to the flat
    /// fields, which are the ones that measure the picture.
    pub(crate) fn show(&mut self, index: usize) {
        let Some(item) = self.gallery.get(index) else {
            return;
        };
        self.index = index;
        self.message_id = item.message_id.clone();
        self.path = item.path.clone();
        self.caption = item.caption.clone();
        self.sticker = item.sticker;
        self.width = item.width;
        self.height = item.height;
        self.zoom = 1.;
    }

    /// Whether there is a picture before or after this one.
    pub(crate) fn has_previous(&self) -> bool {
        self.index > 0
    }

    pub(crate) fn has_next(&self) -> bool {
        self.index + 1 < self.gallery.len()
    }
}

/// The multiple selection of the chat list: the menu of the column picks
/// several chats and then applies one action to all of them.
#[derive(Default)]
pub(crate) struct ChatSelection {
    pub(crate) ids: HashSet<String>,
}

/// The confirmation dialog to delete a message.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ConfirmDeleteMessage {
    pub(crate) message_id: String,
    pub(crate) delete_for_everyone: bool,
}

/// What the PIN dialog does: turn the lock on for the first time, or off.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum PinDialogMode {
    Enable,
    Disable,
}

/// The dialog of the PIN lock.
pub(crate) struct PinDialog {
    pub(crate) mode: PinDialogMode,
    pub(crate) pin: Entity<InputState>,
    pub(crate) repeat: Entity<InputState>,
    pub(crate) error: Option<String>,
}

/// The creation of a group: the name and the contacts the user marks in the
/// list.
pub(crate) struct CreateGroupState {
    pub(crate) name: Entity<InputState>,
    /// The transport jids of the participants that are marked.
    pub(crate) selected: HashSet<String>,
    /// The request is on its way: the button of the dialog waits.
    pub(crate) pending: bool,
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
    /// Whether the capture is paused: the clock and the wave stay still.
    pub(crate) paused: bool,
    /// When the current pause started, to take it out of the clock.
    pub(crate) paused_at: Option<Instant>,
    /// The time that the note spent paused, before the current pause.
    pub(crate) paused_total: Duration,
    /// The loudness of the latest ticks, from zero to one: the wave of the
    /// pill paints them from left to right.
    pub(crate) levels: Vec<f32>,
}

impl RecordingState {
    /// The length of the note so far, in seconds.
    pub(crate) fn elapsed_seconds(&self) -> f32 {
        let mut elapsed = self.started.elapsed().saturating_sub(self.paused_total);
        if let Some(paused_at) = self.paused_at {
            elapsed = elapsed.saturating_sub(paused_at.elapsed());
        }
        elapsed.as_secs_f32()
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

/// The menu that the right click opens over the picture of the viewer.
///
/// WhatsApp offers the copy of the picture where the pointer is: the viewer
/// works on the picture on screen, so the menu only remembers the place.
pub(crate) struct ImageContextMenu {
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
                title: i18n::t("Contacts"),
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
                title: i18n::t("Chats"),
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
                title: i18n::t("Messages"),
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

/// The picture of a call converted to what the atlas of the interface paints:
/// both ends use the fixed geometry of the call.
fn call_image(decoded: crate::call_video::Decoded) -> Option<Arc<gpui::RenderImage>> {
    crate::video::render_image(crate::video::VideoFrame {
        width: crate::call_video::WIDTH,
        height: crate::call_video::HEIGHT,
        bgra: decoded.bgra,
    })
}

/// Where the self-update of the app stands.
///
/// The phase drives both the lines of the settings panel and which button it
/// offers: the check, the download and the install are three steps that the
/// user can leave between.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum UpdatePhase {
    /// Nothing asked yet.
    #[default]
    Idle,
    /// The manifest is on its way.
    Checking,
    /// The published version is this one.
    UpToDate,
    /// A newer version is published.
    Available,
    /// The two files are on their way.
    Downloading,
    /// The files are on disk and the restart installs them.
    Ready,
    /// The files are moving into place.
    Installing,
    /// This platform has no package that the app can install itself.
    Unsupported,
    /// The last step failed; `update_error` says why.
    Failed,
}

/// The state of the application.
pub(crate) struct Whatfast {
    pub(crate) rpc: RpcClient,
    pub(crate) store: Store,
    /// The live call's audio pipes: the microphone stream and the playout
    /// sink. Both exist only while a call is connected.
    call_capture: Option<crate::call_audio::Capture>,
    call_playback: Option<Arc<crate::call_audio::Playback>>,
    /// The id whose audio the pump task is carrying.
    call_audio_id: Option<String>,
    /// The live call's video: the camera, the decoder of the peer and the
    /// decoder of the local preview. All three exist only with video on.
    call_camera: Option<crate::call_video::Camera>,
    call_decoder: Option<crate::call_video::Decoder>,
    local_decoder: Option<crate::call_video::Decoder>,
    /// The id whose video the pump task is carrying.
    call_video_id: Option<String>,
    /// Bumped whenever the video stops: a pump of an older generation exits
    /// at its next step, so two calls never paint each other.
    call_video_generation: u64,
    /// What the overlay paints: the pictures of both ends and the reason the
    /// video is not there, when it is not.
    pub(crate) call_video: crate::call_video::CallVideo,
    pub(crate) composer: Entity<InputState>,
    /// The pictures that the user pasted or picked and that wait for the
    /// button of the send, the same way the reference shows them over the
    /// field of the message.
    pub(crate) pending_images: PendingImages,
    pub(crate) chat_search_open: bool,
    pub(crate) chat_search_input: Entity<InputState>,
    pub(crate) chat_search_query: String,
    pub(crate) chat_search_matches: Vec<String>,
    pub(crate) chat_search_current: usize,
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
    pub(crate) image_context_menu: Option<ImageContextMenu>,
    /// Whether the row of quick reactions of the viewer is open.
    pub(crate) viewer_reactions_open: bool,
    /// Whether the card with the information of the message is open.
    pub(crate) viewer_info_open: bool,
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
    /// Whether the app asks the releases for a newer version when it starts.
    pub(crate) check_updates: bool,
    /// Where the self-update stands: the settings panel paints it.
    pub(crate) update_phase: UpdatePhase,
    /// The version of the release that is available, when there is one.
    pub(crate) update_version: String,
    /// The reason of the last failure, as the panel shows it.
    pub(crate) update_error: String,
    /// The release that is downloaded or ready to install.
    pub(crate) update_manifest: Option<crate::update::Manifest>,
    /// The folder with the two files of the release.
    pub(crate) update_staging: Option<PathBuf>,
    /// The bytes of the download that arrived, and the size of the file.
    pub(crate) update_received: u64,
    pub(crate) update_total: u64,
    /// When the files became ready. A click that started on the Download
    /// button must not install them by landing on the button that replaced
    /// it: the install waits a moment.
    pub(crate) update_ready_at: Option<Instant>,
    /// The air between rows and messages, mirrored for the hot path of the
    /// drawing.
    pub(crate) density: Density,
    /// The width of the thread of the messages, as the last frame measured
    /// it: the bubble asks for its widest part of it.
    pub(crate) message_wrap_width: gpui::Pixels,
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
    /// The token of the theme whose value the field of the editor holds.
    pub(crate) editing_token: Option<ThemeToken>,
    /// The hex value the user is typing for [`Self::editing_token`].
    pub(crate) theme_token_draft: String,
    /// The field of the value of a token.
    pub(crate) theme_token_input: Entity<InputState>,
    /// The queue of the toasts of the system.
    pub(crate) notifier: Notifier,
    pub(crate) chat_scroll: UniformListScrollHandle,
    pub(crate) chat_smooth_scroll: SmoothScrollState,
    pub(crate) message_scroll: VirtualListScrollHandle,
    pub(crate) message_smooth_scroll: SmoothScrollState,
    pub(crate) chat_info_scroll: ScrollHandle,
    pub(crate) chat_info_smooth_scroll: SmoothScrollState,
    pub(crate) settings_scroll: ScrollHandle,
    pub(crate) settings_smooth_scroll: SmoothScrollState,
    pub(crate) scroll_to_bottom_generation: Option<u64>,
    pub(crate) messages_scrolled_up: bool,
    pub(crate) last_event_sequence: u64,
    /// Cuantas veces se volvio a preguntar el estado de la cuenta antes de
    /// mandar al usuario a la pantalla del QR.
    pub(crate) auth_probe: u8,
    /// La cuenta ya dijo que hay un dispositivo vinculado.
    pub(crate) account_ready: bool,
    pub(crate) pending_prepend_anchor: Option<(String, Point<Pixels>)>,
    pub(crate) newer_load_failed: bool,
    pub(crate) message_generation: u64,
    pub(crate) sync_complete_reloaded: bool,
    pub(crate) confirming_logout: bool,
    pub(crate) avatar_attempted: HashSet<String>,
    pub(crate) avatar_retries: HashMap<String, u8>,
    pub(crate) participant_avatar_attempted: HashSet<String>,
    pub(crate) image_download_attempted: HashSet<(String, String)>,
    pub(crate) image_download_queue: VecDeque<(String, String)>,
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
    /// El video que el usuario pidio reproducir y que todavia no esta en el
    /// disco: arranca solo cuando el archivo llega.
    pub(crate) attachment_play_requested: Option<VideoKey>,
    /// El video que la ventana esta reproduciendo, cuando hay uno.
    pub(crate) video: VideoPlayback,
    /// Whether the video is being watched in the viewer that fills the window.
    pub(crate) video_fullscreen: bool,
    /// El hilo que decodifica el video.
    pub(crate) video_engine: VideoEngine,
    /// El volumen de los videos, de 0 a 1, y si estan en silencio: el video
    /// que se abre despues arranca con el mismo sonido.
    pub(crate) video_volume: f32,
    pub(crate) video_muted: bool,
    /// La barra del volumen del reproductor de video.
    pub(crate) volume_slider: Entity<SliderState>,
    /// Whether the tick that moves the frames of the video is running.
    pub(crate) video_tick: bool,
    /// The icon of the application in the tray, when the shell took it.
    pub(crate) tray: Option<crate::tray::Tray>,
    /// The width of the column of the chats, before the scale of the screen.
    pub(crate) sidebar_width: f32,
    /// Whether the column of the chats is hidden, so the conversation takes
    /// the whole window.
    pub(crate) sidebar_hidden: bool,
    /// Whether the menu of the three dots of the column of the chats is open.
    pub(crate) chat_list_menu: bool,
    /// Whether the dialog that confirms the logout is open.
    pub(crate) confirm_logout_open: bool,
    /// Confirmation dialog before deleting a message.
    pub(crate) confirm_delete_message: Option<ConfirmDeleteMessage>,
    /// Whether the card that explains the view-once messages is open.
    pub(crate) view_once_info_open: bool,
    /// The dialog that creates a group, when it is open.
    pub(crate) create_group: Option<CreateGroupState>,
    /// The multiple selection of the chat list, while the mode is on.
    pub(crate) selection: Option<ChatSelection>,
    /// Whether the window is covered by the PIN of the app lock.
    pub(crate) locked: bool,
    /// The digits of the PIN written in the lock screen.
    pub(crate) pin_buffer: String,
    /// The error of the last unlock attempt.
    pub(crate) lock_error: Option<String>,
    /// The dialog that turns the PIN on or off, when it is open.
    pub(crate) pin_dialog: Option<PinDialog>,
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
    /// The space the media cache takes on this device, when it was read.
    pub(crate) storage_usage: Option<whatfast_protocol::StorageUsageResponse>,
    /// Whether the media cache is being cleared.
    pub(crate) storage_clearing: bool,
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
    /// Whether the view of the agents is in front of the column of chats.
    ///
    /// The screen of the harnesses is a view of its own, like the states.
    pub(crate) agents_open: bool,
    /// When the statuses of the profile were read the last time.
    pub(crate) last_status_refresh: Instant,
    /// The column of icons at the left of the window.
    pub(crate) nav_rail: Entity<NavRail>,
    /// The strip of the open chats.
    pub(crate) tab_strip: Entity<TabStrip>,
    /// The editor of a picture, when it is open.
    pub(crate) media_editor: Option<Entity<MediaEditor>>,
    /// Target of media editor (Chat or Status).
    pub(crate) media_editor_target: MediaEditorTarget,
    /// Confirm delete status message id modal.
    pub(crate) confirm_delete_status: Option<String>,
    /// The panel of the newsletters.
    pub(crate) newsletter_panel: NewsletterPanel,
    /// The strip of the states of the contacts.
    pub(crate) status_strip: StatusStrip,
    /// The screen of the agents and the tray of their drafts.
    pub(crate) agents_panel: AgentsPanel,
    /// The fast mode of the chat: the question to an agent about the open
    /// conversation, and the answer it writes into the composer.
    pub(crate) quick_ask: QuickAsk,
    /// The drafts of the agents that wait for the open chat, newest first.
    ///
    /// They are read from `agents.db`, which belongs to the profile and not to
    /// the window: another process writes it while the conversation is open.
    pub(crate) agent_drafts: Vec<whatfast_auto::Draft>,
    /// When [`Whatfast::agent_drafts`] was read last, so the sweep of the
    /// second does not open the database on every step.
    pub(crate) agent_drafts_read: Instant,
    /// The automations of the profile: the tasks of the timer.
    pub(crate) automations_panel: AutomationsPanel,
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
        let check_updates = settings.check_updates;
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
                // El campo crece con el texto, de una a seis lineas: el
                // mensaje largo se ve entero mientras se escribe y el pegado
                // conserva los saltos de linea. En una sola linea, el
                // componente borraba los `\n` del portapapeles.
                .auto_grow(COMPOSER_MIN_ROWS, COMPOSER_MAX_ROWS)
                // Enter is decided in the subscription: the input has no
                // setter for this and the setting changes at runtime.
                .submit_on_enter(false)
                .clean_on_escape()
        });
        let emoji_search =
            cx.new(|cx| InputState::new(window, cx).placeholder(i18n::t("Search emoji")));
        let chat_search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(i18n::t("Find in chat"))
                .clean_on_escape()
        });
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                // port: the placeholder of the origin
                // (`tauri/src/lib/i18n.ts`, `sidebar.searchPlaceholder`).
                .placeholder(i18n::t("Search or start a new chat"))
                .clean_on_escape()
        });
        // The field of the value of a token of a theme, in the editor.
        let theme_token_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(i18n::t("#rrggbb"))
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
                tab_view.update(cx, |_this, cx| {
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
        // The card of the fast mode is built before the subscriptions: the
        // return key of its field sends the question, and the subscription is
        // the only way to hear it.
        let quick_ask = QuickAsk::new(window, cx);
        let quick_ask_input = quick_ask.input().clone();
        // La barra del volumen del video empieza con todo el sonido.
        let volume_slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(1.0)
                .step(0.05)
                .default_value(1.0)
        });
        let _subscriptions = vec![
            cx.subscribe_in(&composer, window, {
                let composer = composer.clone();
                move |this, _, event, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        // El campo tambien escribe la respuesta de un estado:
                        // ese texto no es un borrador del chat ni un aviso de
                        // que el usuario esta escribiendo en el.
                        if this.status_strip.viewer_open() {
                            cx.notify();
                        } else {
                            let text = composer.read(cx).value().to_string();
                            let composing = !text.trim().is_empty();
                            this.update_active_draft_text(text);
                            let picker_was_open = this.mention_picker.is_some();
                            this.refresh_mention_picker(cx);
                            this.signal_typing(composing);
                            // Abrir o cerrar el panel de las menciones cambia el
                            // pie del chat, y el campo se queda con el texto del
                            // cuadro anterior: el caracter que abrio el panel no
                            // se veia hasta el proximo clic. Se avisa al campo
                            // para que se pinte de nuevo.
                            if picker_was_open != this.mention_picker.is_some() {
                                this.composer.update(cx, |_, cx| cx.notify());
                            }
                            cx.notify();
                        }
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
            cx.subscribe_in(&chat_search_input, window, {
                let chat_search_input = chat_search_input.clone();
                move |this, _, event, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        let query = chat_search_input.read(cx).value().to_string();
                        this.set_chat_search_query(query, cx);
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
            // The value of a token of the editor of the themes.
            cx.subscribe_in(&theme_token_input, window, {
                let theme_token_input = theme_token_input.clone();
                move |this, _, event, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.theme_token_draft = theme_token_input.read(cx).value().to_string();
                        cx.notify();
                    }
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.apply_theme_token(window, cx);
                    }
                }
            }),
            // The return key of the card of the fast mode asks the question,
            // the same way the button of the card does.
            cx.subscribe_in(&quick_ask_input, window, {
                move |this, _, event, _, cx| {
                    if matches!(event, InputEvent::PressEnter { .. })
                        && this.quick_ask.is_open()
                        && !this.quick_ask.is_waiting()
                    {
                        this.submit_quick_ask(cx);
                        cx.notify();
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

        // The lock screen starts on when the profile has a PIN: the window
        // shows nothing until the PIN is written.
        let locked = !settings.app_lock_hash.is_empty();
        let mut this = Self {
            rpc,
            store: Store::default(),
            call_capture: None,
            call_playback: None,
            call_audio_id: None,
            call_camera: None,
            call_decoder: None,
            local_decoder: None,
            call_video_id: None,
            call_video_generation: 0,
            call_video: crate::call_video::CallVideo::default(),
            composer,
            pending_images: PendingImages::default(),
            chat_search_open: false,
            chat_search_input: chat_search_input.clone(),
            chat_search_query: String::new(),
            chat_search_matches: Vec::new(),
            chat_search_current: 0,
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
            image_context_menu: None,
            viewer_reactions_open: false,
            viewer_info_open: false,
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
            check_updates,
            update_phase: UpdatePhase::Idle,
            update_version: String::new(),
            update_error: String::new(),
            update_manifest: None,
            update_staging: None,
            update_received: 0,
            update_total: 0,
            update_ready_at: None,
            density,
            message_wrap_width: px(536.),
            enter_to_send,
            split_view,
            battery_saver,
            theme_id,
            custom_themes,
            editing_theme: None,
            editing_token: None,
            theme_token_draft: String::new(),
            theme_token_input,
            notifier: Notifier::new(),
            chat_scroll: UniformListScrollHandle::new(),
            chat_smooth_scroll: SmoothScrollState::default(),
            message_scroll: VirtualListScrollHandle::new(),
            message_smooth_scroll: SmoothScrollState::default(),
            chat_info_scroll: ScrollHandle::new(),
            chat_info_smooth_scroll: SmoothScrollState::default(),
            settings_scroll: ScrollHandle::new(),
            settings_smooth_scroll: SmoothScrollState::default(),
            scroll_to_bottom_generation: None,
            messages_scrolled_up: false,
            last_event_sequence: 0,
            auth_probe: 0,
            account_ready: false,
            pending_prepend_anchor: None,
            newer_load_failed: false,
            message_generation: 0,
            sync_complete_reloaded: false,
            confirming_logout: false,
            avatar_attempted: HashSet::new(),
            avatar_retries: HashMap::new(),
            participant_avatar_attempted: HashSet::new(),
            image_download_attempted: HashSet::new(),
            image_download_queue: VecDeque::new(),
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
            attachment_play_requested: None,
            video: VideoPlayback::default(),
            video_fullscreen: false,
            video_engine: VideoEngine::new(),
            video_volume: 1.0,
            video_muted: false,
            volume_slider: volume_slider.clone(),
            video_tick: false,
            tray: None,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            sidebar_hidden: false,
            chat_list_menu: false,
            confirm_logout_open: false,
            confirm_delete_message: None,
            view_once_info_open: false,
            create_group: None,
            selection: None,
            locked,
            pin_buffer: String::new(),
            lock_error: None,
            pin_dialog: None,
            sidebar_dragging: false,
            voice: VoiceState::default(),
            voice_audio: VoiceAudio::new(),
            voice_tick: false,
            recorder: VoiceRecorder::new(),
            recording: None,
            recording_tick: false,
            integrity_checking: false,
            storage_usage: None,
            storage_clearing: false,
            sticker_library_loading: false,
            newsletters_loading: false,
            selected_newsletter: None,
            states_open: false,
            channels_open: false,
            archived_open: false,
            agents_open: false,
            last_status_refresh: Instant::now(),
            nav_rail,
            tab_strip,
            media_editor: None,
            media_editor_target: MediaEditorTarget::Chat,
            confirm_delete_status: None,
            newsletter_panel: NewsletterPanel::new(window, cx),
            status_strip: StatusStrip::new(window, cx),
            agents_panel: AgentsPanel::new(window, cx),
            quick_ask,
            agent_drafts: Vec::new(),
            agent_drafts_read: Instant::now(),
            automations_panel: AutomationsPanel::new(window, cx),
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
        // El icono de la bandeja: cerrar la ventana la esconde y la
        // aplicacion sigue con la sesion del telefono.
        this.tray = crate::tray::Tray::install(window, &*cx);
        // La barra del volumen del video es la que manda: cada vez que el
        // usuario la mueve, el sonido del reproductor la sigue.
        let volume = cx.subscribe(&this.volume_slider, |this, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(value) = event {
                this.set_video_volume(value.end(), cx);
            }
        });
        this._subscriptions.push(volume);
        if startup_error.is_some() {
            this.store.screen = Screen::Fatal;
            this.store.fatal_error = Some(
                i18n::t("Whatfast could not start its local backend. Restart Whatfast.").into(),
            );
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
                ..Default::default()
            }),
            PendingRequest::Hello,
        );
        // A download that was waiting for a restart is offered again, and the
        // start asks the release for a newer version unless the profile
        // turned the check off.
        if let Some((manifest, folder)) = crate::update::ready() {
            this.update_version = manifest.version.clone();
            this.update_manifest = Some(manifest);
            this.update_staging = Some(folder);
            this.update_phase = UpdatePhase::Ready;
            this.update_ready_at = Some(Instant::now());
        }
        if this.check_updates && this.update_phase == UpdatePhase::Idle {
            this.start_update_check(true, cx);
        }
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
            ScrollSurface::Settings => &mut self.settings_smooth_scroll,
        }
    }

    pub(crate) fn smooth_scroll_handle(&self, surface: ScrollSurface) -> ScrollHandle {
        match surface {
            ScrollSurface::ChatList => self.chat_scroll.0.borrow().base_handle.clone(),
            ScrollSurface::Search => self.search_scroll.base_handle().clone(),
            ScrollSurface::Messages => self.message_scroll.base_handle().clone(),
            ScrollSurface::ChatInfo => self.chat_info_scroll.clone(),
            ScrollSurface::Settings => self.settings_scroll.clone(),
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
        // The drafts of the agents come from another process, which writes
        // them in `agents.db` while the conversation is open. The band reads
        // them again every few seconds, so a new draft shows up in the chat
        // without the user leaving it.
        if self.store.screen == Screen::Chats
            && self.store.selected_chat_id.is_some()
            && self.agent_drafts_read.elapsed() >= crate::chat_drafts::REFRESH
        {
            self.reload_agent_drafts();
        }
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
                        i18n::t("The local backend stopped responding during startup. Restart Whatfast to reconnect.").into(),
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
                        state.error =
                            Some(i18n::t("Loading chat info timed out. Try again.").into());
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
                            i18n::t("Sticker download timed out · Click to retry").into()
                        } else {
                            i18n::t("Photo download timed out · Click to retry").into()
                        },
                    );
                    self.pump_image_download_queue();
                }
                PendingRequest::Search { generation, .. }
                    if generation == self.search_generation =>
                {
                    self.search_error = Some(i18n::t("Search timed out. Try again.").into());
                }
                PendingRequest::SendText {
                    chat_id,
                    draft_text,
                    reply_to_message_id,
                    mentions,
                } => {
                    // A row of the tray that timed out is marked failed, and
                    // its text does not come back to the field of the message:
                    // the row of the tray is where the user tries again.
                    if self.agents_panel.fail_send(&chat_id, &draft_text) {
                        self.store.toast_error =
                            Some(i18n::t("Sending timed out. Try again.").into());
                        continue;
                    }
                    self.restore_failed_draft(
                        &chat_id,
                        &draft_text,
                        reply_to_message_id,
                        &mentions,
                        window,
                        cx,
                    );
                    self.store.toast_error = Some(
                        i18n::t("Sending timed out. Your draft was restored; try again.").into(),
                    );
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
                    self.store.toast_error = Some(
                        i18n::t("Sending timed out. Your draft was restored; try again.").into(),
                    );
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
                    if self.store.selected_chat_id.as_deref() != Some(chat_id.as_str()) {
                        self.store.restore_unread(&chat_id, previous_unread);
                        self.store.last_mark_read_id = None;
                    }
                }
                PendingRequest::Messages { prepend: true, .. } => {
                    self.pending_prepend_anchor = None;
                    self.store.toast_error =
                        Some(i18n::t("Loading messages timed out. Try again.").into());
                }
                PendingRequest::MessagesAfter { .. } => {
                    self.newer_load_failed = true;
                    self.store.toast_error =
                        Some(i18n::t("Loading newer messages timed out. Try again.").into());
                }
                PendingRequest::Logout => {
                    self.confirming_logout = false;
                    self.store.toast_error = Some(i18n::t("Logout timed out. Try again.").into());
                }
                PendingRequest::CreateGroup => {
                    if let Some(state) = self.create_group.as_mut() {
                        state.pending = false;
                    }
                    self.store.toast_error = Some(i18n::t("The group could not be created").into());
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
                        Some(i18n::t("Archiving the chat timed out. Try again.").into());
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
                        Some(i18n::t("Deleting the message timed out. Try again.").into());
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
                        i18n::t(
                            "Editing the message timed out. Your draft was restored; try again.",
                        )
                        .into(),
                    );
                }
                PendingRequest::OlderHistory { chat_id } => {
                    report_failed_request(
                        &mut self.store,
                        self.rpc.request_history_coverage(chat_id.clone()),
                    );
                    self.store.toast_error =
                        Some(i18n::t("Asking for older history timed out. Try again.").into());
                }
                PendingRequest::Reconnect => {
                    self.store.connection = ConnectionState::Offline;
                    self.store.connection_detail = i18n::t("Reconnect failed").into();
                    self.store.toast_error =
                        Some(i18n::t("Reconnect timed out. Try again.").into());
                }
                PendingRequest::IntegrityStatus | PendingRequest::RepairLocalCache => {
                    self.integrity_checking = false;
                    self.store.toast_error =
                        Some(i18n::t("Checking the local storage timed out. Try again.").into());
                }
                PendingRequest::StorageUsage | PendingRequest::ClearMediaCache => {
                    self.storage_clearing = false;
                    self.store.toast_error = Some(
                        i18n::t("Reading the space of the media timed out. Try again.").into(),
                    );
                }
                PendingRequest::Stickers => {
                    self.sticker_library_loading = false;
                    self.store.toast_error =
                        Some(i18n::t("Loading the stickers timed out. Try again.").into());
                }
                PendingRequest::Newsletters => {
                    self.newsletters_loading = false;
                    self.store.toast_error =
                        Some(i18n::t("Loading the newsletters timed out. Try again.").into());
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
                        i18n::t("Downloading the file timed out · Click to retry").into(),
                    );
                }
                PendingRequest::AgentAsk { .. } => {
                    self.quick_ask
                        .fail(i18n::t("The agent did not answer in time.").to_string());
                }
                PendingRequest::Schedules
                | PendingRequest::SchedulePut
                | PendingRequest::SchedulePause { .. }
                | PendingRequest::ScheduleRemove { .. } => {
                    self.automations_panel
                        .fail(i18n::t("The automations did not answer in time.").to_string());
                }
                _ => {
                    self.store.toast_error =
                        Some(i18n::t("Backend request timed out. Try again.").into());
                }
            }
        }
    }

    pub(crate) fn set_chat_search_query(&mut self, query: String, cx: &mut Context<Self>) {
        self.chat_search_query = query.clone();
        let query_lower = query.trim().to_lowercase();
        if query_lower.is_empty() {
            self.chat_search_matches.clear();
            self.chat_search_current = 0;
            self.search_target_message_id = None;
            cx.notify();
            return;
        }
        let matches: Vec<String> = self
            .store
            .messages
            .iter()
            .filter(|m| {
                crate::attachments::bubble_text(m).to_lowercase().contains(&query_lower)
            })
            .map(|m| m.id.clone())
            .collect();
        self.chat_search_matches = matches;
        if !self.chat_search_matches.is_empty() {
            self.chat_search_current = 1;
            let target_id = self.chat_search_matches[0].clone();
            self.scroll_to_message(&target_id);
            self.search_target_message_id = Some(target_id);
        } else {
            self.chat_search_current = 0;
            self.search_target_message_id = None;
        }
        cx.notify();
    }

    pub(crate) fn chat_search_next(&mut self, cx: &mut Context<Self>) {
        if self.chat_search_matches.is_empty() {
            return;
        }
        if self.chat_search_current >= self.chat_search_matches.len() {
            self.chat_search_current = 1;
        } else {
            self.chat_search_current += 1;
        }
        let target_id = self.chat_search_matches[self.chat_search_current - 1].clone();
        self.scroll_to_message(&target_id);
        self.search_target_message_id = Some(target_id);
        cx.notify();
    }

    pub(crate) fn chat_search_prev(&mut self, cx: &mut Context<Self>) {
        if self.chat_search_matches.is_empty() {
            return;
        }
        if self.chat_search_current <= 1 {
            self.chat_search_current = self.chat_search_matches.len();
        } else {
            self.chat_search_current -= 1;
        }
        let target_id = self.chat_search_matches[self.chat_search_current - 1].clone();
        self.scroll_to_message(&target_id);
        self.search_target_message_id = Some(target_id);
        cx.notify();
    }

    pub(crate) fn close_chat_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chat_search_open = false;
        self.chat_search_query.clear();
        self.chat_search_matches.clear();
        self.chat_search_current = 0;
        self.search_target_message_id = None;
        self.chat_search_input.update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
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
        if self.store.selected_chat_id.is_some() {
            self.chat_search_open = true;
            self.chat_search_input.update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
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
            RpcIncoming::Event(event) => self.handle_event(event, window, cx),
            RpcIncoming::Exited(_) => {
                self.store.connection = ConnectionState::Failed;
                let message = i18n::t("Backend stopped. Restart Whatfast to reconnect.");
                self.store.connection_detail = message.into();
                self.store.toast_error = Some(message.into());
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
        // The tray of the drafts sends its text with the same RPC as the
        // composer. A row that is waiting closes here: a failed send is taken
        // and a good one falls through, so the message of the chat still lands
        // in the store.
        if let Some(sent) = self.agents_panel.take_send_response(&pending, &result)
            && !sent
        {
            return;
        }
        if let rpc_response::Result::Error(error) = result {
            if let PendingRequest::Search { generation, .. } = &pending {
                if *generation == self.search_generation {
                    self.search_error = Some(i18n::t("The search failed. Try again.").into());
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
                self.store.toast_error = Some(i18n::t("The message could not be found.").into());
                return;
            }
            if matches!(&pending, PendingRequest::MessagesAfter { .. }) {
                self.newer_load_failed = true;
            }
            // The fast mode and the automations keep their own line of error:
            // a toast at the corner does not say which card it belongs to.
            if let PendingRequest::AgentAsk { .. } = &pending {
                self.quick_ask
                    .fail(i18n::t("The agent did not answer. Try again.").to_string());
                cx.notify();
                return;
            }
            if self.automations_panel.take_error(&pending, &error.message) {
                cx.notify();
                return;
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
                    state.error = Some(
                        i18n::t("The chat information could not be loaded. Try again.").into(),
                    );
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
                        i18n::t("Couldn't load sticker · Click to retry").into()
                    } else if sticker {
                        i18n::t("That sticker is no longer available").into()
                    } else if error.retryable {
                        i18n::t("Couldn't load photo · Click to retry").into()
                    } else {
                        i18n::t("Photo is no longer available").into()
                    },
                );
                self.pump_image_download_queue();
                return;
            }
            if let PendingRequest::MarkRead {
                chat_id,
                previous_unread,
            } = &pending
            {
                if self.store.selected_chat_id.as_deref() != Some(chat_id.as_str()) {
                    self.store.restore_unread(chat_id, *previous_unread);
                    self.store.last_mark_read_id = None;
                }
            }
            // The changes below are optimistic: the client shows them before
            // the backend answers, so a failure has to put them back.
            if matches!(pending, PendingRequest::CreateGroup) {
                if let Some(state) = self.create_group.as_mut() {
                    state.pending = false;
                }
                self.store.toast_error = Some(i18n::t("The group could not be created").into());
                return;
            }
            if let PendingRequest::SetChatPin {
                chat_id,
                previous_pinned,
            } = &pending
            {
                self.store.set_chat_pinned(chat_id, *previous_pinned);
                self.store.toast_error =
                    Some(i18n::t("The chat could not be pinned. Try again.").into());
                return;
            }
            if let PendingRequest::SetChatArchive {
                chat_id,
                previous_archived,
            } = &pending
            {
                self.store.set_chat_archived(chat_id, *previous_archived);
                self.store.toast_error =
                    Some(i18n::t("The chat could not be archived. Try again.").into());
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
                self.store.toast_error =
                    Some(i18n::t("The chat could not be muted. Try again.").into());
                return;
            }
            if let PendingRequest::SetChatBlock {
                chat_id,
                previous_blocked,
            } = &pending
            {
                self.store.set_chat_blocked(chat_id, *previous_blocked);
                self.store.toast_error =
                    Some(i18n::t("The chat could not be blocked. Try again.").into());
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
                self.store.toast_error =
                    Some(i18n::t("The message could not be deleted. Try again.").into());
                return;
            }
            if let PendingRequest::VotePoll {
                chat_id,
                poll_message_id,
                generation,
            } = &pending
            {
                self.rollback_poll_vote(chat_id, poll_message_id, *generation);
                self.store.toast_error =
                    Some(i18n::t("The vote could not be sent. Try again.").into());
                return;
            }
            if let PendingRequest::EditMessage {
                chat_id,
                message_id,
                draft_text,
            } = &pending
            {
                self.restore_failed_edit(chat_id, message_id, draft_text, window, cx);
                self.store.toast_error =
                    Some(i18n::t("The message could not be edited. Try again.").into());
                return;
            }
            if let PendingRequest::OlderHistory { chat_id } = &pending {
                // port: the origin reads the coverage again when the request
                // of more history fails, because the failure moves it.
                report_failed_request(
                    &mut self.store,
                    self.rpc.request_history_coverage(chat_id.clone()),
                );
                self.store.toast_error =
                    Some(i18n::t("The old messages could not be loaded. Try again.").into());
                return;
            }
            if let PendingRequest::Reconnect = &pending {
                self.store.connection = ConnectionState::Offline;
                self.store.connection_detail = i18n::t("Reconnect failed").into();
                self.store.toast_error =
                    Some(i18n::t("WhatsApp could not connect. Try again.").into());
                return;
            }
            if let PendingRequest::IntegrityStatus | PendingRequest::RepairLocalCache = &pending {
                self.integrity_checking = false;
            }
            if let PendingRequest::StorageUsage | PendingRequest::ClearMediaCache = &pending {
                self.storage_clearing = false;
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
                        i18n::t("Couldn't download the file · Click to retry").into()
                    } else {
                        i18n::t("That file is no longer available").into()
                    },
                );
                return;
            }
            if matches!(
                &pending,
                PendingRequest::CallPlace | PendingRequest::CallAccept
            ) {
                // A call that the backend refused leaves no bar behind.
                self.store.call = None;
                self.stop_call_audio();
                self.stop_call_video();
            }
            if matches!(
                &pending,
                PendingRequest::CallAudio | PendingRequest::CallVideo
            ) {
                // Fire-and-forget frames: a lost one is not an error to show.
                return;
            }
            if matches!(&pending, PendingRequest::Logout) {
                self.confirming_logout = false;
                self.store.screen = Screen::Fatal;
                self.store.fatal_error = Some(
                    i18n::t("Logout stopped because the local account data could not be cleared. Restart Whatfast.")
                        .into(),
                );
                return;
            }
            if matches!(&pending, PendingRequest::Messages { prepend: true, .. }) {
                self.pending_prepend_anchor = None;
            }
            if matches!(pending, PendingRequest::Hello) {
                self.store.fatal_error =
                    Some(i18n::t("Whatfast could not start. Restart Whatfast.").into());
                self.store.screen = Screen::Fatal;
            } else {
                self.store.toast_error = Some(i18n::t("Something went wrong. Try again.").into());
            }
            return;
        }
        match (pending, result) {
            (PendingRequest::Hello, rpc_response::Result::Hello(hello)) => {
                if hello.protocol_version != PROTOCOL_VERSION {
                    self.store.screen = Screen::Fatal;
                    self.store.fatal_error = Some(
                        i18n::t(
                            "Whatfast and its backend are different versions. Restart Whatfast.",
                        )
                        .into(),
                    );
                } else {
                    self.request(
                        rpc_request::Request::GetAuthState(
                            whatfast_protocol::GetAuthStateRequest {},
                        ),
                        PendingRequest::Auth,
                    );
                    // La lista de conversaciones sale de la base local, no de
                    // la sesion de WhatsApp: se pide ya mismo, en paralelo,
                    // para que la ventana muestre el chat sin esperar al
                    // handshake.
                    self.load_chats(String::new());
                    // El sidecar abre la sesion recien cuando alguien se lo
                    // pide. Sin esto la cuenta vinculada tarda en conectar y
                    // la ventana cree que no hay dispositivo. Un dispositivo
                    // sin vincular responde que no arranco nada, sin efectos.
                    self.request_reconnect();
                }
            }
            (PendingRequest::Auth, rpc_response::Result::AuthState(auth)) => {
                if auth.paired {
                    // Solo una cuenta vinculada informa el estado de la
                    // conexion. Un "sin vincular" es la sesion todavia sin
                    // abrir: su `Offline` no es una conexion perdida y no
                    // tiene que pisar el "conectando" del arranque.
                    self.store.connection = auth.connection_state();
                    self.auth_probe = 0;
                    self.account_ready = true;
                    // Si la lista de la base local ya llego, no se vuelve a
                    // la pantalla de sincronizacion: el usuario ya esta en
                    // sus conversaciones.
                    if self.store.screen != Screen::Chats {
                        self.store.screen = Screen::Syncing;
                        self.load_chats(String::new());
                    }
                    // port: the origin reads the status of the sync with the
                    // state of the account, in the same round trip.
                    self.load_sync_status();
                } else if self.auth_probe < AUTH_PROBE_ATTEMPTS && !self.account_ready {
                    // El sidecar abre la sesion despues del saludo, y
                    // mientras no la tiene contesta "sin vincular": si se le
                    // cree a la primera, la ventana muestra el QR por un
                    // segundo y despues entra sola. Se vuelve a preguntar
                    // antes de mandar al usuario a vincular.
                    self.auth_probe += 1;
                    self.schedule_auth_probe(window, cx);
                } else {
                    self.store.connection = auth.connection_state();
                    self.request_pairing();
                }
            }
            (PendingRequest::Chats { cursor }, rpc_response::Result::ListChats(page)) => {
                let newest_chat_id = if cursor.is_empty() {
                    page.chats.first().map(|chat| chat.id.clone())
                } else {
                    None
                };
                // Una cuenta sin conversaciones guardadas no pasa a la lista:
                // la pantalla sigue en arranque hasta que la cuenta diga si
                // hay un dispositivo vinculado o no.
                let has_chats = !page.chats.is_empty();
                self.store.replace_chat_page(
                    &cursor,
                    page.chats,
                    page.total_count,
                    page.next_cursor,
                );
                if has_chats
                    || !cursor.is_empty()
                    || self.account_ready
                    || self.store.screen != Screen::Starting
                {
                    self.store.screen = Screen::Chats;
                }
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
                let nothing_unread = page.first_unread_message_id.is_empty();
                if nothing_unread {
                    self.scroll_to_bottom_generation = Some(generation);
                } else {
                    self.pending_top_scroll_id = Some(page.first_unread_message_id);
                }
                // Mark read on open unconditionally so the badge clears
                // even if list render gate has not fired.
                self.mark_read();
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
                        i18n::t("That sticker is no longer available")
                    } else {
                        i18n::t("Photo is no longer available")
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
                    self.pump_image_download_queue();
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
            (PendingRequest::StorageUsage, rpc_response::Result::StorageUsage(response)) => {
                self.storage_usage = Some(response);
            }
            (PendingRequest::ClearMediaCache, rpc_response::Result::ClearMediaCache(response)) => {
                let freed = response.freed_bytes;
                let files = response.deleted_files;
                self.storage_usage = response.usage;
                self.storage_clearing = false;
                // The files of the avatars went with the rest of the cache:
                // the paths of this session would paint a hole, so they go
                // back to empty and the next paint asks for the picture.
                self.store.forget_avatars();
                self.avatar_attempted.clear();
                self.participant_avatar_attempted.clear();
                // The rows that pointed at the deleted files were reset by
                // the sidecar: the next paint of a bubble asks for the file
                // again, so the open conversation reloads its messages.
                if self.store.selected_chat_id.is_some() {
                    self.refresh_latest();
                }
                let freed = crate::attachments::format_bytes(freed);
                if files == 0 {
                    self.store.toast_info = Some(i18n::t("There was nothing to clear.").into());
                } else if freed.is_empty() {
                    let files = files.to_string();
                    self.store.toast_info = Some(i18n::t_args(
                        "The media cache was cleared. {files} files deleted.",
                        &[("files", files.as_str())],
                    ));
                } else {
                    self.store.toast_info = Some(i18n::t_args(
                        "The media cache was cleared. {size} freed.",
                        &[("size", freed.as_str())],
                    ));
                }
            }
            (PendingRequest::Reconnect, rpc_response::Result::Reconnect(reconnect)) => {
                // El arranque tambien pide reconectar: solo un rechazo real
                // deja su motivo a la vista, y nunca pisa una conexion viva.
                if !reconnect.started && self.store.connection != ConnectionState::Connected {
                    self.store.connection_detail =
                        i18n::t("WhatsApp is not linked on this device").into();
                }
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
                    // El video que esperaba este archivo no queda colgado en
                    // "descargando": se avisa en su propia burbuja.
                    if self.attachment_play_requested.as_ref() == Some(&key) {
                        self.attachment_play_requested = None;
                        self.video.waiting = false;
                        self.video.error = Some(i18n::t("That file is no longer available").into());
                    }
                    self.attachment_failures
                        .insert(key, i18n::t("That file is no longer available").into());
                } else {
                    self.attachment_failures.remove(&key);
                    self.store.set_message_attachment_path(
                        &chat_id,
                        &message_id,
                        response.local_path.clone(),
                    );
                    // El video que el usuario mando a reproducir antes de que
                    // el archivo estuviera: arranca ahora, en la burbuja.
                    if self.attachment_play_requested.as_ref() == Some(&key) {
                        self.attachment_play_requested = None;
                        self.video.waiting = false;
                        self.video.error = None;
                        self.video.path = response.local_path.clone();
                        self.video.active = true;
                        self.video.playing = true;
                        self.video_engine
                            .play(key.clone(), Path::new(&response.local_path));
                        self.start_video_tick(window, cx);
                    }
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
                // La foto de los que subieron un estado se pide apenas llega
                // la lista: el tope de descargas del cliente deja salir unas
                // pocas y el resto se queda con la foto del chat.
                let senders: Vec<String> = response
                    .groups
                    .iter()
                    .map(|group| group.sender_id.clone())
                    .collect();
                self.store.set_statuses(response.chat_id, response.groups);
                for sender in senders {
                    self.load_participant_avatar(sender);
                }
                cx.notify();
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
            (PendingRequest::AgentAsk { .. }, rpc_response::Result::AgentAsk(answer)) => {
                // The answer goes to the composer of the chat it was asked
                // about: the user reads it and decides, so nothing is sent.
                if let Some((chat_id, text)) = self.quick_ask.take_answer(answer.text) {
                    if self.store.selected_chat_id.as_deref() == Some(chat_id.as_str()) {
                        self.apply_agent_answer(text, window, cx);
                    } else if let Some(draft) = self.chat_drafts.get_mut(&chat_id) {
                        // The chat changed while the agent was writing: the
                        // answer waits in the draft of its own conversation.
                        draft.text = text;
                    } else {
                        self.chat_drafts.insert(
                            chat_id,
                            ChatDraft {
                                text,
                                ..ChatDraft::default()
                            },
                        );
                    }
                }
            }
            (PendingRequest::Schedules, rpc_response::Result::ScheduleList(list)) => {
                self.automations_panel.set_schedules(list.schedules);
            }
            (PendingRequest::SchedulePut, rpc_response::Result::SchedulePut(response)) => {
                if self.automations_panel.take_put(&response) {
                    self.automations_panel.close_form(window, cx);
                    crate::automations_panel::load(self);
                }
            }
            (
                PendingRequest::SchedulePause { .. },
                rpc_response::Result::SchedulePause(response),
            ) => {
                if self.automations_panel.take_pause(&response) {
                    crate::automations_panel::load(self);
                }
            }
            (
                PendingRequest::ScheduleRemove { .. },
                rpc_response::Result::ScheduleRemove(response),
            ) => {
                self.automations_panel.take_remove(&response);
                crate::automations_panel::load(self);
            }
            (PendingRequest::CallPlace, rpc_response::Result::CallPlace(response)) => {
                // A placed call has no id until this answer: the updates and
                // the hang up of the backend carry the one it gives here.
                match self.store.call.as_mut() {
                    Some(call) => call.placed(response.call_id),
                    // The user hung up before the id arrived: the call would
                    // keep ringing on the other phone with nobody to end it.
                    None => self.request(
                        rpc_request::Request::CallHangup(whatfast_protocol::CallHangupRequest {
                            call_id: response.call_id,
                        }),
                        PendingRequest::CallHangup,
                    ),
                }
            }
            (PendingRequest::CallAccept, rpc_response::Result::CallAccept(_)) => {
                if let Some(call) = self.store.call.as_mut() {
                    call.phase = "connecting".into();
                }
            }
            (PendingRequest::CreateGroup, rpc_response::Result::CreateGroup(response)) => {
                if let Some(state) = self.create_group.as_mut() {
                    state.pending = false;
                }
                let Some(chat) = response.chat else {
                    self.store.toast_error = Some(i18n::t("The group could not be created").into());
                    return;
                };
                let chat_id = chat.id.clone();
                self.store.upsert_chat(chat);
                self.create_group = None;
                self.open_chat(chat_id, window, cx);
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

    // ------------------------------------------------------------------
    // Calls (v22)
    // ------------------------------------------------------------------

    /// Places a call to the selected conversation, with video when asked.
    pub(crate) fn call_place(&mut self, chat_id: String, video: bool) {
        if self.store.call.is_some() {
            return;
        }
        // Video needs the encoder and the decoder of this machine: without
        // them the call goes out as a voice call instead of failing late.
        let video = video && crate::call_video::ffmpeg().is_some();
        // The sidecar calls the transport jid: the local id of a chat is not
        // always the address that WhatsApp routes the call through.
        let transport = self
            .store
            .chat(&chat_id)
            .map(|chat| chat.transport_jid.clone())
            .filter(|jid| !jid.is_empty())
            .unwrap_or_else(|| chat_id.clone());
        self.store.call = Some(whatfast_model::model::CallState {
            chat_id: chat_id.clone(),
            phase: "placing".into(),
            video,
            ..whatfast_model::model::CallState::default()
        });
        self.request(
            rpc_request::Request::CallPlace(whatfast_protocol::CallPlaceRequest {
                chat_id: transport,
                video,
            }),
            PendingRequest::CallPlace,
        );
    }

    pub(crate) fn call_accept(&mut self) {
        let Some(call) = self.store.call.as_mut() else {
            return;
        };
        // Answering with video needs the offer to carry it and the codec of
        // this machine to be there.
        let video = call.video && crate::call_video::ffmpeg().is_some();
        call.video = video;
        let call_id = call.call_id.clone();
        // The bar changes now, not when the next update of the backend lands.
        call.phase = "connecting".into();
        self.request(
            rpc_request::Request::CallAccept(whatfast_protocol::CallAcceptRequest {
                call_id,
                video,
            }),
            PendingRequest::CallAccept,
        );
    }

    pub(crate) fn call_reject(&mut self) {
        let Some(call) = self.store.call.take() else {
            return;
        };
        self.stop_call_video();
        self.request(
            rpc_request::Request::CallReject(whatfast_protocol::CallRejectRequest {
                call_id: call.call_id,
            }),
            PendingRequest::CallReject,
        );
    }

    pub(crate) fn call_hangup(&mut self) {
        let Some(call) = self.store.call.clone() else {
            return;
        };
        // A call that has no id yet is still being placed: the answer of the
        // backend ends it, so no request goes with an empty id.
        if !call.call_id.is_empty() {
            self.request(
                rpc_request::Request::CallHangup(whatfast_protocol::CallHangupRequest {
                    call_id: call.call_id,
                }),
                PendingRequest::CallHangup,
            );
        }
        self.store.call = None;
        self.stop_call_audio();
        self.stop_call_video();
    }

    pub(crate) fn call_toggle_mute(&mut self) {
        let Some(call) = self.store.call.clone() else {
            return;
        };
        let muted = !call.muted;
        self.request(
            rpc_request::Request::CallMute(whatfast_protocol::CallMuteRequest {
                call_id: call.call_id.clone(),
                muted,
            }),
            PendingRequest::CallMute,
        );
        if let Some(live) = self.store.call.as_mut()
            && live.call_id == call.call_id
        {
            live.muted = muted;
        }
        if let Some(capture) = &self.call_capture {
            capture.set_muted(muted);
        }
    }

    /// Turns the camera of a video call off, or back on.
    ///
    /// Off drops the encoder and stops sending: the peer keeps the last
    /// picture, which is what a camera that is off looks like.
    pub(crate) fn call_toggle_camera(&mut self) {
        let Some(call) = self.store.call.as_ref() else {
            return;
        };
        if !call.video {
            return;
        }
        if self.call_video.camera_on {
            self.call_video.camera_on = false;
            self.call_camera = None; // dropping it kills ffmpeg
            self.local_decoder = None;
            self.call_video.local = None;
            return;
        }
        self.call_video.camera_on = true;
        match crate::call_video::Camera::open() {
            Ok(camera) => self.call_camera = Some(camera),
            Err(error) => {
                eprintln!("call video: {error}");
                self.call_video.error = Some(i18n::t("The camera could not open.").into());
            }
        }
    }

    /// Opens the microphone and the speaker for the live call, and starts
    /// the pump task that carries the microphone frames to the bridge.
    fn start_call_audio(&mut self, call_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.call_capture.is_some() {
            return; // already flowing
        }
        let capture = match crate::call_audio::Capture::open() {
            Ok(capture) => capture,
            Err(error) => {
                self.store.toast_error =
                    Some(i18n::t("The microphone could not open for the call.").into());
                eprintln!("call audio: {error}");
                return;
            }
        };
        // The mute of the last call does not follow the new one: the capture
        // starts at the state the bar shows.
        let muted = self.store.call.as_ref().is_some_and(|call| call.muted);
        capture.set_muted(muted);
        let playback = match crate::call_audio::Playback::open() {
            Ok(playback) => Arc::new(playback),
            Err(error) => {
                self.store.toast_error =
                    Some(i18n::t("The speaker could not open for the call.").into());
                eprintln!("call audio: {error}");
                return;
            }
        };
        self.call_playback = Some(Arc::clone(&playback));
        self.call_capture = Some(capture);
        self.call_audio_id = Some(call_id.to_owned());
        cx.spawn_in(window, async move |this, cx| {
            loop {
                Timer::after(std::time::Duration::from_millis(20)).await;
                let Ok(live) = this.update(cx, |app, _| {
                    let Some(call_id) = app.call_audio_id.clone() else {
                        return false;
                    };
                    // The frames are drained into an owned list first, so
                    // the mutable borrow of the app ends before the sends.
                    let mut batch: Vec<Vec<u8>> = Vec::new();
                    if let Some(capture) = app.call_capture.as_ref() {
                        while let Some(frame) = capture.try_take() {
                            batch.push(frame.iter().flat_map(|s| s.to_le_bytes()).collect());
                        }
                    }
                    for pcm in batch {
                        app.request(
                            rpc_request::Request::CallAudio(whatfast_protocol::CallAudioRequest {
                                call_id: call_id.clone(),
                                pcm,
                            }),
                            PendingRequest::CallAudio,
                        );
                    }
                    app.call_capture.is_some()
                }) else {
                    return; // the window closed
                };
                if !live {
                    return;
                }
            }
        })
        .detach();
    }

    fn stop_call_audio(&mut self) {
        self.call_audio_id = None;
        if let Some(capture) = &self.call_capture {
            // The mute is a static flag that outlives the capture: it does
            // not follow the next call.
            capture.set_muted(false);
        }
        self.call_capture = None; // dropping closes the microphone and ends the pump
        if let Some(playback) = &self.call_playback {
            playback.flush();
        }
        self.call_playback = None;
    }

    /// Opens the camera and the two decoders of a video call.
    ///
    /// The local preview decodes the very stream that leaves toward the
    /// bridge: one encode, one extra decode, and no second open of the camera.
    fn start_call_video(&mut self, call_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let wants_video = self.store.call.as_ref().is_some_and(|call| call.video);
        if !wants_video || self.call_video_id.as_deref() == Some(call_id) {
            return;
        }
        self.call_video_id = Some(call_id.to_owned());
        self.call_video.on = true;
        self.call_video.error = None;
        if crate::call_video::ffmpeg().is_none() {
            self.call_video.on = false;
            self.call_video.error =
                Some(i18n::t("Video calls need ffmpeg, which is not installed.").into());
            if let Some(call) = self.store.call.as_mut() {
                call.video = false;
            }
            return;
        }
        match crate::call_video::Camera::open() {
            Ok(camera) => {
                match crate::call_video::Decoder::open() {
                    Ok(decoder) => self.local_decoder = Some(decoder),
                    Err(error) => eprintln!("call video: {error}"),
                }
                self.call_camera = Some(camera);
                self.call_video.camera_on = true;
            }
            Err(error) => {
                eprintln!("call video: {error}");
                self.call_video.camera_on = false;
                self.call_video.error = Some(i18n::t("The camera could not open.").into());
            }
        }
        self.start_call_video_pump(window, cx);
    }

    /// The clock of the video: the camera out, the pictures in.
    fn start_call_video_pump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.call_video_generation = self.call_video_generation.wrapping_add(1);
        let generation = self.call_video_generation;
        cx.spawn_in(window, async move |this, cx| {
            loop {
                Timer::after(std::time::Duration::from_millis(20)).await;
                let Ok(live) = this.update(cx, |app, cx| {
                    if app.call_video_generation != generation {
                        return false;
                    }
                    app.pump_call_video(cx);
                    app.call_video_id.is_some()
                }) else {
                    return; // the window closed
                };
                if !live {
                    return;
                }
            }
        })
        .detach();
    }

    /// One step of the video pump: the camera toward the bridge and the
    /// decoded pictures toward the overlay.
    fn pump_call_video(&mut self, cx: &mut Context<Self>) {
        let Some(call_id) = self.call_video_id.clone() else {
            return;
        };
        if let Some(camera) = self.call_camera.take() {
            let mut batch: Vec<Vec<u8>> = Vec::new();
            while let Some(chunk) = camera.try_take() {
                batch.push(chunk);
            }
            let failed = camera.failed();
            let ended = camera.ended();
            if failed {
                self.call_video.camera_on = false;
                self.call_video.error = Some(i18n::t("The camera could not open.").into());
            } else if ended {
                self.call_video.camera_on = false;
                self.call_video.error = Some(i18n::t("The camera stopped.").into());
            } else {
                self.call_camera = Some(camera);
            }
            for chunk in batch {
                if let Some(local) = &self.local_decoder {
                    local.push(chunk.clone());
                }
                self.request(
                    rpc_request::Request::CallVideo(whatfast_protocol::CallVideoRequest {
                        call_id: call_id.clone(),
                        data: chunk,
                    }),
                    PendingRequest::CallVideo,
                );
            }
        }
        let mut painted = false;
        // A decoder that died gets a new process: the encoder repeats its
        // headers, so the next keyframe resynchronizes it.
        if self.local_decoder.is_none() && self.call_camera.is_some() {
            match crate::call_video::Decoder::open() {
                Ok(decoder) => self.local_decoder = Some(decoder),
                Err(error) => eprintln!("call video: {error}"),
            }
        }
        if let Some(decoder) = self.local_decoder.as_mut() {
            let alive = decoder.alive();
            while let Some(decoded) = decoder.try_take() {
                if let Some(image) = call_image(decoded) {
                    self.call_video.local = Some(image);
                    painted = true;
                }
            }
            if !alive {
                self.local_decoder = None;
            }
        }
        if let Some(decoder) = self.call_decoder.as_mut() {
            let alive = decoder.alive();
            while let Some(decoded) = decoder.try_take() {
                if let Some(image) = call_image(decoded) {
                    self.call_video.remote = Some(image);
                    painted = true;
                }
            }
            if !alive {
                self.call_decoder = None;
            }
        }
        if painted {
            cx.notify();
        }
    }

    fn stop_call_video(&mut self) {
        self.call_video_id = None;
        // The pump of this call stops at its next step.
        self.call_video_generation = self.call_video_generation.wrapping_add(1);
        self.call_camera = None;
        self.call_decoder = None;
        self.local_decoder = None;
        self.call_video.reset();
    }

    pub(crate) fn handle_event(
        &mut self,
        event: BackendEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
                // The detail of the backend is a technical line in English
                // (`Disconnected(ConnectionClosed)`, for example). The header
                // says the state of the connection with its own label.
                self.store.connection_detail.clear();
                if state == ConnectionState::LoggedOut {
                    if self.rpc.logout_pending() {
                        self.store.connection_detail =
                            i18n::t("Logged out; securely clearing the local account data…").into();
                    } else {
                        self.begin_pairing();
                    }
                } else if state == ConnectionState::Connected {
                    // La cuenta quedo conectada: si la ventana todavia no
                    // sabe que hay sesion, se le pregunta ahora. Es el caso
                    // en que el QR aparece un segundo y se va solo.
                    if (matches!(self.store.screen, Screen::Pairing | Screen::Starting)
                        || !self.account_ready)
                        && !self
                            .rpc
                            .pending_requests()
                            .any(|pending| matches!(pending, PendingRequest::Auth))
                    {
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
                    self.store.connection_detail = i18n::t_args(
                        "Importing older history… {chats} chats · {messages} messages",
                        &[
                            ("chats", &self.store.sync_chats.to_string()),
                            ("messages", &self.store.sync_messages.to_string()),
                        ],
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
            // v22 calls: the offer rings the overlay, the updates move the
            // phase, and the audio frames ride to the speaker.
            Some(backend_event::Event::CallOffer(offer)) => {
                // One call at a time, also on the desktop: a second offer
                // while one is up rings busy on the phone anyway.
                if self.store.call.is_none() {
                    self.store.call = Some(whatfast_model::model::CallState {
                        call_id: offer.call_id,
                        chat_id: offer.chat_id,
                        phase: "incoming".into(),
                        muted: false,
                        video: offer.video,
                        incoming: true,
                    });
                    self.notifier.show(Note {
                        title: i18n::t("Incoming WhatsApp call").into(),
                        body: i18n::t("Open Whatfast to answer.").into(),
                    });
                }
            }
            Some(backend_event::Event::CallUpdate(update)) => {
                let ours = self
                    .store
                    .call
                    .as_ref()
                    .is_some_and(|call| call.call_id == update.call_id);
                if ours {
                    if update.phase == "failed" || update.phase == "ended" {
                        // The media never came up: the bar goes away and the
                        // user reads why, instead of a call stuck in a phase.
                        self.store.call = None;
                        self.stop_call_audio();
                        self.stop_call_video();
                        self.store.toast_error =
                            Some(i18n::t("The call could not connect.").into());
                    } else {
                        if let Some(call) = self.store.call.as_mut() {
                            call.phase = update.phase.clone();
                        }
                        if update.phase == "active" {
                            self.start_call_audio(&update.call_id, window, cx);
                            self.start_call_video(&update.call_id, window, cx);
                        }
                    }
                    cx.notify();
                }
            }
            Some(backend_event::Event::CallEnded(ended)) => {
                let ended_call = self
                    .store
                    .call
                    .as_ref()
                    .is_some_and(|call| call.call_id == ended.call_id);
                // Only the call that owns the media stops it: a stale end of
                // another call does not cut the one that is up.
                let ended_audio = self.call_audio_id.as_deref() == Some(ended.call_id.as_str());
                let ended_video = self.call_video_id.as_deref() == Some(ended.call_id.as_str());
                if ended_call {
                    self.store.call = None;
                }
                if ended_call || ended_audio {
                    self.stop_call_audio();
                }
                if ended_call || ended_video {
                    self.stop_call_video();
                }
                cx.notify();
            }
            Some(backend_event::Event::CallAudioOut(audio)) => {
                if let Some(playback) = &self.call_playback
                    && audio.call_id
                        == self
                            .store
                            .call
                            .as_ref()
                            .map(|call| call.call_id.clone())
                            .unwrap_or_default()
                {
                    let frame: Vec<i16> = audio
                        .pcm
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|pair| i16::from_le_bytes(*pair))
                        .collect();
                    playback.play(frame);
                }
            }
            Some(backend_event::Event::CallVideoOut(video)) => {
                // The frames of a call that is up arrive before the camera
                // warms up, so the id of the pump is not the gate: the call
                // itself is, and it must carry video.
                let belongs = self
                    .store
                    .call
                    .as_ref()
                    .is_some_and(|call| call.call_id == video.call_id && call.video);
                if !belongs {
                    return;
                }
                // A decoder starts on a keyframe with its parameter sets, and
                // restarts the same way when the process died.
                let dead = self
                    .call_decoder
                    .as_mut()
                    .is_some_and(|decoder| !decoder.alive());
                if dead {
                    self.call_decoder = None;
                }
                if self.call_decoder.is_none() && video.keyframe {
                    match crate::call_video::Decoder::open() {
                        Ok(decoder) => self.call_decoder = Some(decoder),
                        Err(error) => {
                            eprintln!("call video: {error}");
                            self.call_video.error =
                                Some(i18n::t("The video could not start.").into());
                        }
                    }
                }
                if let Some(decoder) = &self.call_decoder {
                    decoder.push(video.au);
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

    pub(crate) fn pump_image_download_queue(&mut self) {
        while self.rpc.pending_media_count() < 4 {
            if let Some((chat_id, message_id)) = self.image_download_queue.pop_front() {
                let key = (chat_id.clone(), message_id.clone());
                if !self.image_download_attempted.contains(&key) {
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
            } else {
                break;
            }
        }
    }

    pub(crate) fn load_message_image(&mut self, chat_id: String, message_id: String) {
        let key = (chat_id.clone(), message_id.clone());
        if self.image_download_attempted.contains(&key) {
            return;
        }
        if self.rpc.pending_media_count() >= 4 {
            if !self.image_download_queue.iter().any(|k| k == &key) {
                self.image_download_queue.push_back(key);
            }
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

    pub(crate) fn open_image_viewer(&mut self, viewer: ImageViewer) {
        self.image_viewer = Some(viewer);
        self.image_context_menu = None;
        self.viewer_reactions_open = false;
        self.viewer_info_open = false;
        self.emoji_target = None;
        self.reaction_details = None;
    }

    /// Reacts to the picture on screen with one of the quick reactions of the
    /// viewer, the way the smile of WhatsApp does.
    pub(crate) fn react_to_viewed_image(&mut self, emoji: &str) {
        let Some(viewer) = self.image_viewer.as_ref() else {
            return;
        };
        let chat_id = viewer.chat_id.clone();
        let message_id = viewer.message_id.clone();
        self.send_reaction_request(
            chat_id,
            message_id,
            emoji.to_string(),
            Uuid::new_v4().to_string(),
            0,
        );
        self.viewer_reactions_open = false;
    }

    /// Opens the viewer of a picture of the conversation with every picture
    /// of the conversation in the gallery: the arrows and the strip of
    /// thumbnails walk them, the way WhatsApp does it.
    pub(crate) fn open_chat_image(&mut self, chat_id: String, message_id: String) {
        let mut gallery: Vec<ViewerItem> = self
            .store
            .messages
            .iter()
            .filter_map(crate::render::viewer_item)
            .collect();
        let Some(index) = gallery
            .iter()
            .position(|item| item.message_id == message_id)
        else {
            return;
        };
        // The header of the viewer names the sender: a direct chat without a
        // saved name falls back to the title of the conversation, and the
        // avatar falls back to the avatar of the chat.
        let (title, avatar) = self
            .store
            .chat(&chat_id)
            .map(|chat| (chat.title.clone(), chat.avatar_path.clone()))
            .unwrap_or_default();
        for item in &mut gallery {
            if item.from_me {
                item.sender_name = i18n::t("You").to_string();
            } else if item.sender_name.is_empty() {
                item.sender_name = title.clone();
            }
            if item.sender_avatar.is_empty() {
                item.sender_avatar = avatar.clone();
            }
        }
        let mut viewer = ImageViewer {
            chat_id,
            message_id,
            path: String::new(),
            caption: String::new(),
            sticker: false,
            width: 0,
            height: 0,
            gallery,
            index: 0,
            zoom: 1.,
        };
        viewer.show(index);
        self.open_image_viewer(viewer);
    }

    /// Moves the viewer to the previous or the next picture of the gallery.
    pub(crate) fn step_image_viewer(&mut self, forward: bool) {
        let Some(viewer) = self.image_viewer.as_mut() else {
            return;
        };
        let target = if forward {
            viewer.index + 1
        } else {
            viewer.index.saturating_sub(1)
        };
        viewer.show(target);
    }

    /// Zooms the picture of the viewer over its natural fit.
    pub(crate) fn zoom_image_viewer(&mut self, factor: f32) {
        if let Some(viewer) = self.image_viewer.as_mut() {
            viewer.zoom = (viewer.zoom * factor).clamp(0.5, 4.);
        }
    }

    /// Copies the picture on screen to the clipboard, the way WhatsApp does
    /// from the right click of the viewer.
    pub(crate) fn copy_viewed_image(&mut self, cx: &mut Context<Self>) {
        let Some(viewer) = self.image_viewer.as_ref() else {
            return;
        };
        let path = PathBuf::from(&viewer.path);
        let copied = std::fs::read(&path)
            .ok()
            .and_then(|bytes| {
                crate::helpers::image_file_format(&bytes).map(|format| (format, bytes))
            })
            .map(|(format, bytes)| gpui::Image::from_bytes(format, bytes));
        match copied {
            Some(image) => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_image(&image));
                self.store.toast_info = Some(i18n::t("Photo copied").into());
                self.clear_toast_info_later(cx);
            }
            None => {
                self.store.toast_error = Some(i18n::t("The photo could not be copied.").into());
            }
        }
        self.image_context_menu = None;
    }

    /// Opens a file with the default application of the system: the "Open
    /// with" of the menu of the viewer.
    pub(crate) fn open_file_externally(&mut self, path: &str, cx: &mut Context<Self>) {
        let path = Path::new(path);
        if path.as_os_str().is_empty() || !path.is_file() {
            self.store.toast_error = Some(i18n::t("That file is no longer available").into());
            self.image_context_menu = None;
            return;
        }
        cx.open_with_system(path);
        self.image_context_menu = None;
    }

    /// Saves the picture on screen where the user chooses: the "Save as…" of
    /// the viewer opens the dialog of the system, the way WhatsApp does.
    pub(crate) fn save_viewed_image_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(viewer) = self.image_viewer.as_ref() else {
            return;
        };
        let source = PathBuf::from(&viewer.path);
        if !source.is_file() {
            self.store.toast_error = Some(i18n::t("That file is no longer available").into());
            self.image_context_menu = None;
            cx.notify();
            return;
        }
        let caption = viewer.caption.trim().to_string();
        let stem = {
            let caption = sanitize_file_name(&caption.chars().take(60).collect::<String>());
            if caption.is_empty() {
                "photo".to_string()
            } else {
                caption
            }
        };
        let extension = source
            .extension()
            .map(|extension| extension.to_string_lossy().into_owned())
            .unwrap_or_else(|| "jpg".to_string());
        let suggested = format!("{stem}.{extension}");
        let directory = self.download_directory();
        let selected = cx.prompt_for_new_path(&directory, Some(&suggested));
        self.image_context_menu = None;
        let view = cx.entity().downgrade();
        cx.spawn_in(window, async move |_, window| {
            let Ok(Ok(Some(destination))) = selected.await else {
                return;
            };
            let copy_source = source.clone();
            let copy_destination = destination.clone();
            let result = window
                .background_spawn(async move {
                    // La copia muere en un `.part`: el archivo elegido aparece
                    // entero o no aparece, nunca a medias.
                    let partial = copy_destination
                        .with_file_name(format!(".whatfast-{}.part", uuid::Uuid::new_v4()));
                    let result = std::fs::copy(&copy_source, &partial)
                        .and_then(|_| std::fs::rename(&partial, &copy_destination));
                    if result.is_err() {
                        let _ = std::fs::remove_file(&partial);
                    }
                    result.map(|_| copy_destination)
                })
                .await;
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
                .ok();
        })
        .detach();
    }

    /// La carpeta donde arranca el dialogo de "Guardar como…": la de las
    /// descargas configurada, o la del sistema.
    fn download_directory(&self) -> PathBuf {
        let configured = PathBuf::from(self.settings.download_dir.trim());
        if configured.is_dir() {
            return configured;
        }
        if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
            let downloads = home.join("Downloads");
            if downloads.is_dir() {
                return downloads;
            }
            if home.is_dir() {
                return home;
            }
        }
        std::env::temp_dir()
    }

    /// Closes the viewer and leaves the conversation on the message of the
    /// photo: the "Go to message" of the menu.
    pub(crate) fn go_to_viewed_message(&mut self) {
        let Some(viewer) = self.image_viewer.as_ref() else {
            return;
        };
        let message_id = viewer.message_id.clone();
        self.image_viewer = None;
        self.image_context_menu = None;
        self.scroll_to_message(&message_id);
    }

    /// Searches the caption of the photo on the web; with no caption it opens
    /// the images page of the search engine.
    pub(crate) fn search_viewed_image(&mut self, cx: &mut Context<Self>) {
        let Some(viewer) = self.image_viewer.as_ref() else {
            return;
        };
        let caption = viewer.caption.trim().to_string();
        let url = if caption.is_empty() {
            "https://www.google.com/imghp".to_string()
        } else {
            format!(
                "https://www.google.com/search?q={}",
                crate::helpers::percent_encode(&caption)
            )
        };
        cx.open_url(&url);
        self.image_context_menu = None;
    }

    pub(crate) fn advance_status_viewer(&mut self, cx: &mut Context<Self>) {
        let state = self.status_strip.state();
        let groups = self.store.statuses().to_vec();
        crate::statuses::move_viewer(self, &state, true, &groups, cx);
    }

    pub(crate) fn previous_status_viewer(&mut self, cx: &mut Context<Self>) {
        let state = self.status_strip.state();
        let groups = self.store.statuses().to_vec();
        crate::statuses::move_viewer(self, &state, false, &groups, cx);
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

    /// The size of the text of the messages: the one of the interface times
    /// the scale of the profile.
    pub(crate) fn message_font_size(&self, cx: &App) -> Pixels {
        px(f32::from(cx.theme().font_size) * self.settings.message_font_scale)
    }

    /// The widest bubble of the thread.
    ///
    /// Without a preference it is the width of the reference; with one it is
    /// that part of the thread, never narrower than a short line of text.
    pub(crate) fn bubble_max_width(&self) -> Pixels {
        match self.settings.bubble_width() {
            // Never narrower than an attachment card: a bubble that is
            // narrower than its content pushes the layout of the thread.
            Some(part) => px((f32::from(self.message_wrap_width) * part).max(320. * self.ui_scale)),
            None => px(560.),
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

    /// Sets whether the app asks for a newer release when it starts.
    pub(crate) fn set_check_updates(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.check_updates == enabled {
            return;
        }
        self.check_updates = enabled;
        self.settings.check_updates = enabled;
        self.save_settings();
        cx.notify();
    }

    /// Asks the release for a newer version.
    ///
    /// The request lives in its own thread: the check waits on the network,
    /// and the window keeps painting while it does. `automatic` is the check
    /// of the start: it only tells the user when it finds something.
    pub(crate) fn start_update_check(&mut self, automatic: bool, cx: &mut Context<Self>) {
        if matches!(
            self.update_phase,
            UpdatePhase::Checking
                | UpdatePhase::Downloading
                | UpdatePhase::Installing
                | UpdatePhase::Ready
        ) {
            return;
        }
        if !crate::update::enabled() {
            self.update_phase = UpdatePhase::Failed;
            self.update_error = i18n::t("The automatic update is off in this copy.").to_string();
            cx.notify();
            return;
        }
        let Some(manifest_name) = crate::update::platform_manifest() else {
            self.update_phase = UpdatePhase::Unsupported;
            cx.notify();
            return;
        };
        self.update_phase = UpdatePhase::Checking;
        self.update_error.clear();
        self.update_received = 0;
        self.update_total = 0;
        cx.notify();
        let (sender, receiver) = async_channel::unbounded();
        let base = crate::update::base_url();
        let name = manifest_name.to_string();
        let started = std::thread::Builder::new()
            .name("whatfast-update-check".to_string())
            .spawn(move || {
                let event = match crate::update::check(&base, &name) {
                    Ok(Some(manifest)) => crate::update::Event::Found(manifest),
                    Ok(None) => crate::update::Event::Current,
                    Err(error) => crate::update::Event::Failed(error),
                };
                let _ = sender.send_blocking(event);
            });
        if started.is_err() {
            self.update_phase = UpdatePhase::Failed;
            self.update_error = i18n::t("The update could not be checked.").to_string();
            cx.notify();
            return;
        }
        self.watch_update(receiver, automatic, cx);
    }

    /// Follows the events of the thread of the update until it ends.
    fn watch_update(
        &mut self,
        receiver: async_channel::Receiver<crate::update::Event>,
        automatic: bool,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            while let Ok(event) = receiver.recv().await {
                let Ok(keep) =
                    this.update(cx, |this, cx| this.on_update_event(event, automatic, cx))
                else {
                    break;
                };
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }

    /// Paints one note of the update thread in the state of the panel.
    fn on_update_event(
        &mut self,
        event: crate::update::Event,
        automatic: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        match event {
            crate::update::Event::Found(manifest) => {
                self.update_version = manifest.version.clone();
                self.update_manifest = Some(manifest);
                self.update_phase = UpdatePhase::Available;
                self.update_error.clear();
                if automatic {
                    self.store.toast_info = Some(i18n::t_args(
                        "Whatfast {version} is available. Open Settings, Updates.",
                        &[("version", &self.update_version)],
                    ));
                    self.clear_toast_info_later(cx);
                }
            }
            crate::update::Event::Current => {
                self.update_phase = UpdatePhase::UpToDate;
                self.update_error.clear();
            }
            crate::update::Event::Failed(error) => {
                self.update_phase = UpdatePhase::Failed;
                self.update_error = error;
            }
            crate::update::Event::Progress { received, total } => {
                self.update_phase = UpdatePhase::Downloading;
                self.update_received = received;
                self.update_total = total;
            }
            crate::update::Event::Downloaded(folder) => {
                self.update_phase = UpdatePhase::Ready;
                self.update_staging = Some(folder);
                self.update_ready_at = Some(Instant::now());
                self.update_received = 0;
                self.update_total = 0;
                self.store.toast_info = Some(i18n::t_args(
                    "Whatfast {version} is ready. Restart from Settings, Updates.",
                    &[("version", &self.update_version)],
                ));
                self.clear_toast_info_later(cx);
            }
        }
        cx.notify();
        true
    }

    /// Downloads the two files of the release that the check found.
    pub(crate) fn start_update_download(&mut self, cx: &mut Context<Self>) {
        if self.update_phase != UpdatePhase::Available {
            return;
        }
        let Some(manifest) = self.update_manifest.clone() else {
            return;
        };
        self.update_phase = UpdatePhase::Downloading;
        self.update_error.clear();
        self.update_received = 0;
        self.update_total = 0;
        cx.notify();
        let (sender, receiver) = async_channel::unbounded();
        let base = crate::update::base_url();
        let staging = crate::update::staging_root();
        let started = std::thread::Builder::new()
            .name("whatfast-update-download".to_string())
            .spawn(move || {
                let mut last = 0u64;
                let progress_sender = sender.clone();
                let mut progress = move |received: u64, total: u64| {
                    // One note per half a megabyte keeps the interface from
                    // repainting for every chunk of 64 KiB.
                    if received.saturating_sub(last) >= 512 * 1024
                        || (total > 0 && received == total)
                    {
                        last = received;
                        let _ = progress_sender
                            .send_blocking(crate::update::Event::Progress { received, total });
                    }
                };
                let event = match crate::update::download(&manifest, &base, &staging, &mut progress)
                {
                    Ok(folder) => crate::update::Event::Downloaded(folder),
                    Err(error) => crate::update::Event::Failed(error),
                };
                let _ = sender.send_blocking(event);
            });
        if started.is_err() {
            self.update_phase = UpdatePhase::Failed;
            self.update_error = i18n::t("The update could not be downloaded.").to_string();
            cx.notify();
            return;
        }
        self.watch_update(receiver, false, cx);
    }

    /// Replaces the two executables with the downloaded ones and starts the
    /// new copy.
    pub(crate) fn install_update(&mut self, cx: &mut Context<Self>) {
        if self.update_phase != UpdatePhase::Ready {
            return;
        }
        // A click that began on the Download button can end on the button
        // that replaces it when the download is fast. The install asks for a
        // deliberate second click.
        if self
            .update_ready_at
            .is_some_and(|ready| ready.elapsed() < Duration::from_millis(1500))
        {
            return;
        }
        let (Some(manifest), Some(staging)) =
            (self.update_manifest.clone(), self.update_staging.clone())
        else {
            return;
        };
        self.update_phase = UpdatePhase::Installing;
        self.update_error.clear();
        cx.notify();
        // The sidecar runs the executable of the backend: it has to end
        // before the files move. The interface is about to exit, so a
        // backend that does not answer is not a reason to stay.
        self.rpc.stop_backend();
        let exe = std::env::current_exe().unwrap_or_default();
        match crate::update::install(&manifest, &staging, &exe) {
            Ok(installed) => {
                // The download is spent: a restart must never offer the same
                // files again, even if the new build reports the old version.
                let _ = std::fs::remove_dir_all(&staging);
                // The new copy takes the lock that this one releases, and the
                // exit is immediate: this process must never write the lock
                // file again.
                crate::instance::release();
                let _ = std::process::Command::new(&installed).spawn();
                std::process::exit(0);
            }
            Err(error) => {
                self.update_phase = UpdatePhase::Failed;
                self.update_error = error;
                cx.notify();
            }
        }
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
        cx.notify();
    }

    /// Sets the radius of the bubbles; `None` keeps the one of the theme.
    pub(crate) fn set_bubble_radius(&mut self, radius: Option<f32>, cx: &mut Context<Self>) {
        self.settings.bubble_radius = radius.unwrap_or(-1.0);
        self.settings.normalize();
        self.store.invalidate_all_message_heights();
        self.save_settings();
        cx.notify();
    }

    /// Sets the widest bubble, as a part of the width of the thread;
    /// `None` keeps the width of the reference.
    pub(crate) fn set_bubble_width(&mut self, width: Option<f32>, cx: &mut Context<Self>) {
        self.settings.bubble_width = width.unwrap_or(-1.0);
        self.settings.normalize();
        self.store.invalidate_all_message_heights();
        self.save_settings();
        cx.notify();
    }

    /// Sets the size of the text of the messages, as a part of the one of
    /// the interface.
    pub(crate) fn set_message_font_scale(&mut self, scale: f32, cx: &mut Context<Self>) {
        self.settings.message_font_scale = scale;
        self.settings.normalize();
        self.store.invalidate_all_message_heights();
        self.save_settings();
        cx.notify();
    }

    /// Chooses where the background of the conversation comes from.
    pub(crate) fn set_chat_background(
        &mut self,
        background: ChatBackground,
        cx: &mut Context<Self>,
    ) {
        if self.settings.chat_background == background {
            return;
        }
        self.settings.chat_background = background;
        self.save_settings();
        cx.notify();
    }

    /// Paints the plain background of the conversation; an empty value asks
    /// the theme.
    pub(crate) fn set_chat_background_color(&mut self, color: String, cx: &mut Context<Self>) {
        self.settings.chat_background_color = color;
        self.settings.normalize();
        self.save_settings();
        cx.notify();
    }

    /// Asks the user for the image of the background.
    ///
    /// The image is copied into the profile, so moving the profile or
    /// importing a theme never breaks it: the setting keeps a name relative
    /// to the profile, never the path the user picked.
    pub(crate) fn choose_chat_wallpaper(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(i18n::t("Choose an image for the background").into()),
        });
        let view = cx.entity();
        cx.spawn_in(window, async move |_, window| {
            let paths = selected.await.ok()?.ok()??;
            let path = paths.into_iter().next()?;
            window
                .update(|window, cx| {
                    view.update(cx, |this, cx| {
                        this.copy_chat_wallpaper(&path, window, cx);
                        cx.notify();
                    })
                })
                .ok()
        })
        .detach();
    }

    /// Copies an image into the profile and makes it the background.
    fn copy_chat_wallpaper(&mut self, path: &Path, _window: &mut Window, cx: &mut Context<Self>) {
        let extension = path
            .extension()
            .map(|extension| extension.to_string_lossy().to_lowercase())
            .filter(|extension| matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp"))
            .unwrap_or_else(|| "png".to_string());
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.store.toast_error =
                    Some(format!("{}: {error}", i18n::t("Could not read the image")));
                return;
            }
        };
        // The name is the hash of the content: the same picture picked
        // twice is one file, and the name never depends on the folder.
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&bytes, &mut hasher);
        let name = format!("{:016x}.{extension}", std::hash::Hasher::finish(&hasher));
        let directory = whatfast_model::paths::config_dir().join("wallpapers");
        if let Err(error) = std::fs::create_dir_all(&directory)
            .and_then(|()| std::fs::write(directory.join(&name), &bytes))
        {
            self.store.toast_error =
                Some(format!("{}: {error}", i18n::t("Could not copy the image")));
            return;
        }
        self.settings.chat_wallpaper = format!("wallpapers/{name}");
        self.save_settings();
        cx.notify();
    }

    /// How solid the image of the background is.
    pub(crate) fn set_chat_wallpaper_opacity(&mut self, opacity: f32, cx: &mut Context<Self>) {
        self.settings.chat_wallpaper_opacity = opacity;
        self.settings.normalize();
        self.save_settings();
        cx.notify();
    }

    /// Takes the image of the background out, back to the pattern.
    pub(crate) fn clear_chat_wallpaper(&mut self, cx: &mut Context<Self>) {
        self.settings.chat_wallpaper.clear();
        self.save_settings();
        cx.notify();
    }

    /// Copies a theme under a new name, so the user paints a copy instead of
    /// starting from an empty palette.
    pub(crate) fn duplicate_theme(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source = whatfast_core::theme::builtin_themes()
            .iter()
            .find(|theme| theme.id == id)
            .cloned()
            .or_else(|| {
                self.custom_themes
                    .iter()
                    .find(|theme| theme.id == id)
                    .cloned()
            });
        let Some(source) = source else {
            return;
        };
        let count = self.custom_themes.len() + 1;
        let number = count.to_string();
        let name = i18n::t_args("My theme {count}", &[("count", &number)]);
        let mut theme = whatfast_core::theme::clone_theme(&source, &name);
        theme.id = format!("custom-{}", Uuid::new_v4());
        let id = theme.id.clone();
        self.custom_themes.push(theme);
        self.theme_id = id.clone();
        self.editing_theme = Some(id);
        self.editing_token = None;
        self.apply_active_theme(window, cx);
        self.save_settings();
    }

    /// Writes the JSON of a theme where the user picks.
    pub(crate) fn export_theme(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let theme = whatfast_core::theme::builtin_themes()
            .iter()
            .find(|theme| theme.id == id)
            .cloned()
            .or_else(|| {
                self.custom_themes
                    .iter()
                    .find(|theme| theme.id == id)
                    .cloned()
            });
        let Some(theme) = theme else {
            return;
        };
        let json = whatfast_core::theme::export_theme(&theme);
        let directory = if self.settings.download_dir.trim().is_empty() {
            whatfast_model::paths::config_dir()
        } else {
            PathBuf::from(self.settings.download_dir.trim())
        };
        let suggested = format!("{}.json", theme.id);
        let selected = cx.prompt_for_new_path(&directory, Some(&suggested));
        let view = cx.entity();
        cx.spawn_in(window, async move |_, window| {
            let path = selected.await.ok()?.ok()??;
            let failure = std::fs::write(&path, json)
                .err()
                .map(|error| error.to_string());
            window
                .update(|_, cx| {
                    view.update(cx, |this, cx| {
                        if let Some(error) = failure {
                            this.store.toast_error =
                                Some(format!("{}: {error}", i18n::t("Could not save the theme")));
                        }
                        cx.notify();
                    })
                })
                .ok()
        })
        .detach();
    }

    /// Reads a theme file of the user and adds it to the catalog.
    ///
    /// The file is normalized on the way in: a token that is not a plain
    /// value falls back to the one of the base theme, so a theme file cannot
    /// paint something the window cannot read.
    pub(crate) fn import_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(i18n::t("Choose a theme file").into()),
        });
        let view = cx.entity();
        cx.spawn_in(window, async move |_, window| {
            let path = selected.await.ok()?.ok()??.into_iter().next()?;
            let text = std::fs::read_to_string(&path).ok()?;
            let value: serde_json::Value = serde_json::from_str(&text).ok()?;
            window
                .update(|window, cx| {
                    view.update(cx, |this, cx| {
                        this.add_imported_theme(value, window, cx);
                        cx.notify();
                    })
                })
                .ok()
        })
        .detach();
    }

    /// Adds the theme of a file, with an id of its own.
    fn add_imported_theme(
        &mut self,
        value: serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mut theme) = whatfast_core::theme::normalize_theme(&value) else {
            self.store.toast_error = Some(i18n::t("That file is not a theme").to_string());
            return;
        };
        theme.id = format!("custom-{}", Uuid::new_v4());
        theme.builtin = None;
        let id = theme.id.clone();
        self.custom_themes.push(theme);
        self.theme_id = id.clone();
        self.editing_theme = Some(id);
        self.editing_token = None;
        self.apply_active_theme(window, cx);
        self.save_settings();
    }

    /// Starts editing one token of the theme being edited.
    pub(crate) fn begin_editing_token(
        &mut self,
        token: ThemeToken,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let theme = self.editing_theme.as_deref().and_then(|id| {
            self.custom_themes
                .iter()
                .find(|theme| theme.id == id)
                .cloned()
        });
        let Some(theme) = theme else {
            return;
        };
        let value = theme.tokens.get(token).to_string();
        self.editing_token = Some(token);
        self.theme_token_draft = value.clone();
        self.theme_token_input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
        });
        cx.notify();
    }

    /// Writes the value of the field into the token being edited.
    ///
    /// The value is checked before it lands: a theme file cannot carry a
    /// declaration, and the same rule holds for the field.
    pub(crate) fn apply_theme_token(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(token), Some(id)) = (self.editing_token, self.editing_theme.clone()) else {
            return;
        };
        let value = self.theme_token_draft.trim().to_string();
        if value.is_empty() {
            return;
        }
        if let Some(theme) = self.custom_themes.iter_mut().find(|theme| theme.id == id) {
            theme.tokens.set(token, &value);
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
        let previous_unread = self.store.mark_chat_read_locally(&chat_id).unwrap_or(0);
        cx.notify();
        self.request(
            rpc_request::Request::MarkRead(whatfast_protocol::MarkReadRequest {
                chat_id: chat_id.clone(),
                through_message_id: String::new(),
            }),
            PendingRequest::MarkRead {
                chat_id,
                previous_unread,
            },
        );
    }

    /// Marks every conversation with unread messages as read: the action of
    /// the menu of the list of chats.
    pub(crate) fn mark_all_chats_read(&mut self, cx: &mut Context<Self>) {
        self.chat_list_menu = false;
        let unread: Vec<String> = self
            .store
            .chats
            .iter()
            .filter(|chat| chat.unread_count > 0)
            .map(|chat| chat.id.clone())
            .collect();
        if unread.is_empty() {
            self.store.toast_info = Some(i18n::t("All chats are read").into());
            self.clear_toast_info_later(cx);
            cx.notify();
            return;
        }
        for chat_id in unread {
            self.mark_chat_read(chat_id, cx);
        }
        cx.notify();
    }

    /// Opens the dialog that confirms the logout of the menu of the chats.
    pub(crate) fn ask_logout(&mut self, cx: &mut Context<Self>) {
        self.chat_list_menu = false;
        self.confirm_logout_open = true;
        cx.notify();
    }

    /// Checks the PIN of the lock screen and opens the window.
    pub(crate) fn try_unlock(&mut self, cx: &mut Context<Self>) {
        let pin = self.pin_buffer.clone();
        if crate::app_lock::verify(
            &pin,
            &self.settings.app_lock_salt,
            &self.settings.app_lock_hash,
        ) {
            self.locked = false;
            self.lock_error = None;
        } else {
            self.lock_error = Some(i18n::t("Wrong PIN. Try again.").to_string());
        }
        self.pin_buffer.clear();
        cx.notify();
    }

    /// Takes one key of the lock screen: the digits, backspace and enter. It
    /// returns whether the key belonged to the screen.
    pub(crate) fn lock_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        match key {
            "enter" => self.try_unlock(cx),
            "backspace" => {
                self.pin_buffer.pop();
                self.lock_error = None;
                cx.notify();
            }
            "escape" => {}
            _ => {
                let mut characters = key.chars();
                let (Some(character), None) = (characters.next(), characters.next()) else {
                    return false;
                };
                if !character.is_ascii_digit()
                    || self.pin_buffer.len() >= crate::app_lock::MAX_PIN_LENGTH
                {
                    return false;
                }
                self.pin_buffer.push(character);
                self.lock_error = None;
                cx.notify();
            }
        }
        true
    }

    /// Opens the PIN dialog: the setup when there is no PIN yet, and the
    /// removal when there is one.
    pub(crate) fn open_pin_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chat_list_menu = false;
        let mode = if self.settings.app_lock_hash.is_empty() {
            PinDialogMode::Enable
        } else {
            PinDialogMode::Disable
        };
        let pin = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(i18n::t("PIN"))
                .masked(true)
        });
        let repeat = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(i18n::t("Repeat the PIN"))
                .masked(true)
        });
        pin.update(cx, |input, cx| input.focus(window, cx));
        self.pin_dialog = Some(PinDialog {
            mode,
            pin,
            repeat,
            error: None,
        });
        cx.notify();
    }

    /// Closes the PIN dialog without a change.
    pub(crate) fn close_pin_dialog(&mut self, cx: &mut Context<Self>) {
        self.pin_dialog = None;
        cx.notify();
    }

    /// Stores the new PIN, or deletes the one that was there.
    pub(crate) fn submit_pin_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.pin_dialog.as_ref() else {
            return;
        };
        let pin = dialog.pin.read(cx).value().trim().to_string();
        let repeat = dialog.repeat.read(cx).value().trim().to_string();
        match dialog.mode {
            PinDialogMode::Enable => {
                if !crate::app_lock::is_valid_pin(&pin) {
                    if let Some(dialog) = self.pin_dialog.as_mut() {
                        dialog.error = Some(i18n::t("The PIN needs 4 to 8 digits").to_string());
                    }
                    cx.notify();
                    return;
                }
                if pin != repeat {
                    if let Some(dialog) = self.pin_dialog.as_mut() {
                        dialog.error = Some(i18n::t("The two PINs are not the same").to_string());
                    }
                    cx.notify();
                    return;
                }
                let salt = crate::app_lock::new_salt();
                let hash = crate::app_lock::hash_pin(&pin, &salt);
                self.settings.app_lock_salt = salt;
                self.settings.app_lock_hash = hash;
            }
            PinDialogMode::Disable => {
                if !crate::app_lock::verify(
                    &pin,
                    &self.settings.app_lock_salt,
                    &self.settings.app_lock_hash,
                ) {
                    if let Some(dialog) = self.pin_dialog.as_mut() {
                        dialog.error = Some(i18n::t("Wrong PIN. Try again.").to_string());
                    }
                    cx.notify();
                    return;
                }
                self.settings.app_lock_salt.clear();
                self.settings.app_lock_hash.clear();
                self.locked = false;
            }
        }
        self.pin_dialog = None;
        self.save_settings();
        cx.notify();
    }

    /// Opens the dialog that creates a group, from the menu of the column.
    pub(crate) fn open_create_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chat_list_menu = false;
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(i18n::t("Group name")));
        name.update(cx, |input, cx| input.focus(window, cx));
        self.create_group = Some(CreateGroupState {
            name,
            selected: HashSet::new(),
            pending: false,
        });
        cx.notify();
    }

    /// Closes the group dialog without creating anything.
    pub(crate) fn close_create_group(&mut self, cx: &mut Context<Self>) {
        self.create_group = None;
        cx.notify();
    }

    /// Marks or unmarks one contact of the list of the group.
    pub(crate) fn toggle_group_participant(&mut self, jid: String, cx: &mut Context<Self>) {
        let Some(state) = self.create_group.as_mut() else {
            return;
        };
        if !state.selected.remove(&jid) {
            state.selected.insert(jid);
        }
        cx.notify();
    }

    /// Sends the creation of the group with the name and the marked contacts.
    pub(crate) fn submit_create_group(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.create_group.as_ref() else {
            return;
        };
        if state.pending {
            return;
        }
        let subject = state.name.read(cx).value().trim().to_string();
        if subject.is_empty() || state.selected.is_empty() {
            return;
        }
        let participant_jids: Vec<String> = state.selected.iter().cloned().collect();
        if let Some(state) = self.create_group.as_mut() {
            state.pending = true;
        }
        self.request(
            rpc_request::Request::CreateGroup(whatfast_protocol::CreateGroupRequest {
                subject,
                participant_jids,
            }),
            PendingRequest::CreateGroup,
        );
        cx.notify();
    }

    /// Turns the multiple selection of the chat list on.
    pub(crate) fn start_chat_selection(&mut self, cx: &mut Context<Self>) {
        self.chat_list_menu = false;
        self.selection = Some(ChatSelection::default());
        cx.notify();
    }

    /// Turns the multiple selection off and drops what was marked.
    pub(crate) fn cancel_chat_selection(&mut self, cx: &mut Context<Self>) {
        self.selection = None;
        cx.notify();
    }

    /// Marks or unmarks one conversation of the selection.
    pub(crate) fn toggle_chat_selection(&mut self, chat_id: String, cx: &mut Context<Self>) {
        let Some(selection) = self.selection.as_mut() else {
            return;
        };
        if !selection.ids.remove(&chat_id) {
            selection.ids.insert(chat_id);
        }
        cx.notify();
    }

    /// Applies one action to every marked conversation and turns the
    /// selection off.
    pub(crate) fn selection_mark_read(&mut self, cx: &mut Context<Self>) {
        for chat_id in self.take_selected_chats() {
            self.mark_chat_read(chat_id, cx);
        }
        cx.notify();
    }

    pub(crate) fn selection_mute(&mut self, cx: &mut Context<Self>) {
        for chat_id in self.take_selected_chats() {
            self.set_chat_muted(chat_id, true, None);
        }
        cx.notify();
    }

    pub(crate) fn selection_archive(&mut self, cx: &mut Context<Self>) {
        for chat_id in self.take_selected_chats() {
            self.set_chat_archived(chat_id, true);
        }
        cx.notify();
    }

    pub(crate) fn selection_pin(&mut self, cx: &mut Context<Self>) {
        for chat_id in self.take_selected_chats() {
            self.set_chat_pinned(chat_id, true);
        }
        cx.notify();
    }

    /// Takes the marked chats and turns the selection off.
    fn take_selected_chats(&mut self) -> Vec<String> {
        self.selection
            .take()
            .map(|selection| selection.ids.into_iter().collect())
            .unwrap_or_default()
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
        let previous_unread = self
            .store
            .chat(&chat_id)
            .map(|c| c.unread_count)
            .unwrap_or(0);
        self.load_selected_chat(chat_id.clone());
        self.store.mark_selected_chat_read_locally();
        self.request(
            rpc_request::Request::MarkRead(whatfast_protocol::MarkReadRequest {
                chat_id: chat_id.clone(),
                through_message_id: String::new(),
            }),
            PendingRequest::MarkRead {
                chat_id: chat_id.clone(),
                previous_unread,
            },
        );
        // The pictures of the tray belong to the conversation that was open:
        // the reference drops them when the user moves to another chat.
        if switching_chat {
            self.pending_images.clear();
            // El video que se estaba mirando es de la conversacion que se
            // deja: se corta junto con la descarga que estaba esperando.
            self.stop_video();
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
        // The drafts of the agents belong to the chat, so the band reads the
        // ones of the conversation that just opened.
        self.reload_agent_drafts();
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
        self.store.invalidate_all_message_heights();
        self.ensure_mention_directory(chat_id.clone());
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

    pub(crate) fn jump_to_bottom(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.stop_smooth_scroll(ScrollSurface::Messages);
        self.messages_scrolled_up = false;
        self.pending_top_scroll_id = None;
        self.pending_search_scroll_id = None;

        if self.store.has_newer_messages || self.store.newer_activity {
            self.refresh_latest();
        } else {
            let count = self.store.messages.len();
            if count > 0 {
                self.message_scroll
                    .scroll_to_item(count - 1, ScrollStrategy::Bottom);
            }
            self.mark_read();
        }
        cx.notify();
    }

    pub(crate) fn select_adjacent_chat(
        &mut self,
        next: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let chat_filter = self.store.chat_filter();
        let visible = self.store.chat_ids(chat_filter);
        if visible.is_empty() {
            return;
        }
        let current_index = self
            .store
            .selected_chat_id
            .as_ref()
            .and_then(|id| visible.iter().position(|c| c == id));
        let target_index = match current_index {
            Some(idx) => {
                if next {
                    (idx + 1).min(visible.len() - 1)
                } else {
                    idx.saturating_sub(1)
                }
            }
            None => 0,
        };
        let target_id = visible[target_index].clone();
        self.open_chat(target_id, window, cx);
    }

    pub(crate) fn page_messages(&mut self, down: bool) {
        let current = self.message_scroll.offset();
        let delta = if down { -350. } else { 350. };
        let new_offset = point(current.x, current.y + px(delta));
        self.message_scroll.set_offset(new_offset);
    }

    pub(crate) fn handle_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirm_delete_status.is_some() {
            self.confirm_delete_status = None;
            cx.notify();
            return;
        }
        if self.confirm_delete_message.is_some() {
            self.confirm_delete_message = None;
            cx.notify();
            return;
        }
        if self.video_fullscreen {
            self.video_fullscreen = false;
            cx.notify();
            return;
        }
        if self.media_editor.is_some() {
            self.media_editor = None;
            self.sync_shell(cx);
            cx.notify();
            return;
        }
        // Dentro del visor, escape cierra primero lo que flota encima de la
        // foto (el menu, la info, las reacciones) y no la foto entera.
        if self.image_viewer.is_some() {
            if self.image_context_menu.is_some() {
                self.image_context_menu = None;
            } else if self.viewer_info_open {
                self.viewer_info_open = false;
            } else if self.viewer_reactions_open {
                self.viewer_reactions_open = false;
            } else {
                self.image_viewer = None;
            }
            cx.notify();
            return;
        }
        if self.emoji_target.is_some() {
            self.emoji_target = None;
            cx.notify();
            return;
        }
        if self.sticker_tray_open {
            self.sticker_tray_open = false;
            cx.notify();
            return;
        }
        if self.mention_picker.is_some() {
            self.mention_picker = None;
            cx.notify();
            return;
        }
        if self.reaction_details.is_some() {
            self.reaction_details = None;
            cx.notify();
            return;
        }
        if self.chat_info.is_some() {
            self.chat_info = None;
            cx.notify();
            return;
        }
        if self.member_panel.is_open() {
            self.member_panel.set_open(false);
            cx.notify();
            return;
        }
        // La tarjeta del modo rapido es una capa que flota sobre la
        // conversacion: escape la cierra antes que nada de lo que hay abajo.
        if self.quick_ask.is_open() {
            self.close_quick_ask(window, cx);
            cx.notify();
            return;
        }
        if self.chat_search_open {
            self.close_chat_search(window, cx);
            return;
        }
        if self.replying_to_message_id.is_some() {
            self.cancel_reply();
            cx.notify();
            return;
        }
        if !self.search_query.is_empty() {
            self.clear_search(window, cx);
            cx.notify();
            return;
        }
        if self.settings_open {
            self.settings_open = false;
            self.sync_shell(cx);
            cx.notify();
            return;
        }
        if self.store.selected_chat_id.is_some() {
            self.composer
                .update(cx, |input, cx| input.focus(window, cx));
        }
    }

    pub(crate) fn send_trimmed_video(
        &mut self,
        path: PathBuf,
        start_seconds: f64,
        duration_seconds: Option<f64>,
        caption: String,
    ) {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("mp4");
        let destination = whatfast_model::paths::data_dir()
            .join("trimmed")
            .join(format!("{}.{ext}", uuid::Uuid::new_v4()));
        if let Some(parent) = destination.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match crate::video::trim_video(&path, &destination, start_seconds, duration_seconds) {
            Ok(()) => {
                self.send_attachment(
                    destination,
                    whatfast_protocol::AttachmentKind::Video,
                    caption,
                    false,
                );
            }
            Err(err) => {
                self.store.toast_error = Some(err);
            }
        }
    }

    pub(crate) fn begin_pairing(&mut self) {
        self.auth_probe = 0;
        self.account_ready = false;
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

    /// Pide los codigos de vinculacion sin cambiar de pantalla.
    ///
    /// La pantalla del QR se abre recien cuando llega un codigo de verdad: un
    /// "sin vincular" del arranque, que es la sesion todavia sin abrir, no
    /// alcanza para mostrarle el QR al usuario.
    pub(crate) fn request_pairing(&mut self) {
        if self
            .rpc
            .pending_requests()
            .any(|pending| matches!(pending, PendingRequest::Pairing))
        {
            return;
        }
        self.request(
            rpc_request::Request::StartPairing(whatfast_protocol::StartPairingRequest {}),
            PendingRequest::Pairing,
        );
    }

    /// Vuelve a preguntar el estado de la cuenta un instante despues.
    ///
    /// Solo mientras la ventana siga en `Starting`: si el usuario ya entro a
    /// las conversaciones, un "sin vincular" no lo saca de ahi.
    fn schedule_auth_probe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        cx.spawn_in(window, async move |_, window| {
            Timer::after(AUTH_PROBE_DELAY).await;
            let _ = window.update(|_, cx| {
                view.update(cx, |this, cx| {
                    // Con la cuenta ya contestada, o con el QR en pantalla, no
                    // hay nada que volver a preguntar.
                    if this.account_ready
                        || matches!(this.store.screen, Screen::Pairing | Screen::Fatal)
                    {
                        return;
                    }
                    if this
                        .rpc
                        .pending_requests()
                        .any(|pending| matches!(pending, PendingRequest::Auth))
                    {
                        return;
                    }
                    this.request(
                        rpc_request::Request::GetAuthState(
                            whatfast_protocol::GetAuthStateRequest {},
                        ),
                        PendingRequest::Auth,
                    );
                    cx.notify();
                })
            });
        })
        .detach();
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

    /// Reads the drafts that wait for the chat in front.
    ///
    /// The band over the composer draws the drafts of one conversation, so the
    /// list keeps only the rows of the chat that is open. A database that does
    /// not open leaves an empty list: the conversation never depends on it.
    pub(crate) fn reload_agent_drafts(&mut self) {
        self.agent_drafts = crate::chat_drafts::drafts_of(self.store.selected_chat_id.as_deref());
        self.agent_drafts_read = Instant::now();
    }

    /// Sends the text of one draft of the tray to its chat.
    ///
    /// The request is the one of the composer, because it is the only RPC that
    /// writes a text. The tray keeps the row it closes and takes the answer in
    /// [`Whatfast::handle_response`], so a failure marks the draft and leaves
    /// the field of the message alone.
    pub(crate) fn send_agent_draft(&mut self, draft_id: i64, chat_id: String, text: String) {
        if self.store.connection != ConnectionState::Connected {
            self.agents_panel
                .fail(i18n::t("Reconnect to WhatsApp before sending").into());
            return;
        }
        if let Err(error) = validate_text_message(&text) {
            self.agents_panel.fail(error);
            return;
        }
        self.agents_panel.mark_sending(&chat_id, &text, draft_id);
        self.request(
            rpc_request::Request::SendText(whatfast_protocol::SendTextRequest {
                client_message_id: Uuid::new_v4().to_string(),
                chat_id: chat_id.clone(),
                text: text.clone(),
                reply_to_message_id: String::new(),
                mentioned_jids: Vec::new(),
                reply_to_chat_id: String::new(),
            }),
            PendingRequest::SendText {
                chat_id,
                draft_text: text,
                reply_to_message_id: None,
                mentions: Vec::new(),
            },
        );
    }

    /// Opens the fast mode of the chat: a question about the conversation.
    ///
    /// The card needs a conversation in front, because the context it sends to
    /// the agent is the one of the messages on the screen.
    pub(crate) fn open_quick_ask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.store.screen != Screen::Chats {
            return;
        }
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            return;
        };
        // The card belongs to the conversation: the other views of the window
        // are not a chat and have nothing to ask about.
        self.agents_open = false;
        self.states_open = false;
        self.channels_open = false;
        self.archived_open = false;
        self.settings_open = false;
        self.quick_ask.open(chat_id, window, cx);
        cx.notify();
    }

    /// Closes the card of the fast mode and puts the cursor back in the
    /// composer.
    pub(crate) fn close_quick_ask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.quick_ask.close();
        self.composer
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// Sends the question of the fast mode to the agent in front.
    ///
    /// The answer does not leave the window: when it comes back it is written
    /// into the composer of the chat it was asked about.
    pub(crate) fn submit_quick_ask(&mut self, cx: &mut Context<Self>) {
        let Some((agent_id, chat_id, question)) = self.quick_ask.take_question(cx) else {
            return;
        };
        // The context is the one of the conversation on the screen. A question
        // about another chat has no messages to read, and the agent answers
        // with the question alone.
        let context = if self.store.selected_chat_id.as_deref() == Some(chat_id.as_str()) {
            crate::quick_ask::question_context(&self.store.messages)
        } else {
            Vec::new()
        };
        self.quick_ask.mark_pending(&chat_id);
        self.request(
            rpc_request::Request::AgentAsk(whatfast_protocol::AgentAskRequest {
                agent_id,
                chat_id: chat_id.clone(),
                question,
                context,
            }),
            PendingRequest::AgentAsk { chat_id },
        );
        cx.notify();
    }

    /// Writes the answer of an agent into the composer, without sending it.
    pub(crate) fn apply_agent_answer(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composer.update(cx, |input, cx| {
            input.set_value(text.clone(), window, cx);
            input.focus(window, cx);
        });
        self.update_active_draft_text(text);
    }

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
        // El campo es el mismo del chat: con el visor de estados abierto, el
        // texto es la respuesta a ese estado y no un mensaje de la
        // conversacion que esta detras.
        if self.status_strip.viewer_open() {
            crate::statuses::reply_to_the_open_status(self, window, cx);
            return;
        }
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
        self.open_media_editor(path, MediaEditorTarget::Chat, window, cx);
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
        let previous_unread = self.store.mark_selected_chat_read_locally().unwrap_or(0);
        let through_message_id = self
            .store
            .messages
            .iter()
            .rev()
            .find(|message| !message.from_me)
            .map(|message| message.id.clone())
            .unwrap_or_default();
        if !through_message_id.is_empty()
            && self.store.last_mark_read_id.as_deref() == Some(through_message_id.as_str())
            && previous_unread == 0
        {
            return;
        }
        if !through_message_id.is_empty() {
            self.store.last_mark_read_id = Some(through_message_id.clone());
        }
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

    /// Reads the space the media cache takes on this device.
    ///
    /// The answer paints the breakdown of the settings panel: nothing here
    /// talks to WhatsApp, only to the folder of this computer.
    pub(crate) fn load_storage_usage(&mut self) {
        report_failed_request(&mut self.store, self.rpc.request_storage_usage());
    }

    /// Deletes the files of the media cache of this device.
    ///
    /// Only the copies this computer downloaded: the account, the phone and
    /// the conversation stay as they are, and the app fetches a file again
    /// when the user opens the message.
    pub(crate) fn clear_media_cache(&mut self) {
        if self.storage_clearing {
            return;
        }
        self.storage_clearing = true;
        report_failed_request(&mut self.store, self.rpc.request_clear_media_cache());
    }

    /// Asks the sidecar to reconnect to WhatsApp.
    ///
    /// port: the origin refuses to reconnect when the account is already
    /// connected, and it shows the state of the connection while it waits.
    pub(crate) fn request_reconnect(&mut self) {
        if self.store.connection == ConnectionState::Connected
            || self.store.connection == ConnectionState::Connecting
        {
            return;
        }
        self.store.connection = ConnectionState::Connecting;
        // Sin detalle propio: la cabecera del chat muestra el estado
        // traducido ("connecting...").
        self.store.connection_detail.clear();
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
            paused: false,
            paused_at: None,
            paused_total: Duration::ZERO,
            levels: Vec::new(),
        });
        self.start_recording_tick(window, cx);
        cx.notify();
    }

    /// Pauses the capture: the clock stops and the wave stays still.
    pub(crate) fn pause_voice_recording(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.recording.as_mut() else {
            return;
        };
        if state.paused || state.finishing {
            return;
        }
        state.paused = true;
        state.paused_at = Some(Instant::now());
        self.recorder.pause();
        cx.notify();
    }

    /// Goes on with the capture after a pause.
    pub(crate) fn resume_voice_recording(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.recording.as_mut() else {
            return;
        };
        if !state.paused || state.finishing {
            return;
        }
        if let Some(paused_at) = state.paused_at.take() {
            state.paused_total += paused_at.elapsed();
        }
        state.paused = false;
        self.recorder.resume();
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
        // La onda de la pildora: cada tick suma una barra con el pico de las
        // muestras que llegaron desde el tick anterior. La escala de la voz
        // normal queda baja, asi que el pico se estira para que la barra se
        // vea.
        let new_levels = self.recorder.take_levels();
        let peak = new_levels.iter().fold(0f32, |peak, level| peak.max(*level));
        let bar = (peak * 4.0).clamp(0.08, 1.0);
        if let Some(state) = self.recording.as_mut()
            && !state.paused
            && !state.finishing
        {
            if state.levels.len() >= crate::audio::WAVE_BARS {
                state.levels.remove(0);
            }
            state.levels.push(bar);
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

    /// Whether the message is one of the statuses of the profile.
    fn is_status_message(&self, message_id: &str) -> bool {
        self.store
            .statuses()
            .iter()
            .flat_map(|group| group.messages.iter())
            .any(|message| message.id == message_id)
    }

    /// The attachment of a message, if it has one.
    ///
    /// The message can be in the open thread or in the statuses, which live in
    /// another list: the player and the download use the same attachment.
    fn message_attachment(&self, message_id: &str) -> Option<whatfast_protocol::AttachmentContent> {
        let attachment_of = |message: &whatfast_protocol::Message| match message.content.as_ref() {
            Some(whatfast_protocol::message::Content::Attachment(attachment)) => {
                Some(attachment.clone())
            }
            _ => None,
        };
        self.store
            .message(message_id)
            .and_then(attachment_of)
            .or_else(|| {
                self.store
                    .statuses()
                    .iter()
                    .flat_map(|group| group.messages.iter())
                    .find(|message| message.id == message_id)
                    .and_then(attachment_of)
            })
    }

    /// The attachment of a message, if it is a video.
    pub(crate) fn video_attachment(
        &self,
        message_id: &str,
    ) -> Option<whatfast_protocol::AttachmentContent> {
        self.message_attachment(message_id)
            .filter(crate::video::is_video)
    }

    /// El primer boton del video: reproduce, pausa o sigue donde estaba.
    ///
    /// El archivo que no esta en el disco se pide al sidecar y arranca solo
    /// cuando llega; el que ya esta se reproduce en la burbuja, sin abrir el
    /// reproductor del sistema.
    pub(crate) fn toggle_video_playback(
        &mut self,
        chat_id: String,
        message_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key: VideoKey = (chat_id, message_id);
        if self.video.is_for(&key) {
            // Con el archivo en camino, otro clic vuelve a empezar: si la
            // descarga se trabo, el usuario no queda encerrado.
            if self.video.waiting {
                self.stop_video();
            } else if self.video.is_running() {
                self.video_engine.pause();
                self.video.playing = false;
                cx.notify();
                return;
            } else if self.video.active {
                self.video_engine.resume();
                self.video.playing = true;
                cx.notify();
                return;
            }
        }
        self.stop_video();
        self.start_video_session(&key, window, cx);
        cx.notify();
    }

    /// Abre la sesion del video de un mensaje: con el archivo en el disco
    /// arranca ya, y sin el pide la descarga.
    /// Apaga o prende el sonido del video sin perder el volumen elegido.
    pub(crate) fn toggle_video_mute(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.video_muted = !self.video_muted;
        // Prender el sonido con el volumen en cero no se oye: el boton
        // devuelve la mitad, que es lo que hace cualquier reproductor.
        if !self.video_muted && self.video_volume <= 0.0 {
            self.video_volume = 0.5;
            self.volume_slider.update(cx, |slider, cx| {
                slider.set_value(0.5, window, cx);
            });
        }
        self.apply_video_volume(cx);
    }

    /// El volumen que el usuario solto en la barra del reproductor.
    fn set_video_volume(&mut self, volume: f32, cx: &mut Context<Self>) {
        self.video_volume = volume.clamp(0.0, 1.0);
        // Mover la barra es pedir sonido: si estaba en silencio, vuelve.
        if self.video_volume > 0.0 {
            self.video_muted = false;
        }
        self.apply_video_volume(cx);
    }

    /// Lleva el volumen y el silencio al reproductor y a la burbuja.
    fn apply_video_volume(&mut self, cx: &mut Context<Self>) {
        self.video.volume = self.video_volume;
        self.video.muted = self.video_muted;
        self.video_engine.set_volume(self.video.effective_volume());
        cx.notify();
    }

    fn start_video_session(&mut self, key: &VideoKey, window: &mut Window, cx: &mut Context<Self>) {
        let Some(attachment) = self.video_attachment(&key.1) else {
            return;
        };
        let path = attachment.local_path.clone();
        self.video = VideoPlayback {
            key: Some(key.clone()),
            duration: attachment.duration_seconds as f32,
            ..Default::default()
        };
        // El video arranca con el sonido que el usuario dejo.
        self.video.volume = self.video_volume;
        self.video.muted = self.video_muted;
        self.video_engine.set_volume(self.video.effective_volume());
        if !path.is_empty() && Path::new(&path).is_file() {
            self.video.path = path.clone();
            self.video.active = true;
            self.video.playing = true;
            self.video_engine.play(key.clone(), Path::new(&path));
            self.start_video_tick(window, cx);
            return;
        }
        // El archivo no esta: se baja y el video arranca cuando llega.
        self.store
            .set_message_attachment_path(&key.0, &key.1, String::new());
        self.video.waiting = true;
        self.attachment_play_requested = Some(key.clone());
        self.attachment_download_attempted.remove(key);
        self.load_message_attachment(key.0.clone(), key.1.clone());
    }

    /// Abre el video en el visor que ocupa la ventana entera.
    ///
    /// Un video que todavia no esta en el disco arranca su descarga y el
    /// visor muestra la caja negra hasta que llega el primer cuadro.
    pub(crate) fn open_video_fullscreen(
        &mut self,
        chat_id: String,
        message_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key: VideoKey = (chat_id, message_id);
        if !self.video.is_for(&key) {
            self.stop_video();
            self.start_video_session(&key, window, cx);
        }
        self.video_fullscreen = true;
        cx.notify();
    }

    /// Cierra el visor del video y deja el reproductor en la burbuja.
    pub(crate) fn close_video_fullscreen(&mut self, cx: &mut Context<Self>) {
        self.video_fullscreen = false;
        cx.notify();
    }

    /// Corta el video que se esta mirando.
    pub(crate) fn stop_video(&mut self) {
        if self.video.active {
            self.video_engine.stop();
        }
        self.attachment_play_requested = None;
        self.video_fullscreen = false;
        self.video = VideoPlayback::default();
    }

    /// El reloj que mueve los cuadros del video.
    fn start_video_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.video_tick {
            return;
        }
        self.video_tick = true;
        let view = cx.entity().downgrade();
        cx.spawn_in(window, async move |_, window| {
            loop {
                // Con el video andando se mira cada cuadro; en pausa alcanza
                // con mirar de vez en cuando.
                let running = window.update(|_, cx| {
                    view.update(cx, |this, _| {
                        let step = if this.video.playing {
                            crate::video::TICK_MS
                        } else {
                            100
                        };
                        (this.video.active, step)
                    })
                });
                let Ok(Ok((active, step))) = running else {
                    break;
                };
                if !active {
                    break;
                }
                Timer::after(Duration::from_millis(step)).await;
                let painted = window.update(|_, cx| {
                    view.update(cx, |this, cx| {
                        this.tick_video(cx);
                        let active = this.video.active;
                        if !active {
                            this.video_tick = false;
                        }
                        active
                    })
                });
                match painted {
                    Ok(Ok(true)) => {}
                    _ => break,
                }
            }
        })
        .detach();
    }

    /// Recoge los cuadros que el hilo del video dejo listos.
    pub(crate) fn tick_video(&mut self, cx: &mut Context<Self>) {
        // A video is watched in its conversation: it stops when the chat
        // changes. A status is the exception: it is watched over whatever chat
        // is open.
        if let Some((chat_id, message_id)) = self.video.key.clone()
            && self.store.selected_chat_id.as_deref() != Some(chat_id.as_str())
            && !self.is_status_message(&message_id)
        {
            self.stop_video();
            cx.notify();
            return;
        }
        while let Some(event) = self.video_engine.try_recv() {
            match event {
                crate::video::VideoEvent::Frame {
                    key,
                    frame,
                    position,
                } => {
                    if !self.video.is_for(&key) {
                        continue;
                    }
                    if let Some(image) = crate::video::render_image(frame) {
                        self.video.frame = Some(image);
                    }
                    self.video.position = position;
                }
                crate::video::VideoEvent::Finished { key } => {
                    if self.video.is_for(&key) {
                        // El ultimo cuadro se queda en la burbuja, con el
                        // boton de reproducir otra vez.
                        self.video.playing = false;
                        self.video.active = false;
                        self.video.position = self.video.duration;
                    }
                }
                crate::video::VideoEvent::Failed { key, message } => {
                    // The engine explains the failure in English for the log;
                    // the bubble says it in one line the reader understands.
                    eprintln!("video: {message}");
                    if self.video.is_for(&key) {
                        self.video.error = Some(i18n::t("The video could not be played.").into());
                        self.video.playing = false;
                        self.video.active = false;
                        self.video.waiting = false;
                    }
                }
            }
        }
        cx.notify();
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
        let Some(attachment) = self.message_attachment(&message_id) else {
            // The message has no attachment: there is nothing to open.
            return;
        };
        let path = attachment.local_path;
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
        let Some(attachment) = self.message_attachment(&message_id) else {
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
    pub(crate) fn clear_toast_info_later(&self, cx: &mut Context<Self>) {
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
                Some(i18n::t("Only your own messages can be deleted for everyone").into());
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
                self.agents_open = false;
                self.settings_open = false;
            }
            NavAction::Search => {
                self.states_open = false;
                self.channels_open = false;
                self.archived_open = false;
                self.agents_open = false;
                self.settings_open = false;
                self.focus_search(window, cx);
            }
            NavAction::States => {
                // The states are their own view: they are not conversations.
                self.states_open = true;
                self.channels_open = false;
                self.archived_open = false;
                self.agents_open = false;
                self.settings_open = false;
                self.load_statuses();
            }
            NavAction::Channels => {
                // The channels are their own view too.
                self.states_open = false;
                self.channels_open = true;
                self.archived_open = false;
                self.agents_open = false;
                self.settings_open = false;
                self.load_newsletters();
            }
            NavAction::Agents => {
                // The screen of the harnesses is its own view: the catalog of
                // the machine and the drafts of the profile, with its own
                // reader of `agents.db`.
                self.states_open = false;
                self.channels_open = false;
                self.archived_open = false;
                self.agents_open = true;
                self.settings_open = false;
                self.agents_panel.reload(window, cx);
                // The screen of the agents carries the automations: the list of
                // the tasks and the agents that can run one are read with the
                // rest of the screen.
                crate::automations_panel::open(self);
            }
            NavAction::Archived => {
                // The archived conversations are their own view.
                self.states_open = false;
                self.channels_open = false;
                self.archived_open = true;
                self.agents_open = false;
                self.settings_open = false;
                self.store.set_chat_filter(ChatFilter::Archived);
            }
            NavAction::Privacy | NavAction::Settings => {
                self.settings_open = true;
                // The panel shows the space of the media cache: it reads the
                // folder of this computer when it opens, not on the boot,
                // because a profile may never open it.
                self.load_storage_usage();
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

    /// Opens the screen of the agents.
    ///
    /// The harnesses are their own view: the keyboard reaches them the same
    /// way the column of icons does.
    pub(crate) fn open_agents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.on_nav_action(NavAction::Agents, window, cx);
    }

    /// Toggles the settings panel.
    pub(crate) fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_open {
            self.settings_open = false;
            self.sync_shell(cx);
            cx.notify();
        } else {
            self.on_nav_action(NavAction::Settings, window, cx);
        }
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
        let agents_open = self.agents_open;
        // Los no leidos de la lista y los estados sin ver: los dos datos que
        // la referencia pinta como el circulo del numero y el punto verde.
        let unread: u32 = self
            .store
            .chats
            .iter()
            .filter(|chat| !chat.archived)
            .map(|chat| chat.unread_count)
            .sum();
        let states_unseen = {
            let state = self.status_strip.state();
            let state = state.borrow();
            self.store
                .statuses()
                .iter()
                .any(|group| crate::statuses::has_unseen(&state, group))
        };
        self.nav_rail.update(cx, |rail, _| {
            rail.set_link(link);
            rail.set_sidebar_open(true);
            rail.set_settings_open(settings_open);
            rail.set_states_open(states_open);
            rail.set_channels_open(channels_open);
            rail.set_archived_open(archived_open);
            rail.set_agents_open(agents_open);
            rail.set_unread(unread);
            rail.set_states_unseen(states_unseen);
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
        target: MediaEditorTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.media_editor.is_some() {
            return;
        }
        self.media_editor_target = target;
        let editor = cx.new(|cx| MediaEditor::new(&path, window, cx));
        let subscription = cx.subscribe_in(&editor, window, move |this, _, event, window, cx| {
            match event {
                MediaEditorEvent::Applied(edits) => {
                    this.media_editor = None;
                    match this.media_editor_target {
                        MediaEditorTarget::Chat => {
                            for edit in edits {
                                this.stage_edited_picture(edit.clone(), window, cx);
                            }
                        }
                        MediaEditorTarget::Status => {
                            for edit in edits {
                                this.publish_edited_status(edit.clone(), window, cx);
                            }
                        }
                    }
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

    /// Publishes a status picture that was cropped, rotated and/or given caption/emojis.
    pub(crate) fn publish_edited_status(
        &mut self,
        edit: crate::media_editor::MediaEdit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source = edit.path.clone();
        let caption = edit.caption.clone();
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
        let task_edit = edit.clone();
        let edited = cx.background_spawn(async move {
            std::fs::create_dir_all(parent)
                .map_err(|error| error.to_string())
                .and_then(|()| {
                    crate::media_editor::apply_media_edit(&task_edit, &destination)
                })?;
            Ok::<std::path::PathBuf, String>(destination)
        });
        let state = self.status_strip.state();
        let view = cx.entity().downgrade();
        cx.spawn_in(window, async move |_, window| {
            let prepared = edited.await;
            window
                .update(|_, cx| {
                    view.update(cx, |this, cx| {
                        if let Ok(destination) = prepared {
                            let path_str = destination.to_string_lossy().into_owned();
                            crate::statuses::publish_photo_status_with_caption(
                                this,
                                &state,
                                &path_str,
                                &caption,
                                cx,
                            );
                        }
                    })
                    .ok();
                })
                .ok();
        })
        .detach();
    }

    /// Deletes a status message (both from local store and from backend).
    pub(crate) fn delete_status(&mut self, message_id: &str, cx: &mut Context<Self>) {
        let chat_id = crate::statuses::statuses_chat_id(self);
        let mut groups = self.store.statuses().to_vec();
        let mut removed = false;
        for group in &mut groups {
            if let Some(pos) = group.messages.iter().position(|m| m.id == message_id) {
                group.messages.remove(pos);
                removed = true;
                break;
            }
        }
        if removed {
            groups.retain(|g| !g.messages.is_empty());
            self.store.set_statuses(chat_id, groups);
            if !message_id.starts_with("local-status-") {
                let _ = self.rpc.request_delete_message("status@broadcast".to_string(), message_id.to_string(), true);
            }
            let state = self.status_strip.state();
            let mut inner = state.borrow_mut();
            if let Some(viewer) = inner.viewer.as_mut() {
                let groups_ref = self.store.statuses();
                if let Some(group) = groups_ref.get(viewer.group_index) {
                    if viewer.message_index >= group.messages.len() {
                        viewer.message_index = group.messages.len().saturating_sub(1);
                    }
                } else if !groups_ref.is_empty() {
                    viewer.group_index = viewer.group_index.min(groups_ref.len() - 1);
                    viewer.message_index = 0;
                } else {
                    inner.viewer = None;
                }
            }
            drop(inner);
            self.stop_video();
            self.store.toast_info = Some(i18n::t("Estado eliminado").into());
            cx.notify();
        }
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
        let task_edit = edit.clone();
        let edited = cx.background_spawn(async move {
            std::fs::create_dir_all(parent)
                .map_err(|error| error.to_string())
                .and_then(|()| {
                    crate::media_editor::apply_media_edit(&task_edit, &destination)
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
            MessageAction::Delete => {
                self.confirm_delete_message = Some(ConfirmDeleteMessage {
                    message_id,
                    delete_for_everyone: false,
                });
            }
            MessageAction::DeleteForEveryone => {
                self.confirm_delete_message = Some(ConfirmDeleteMessage {
                    message_id,
                    delete_for_everyone: true,
                });
            }
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
    if request.is_err() {
        store.screen = Screen::Fatal;
        store.fatal_error = Some(
            i18n::t("Whatfast lost the connection with its backend. Restart Whatfast.").into(),
        );
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
    for candidate in [chat.title.as_str(), message.sender_name.as_str()] {
        if !candidate.is_empty() {
            return candidate.to_string();
        }
    }
    if !chat.phone_number.is_empty() {
        return whatfast_core::phone::format(&chat.phone_number);
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
            Content::Unsupported(unsupported) if unsupported.type_name == "view_once" => {
                i18n::t("One-time message").to_string()
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

    /// El visor recorre su galeria: pasar de foto copia los datos de la nueva
    /// y devuelve el zoom a su sitio; fuera de la galeria no cambia nada.
    #[test]
    fn the_viewer_walks_its_gallery_and_resets_the_zoom() {
        let item = |id: &str| ViewerItem {
            message_id: id.into(),
            path: format!("{id}.jpg"),
            caption: String::new(),
            sticker: false,
            width: 10,
            height: 20,
            from_me: false,
            sender_name: String::new(),
            sender_avatar: String::new(),
            timestamp_ms: 1_700_000_000_000,
        };
        let mut viewer = ImageViewer {
            chat_id: "chat".into(),
            message_id: "a".into(),
            path: "a.jpg".into(),
            caption: String::new(),
            sticker: false,
            width: 10,
            height: 20,
            gallery: vec![item("a"), item("b")],
            index: 0,
            zoom: 2.,
        };
        assert!(!viewer.has_previous());
        assert!(viewer.has_next());
        viewer.show(1);
        assert_eq!(viewer.message_id, "b");
        assert_eq!(viewer.path, "b.jpg");
        assert_eq!(viewer.zoom, 1.);
        assert!(viewer.has_previous());
        assert!(!viewer.has_next());
        viewer.show(9);
        assert_eq!(viewer.message_id, "b");
    }

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
        assert_eq!(
            store.fatal_error.as_deref(),
            Some(i18n::t(
                "Whatfast lost the connection with its backend. Restart Whatfast."
            ))
        );
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
            paused: false,
            paused_at: None,
            paused_total: std::time::Duration::ZERO,
            levels: Vec::new(),
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

    /// La pausa no suma al reloj de la nota: el contador de la pildora tiene
    /// que mostrar el audio grabado, no el tiempo con la ventana abierta.
    #[test]
    fn the_clock_of_the_note_leaves_the_pause_out() {
        let state = RecordingState {
            started: std::time::Instant::now(),
            finishing: false,
            chat_id: "chat-a".into(),
            discarded: false,
            paused: false,
            paused_at: None,
            paused_total: std::time::Duration::from_secs(30),
            levels: Vec::new(),
        };
        assert!(state.elapsed_seconds() < 1.0);

        let paused = RecordingState {
            paused: true,
            paused_at: Some(std::time::Instant::now()),
            ..state
        };
        assert!(paused.elapsed_seconds() < 1.0);
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
