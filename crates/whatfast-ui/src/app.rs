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
    path::PathBuf,
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
};
use smol::Timer;
use uuid::Uuid;
use whatfast_model::{
    emoji_picker::{EmojiCategory, filtered as filter_emojis},
    model::{CHAT_PAGE_SIZE, ChatView, MESSAGE_PAGE_SIZE, Screen, Store, reactions_for_emoji},
    settings, sticker,
};
use whatfast_protocol::{
    BackendEvent, Chat, ChatParticipant, ConnectionState, ContactSearchResult, MessageSearchResult,
    RpcResponse, backend_event, rpc_request, rpc_response,
};

use crate::rpc::{
    BridgeMessage, PROTOCOL_VERSION, PendingRequest, REACTION_REPAIR_RETRY, RpcClient, RpcIncoming,
    TIMEOUT_SWEEP_INTERVAL, avatar_retry, reaction_retry,
};
use crate::text::{
    encode_mentions, format_typing_label, mention_display_name, mention_token_start,
    normalized_ui_scale, validate_image_message, validate_text_message,
};

// The actions the application answers to.
//
// port: the origin declared them at the root of its `main.rs`. They live
// here because the global handler of the chat switcher is a method of
// `Whatfast`. **They must not be declared twice**: GPUI panics when two
// actions share a name. The user interface imports them from this module.
actions!(whatfast, [CycleRecentChat, CycleRecentChatReverse]);

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

/// What the user wrote in one conversation, and what it replies to.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ChatDraft {
    pub(crate) text: String,
    pub(crate) reply_to_message_id: Option<String>,
    /// Participants tagged via the @ picker. Each entry backs an `@Name`
    /// token in `text`; tokens the user has since deleted are dropped at send.
    pub(crate) mentions: Vec<MentionEntry>,
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
    pub(crate) fn is_empty(&self) -> bool {
        self.text.is_empty() && self.reply_to_message_id.is_none()
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
    pub(crate) mention_picker: Option<MentionPicker>,
    pub(crate) mention_directories: HashMap<String, Vec<ChatParticipant>>,
    pub(crate) mention_directory_pending: HashSet<String>,
    pub(crate) typing_indicators: HashMap<String, HashMap<String, TypingIndicator>>,
    pub(crate) typing_signal: Option<(String, Instant)>,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) settings_open: bool,
    pub(crate) ui_scale: f32,
    pub(crate) chat_view: ChatView,
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
    pub(crate) _subscriptions: Vec<Subscription>,
}

impl Whatfast {
    /// Builds the application: the inputs, the subscriptions, the thread that
    /// reads from the backend and the sweep of the requests that time out.
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let ui_scale = load_ui_scale();
        apply_theme_scale(ui_scale, window, cx);
        let fake = std::env::args().any(|arg| arg == "--fake-backend")
            || std::env::var_os("WHATFAST_FAKE").is_some();
        let (rpc, startup_error) = match RpcClient::start(fake) {
            Ok(rpc) => (rpc, None),
            Err(error) => (RpcClient::disconnected(), Some(error.to_string())),
        };
        let receiver = rpc.incoming();
        let composer = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Type a message")
                .clean_on_escape()
        });
        let emoji_search = cx.new(|cx| InputState::new(window, cx).placeholder("Search emoji"));
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search contacts, groups, messages")
                .clean_on_escape()
        });
        let focus_handle = cx.focus_handle();
        // GPUI only routes key events along the focused path. Anchor focus to
        // the app root from the start so shortcuts work before (and after) any
        // text input holds focus.
        focus_handle.focus(window, cx);
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
                    if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                        this.send_text(window, cx);
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
            mention_picker: None,
            mention_directories: HashMap::new(),
            mention_directory_pending: HashSet::new(),
            typing_indicators: HashMap::new(),
            typing_signal: None,
            focus_handle,
            settings_open: false,
            ui_scale,
            chat_view: ChatView::Inbox,
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
            _subscriptions,
        };
        if let Some(error) = startup_error {
            this.store.screen = Screen::Fatal;
            this.store.fatal_error = Some(format!("Could not start the local backend: {error}"));
            return this;
        }
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
        if let Err(error) = self.rpc.send(request, pending) {
            self.store.screen = Screen::Fatal;
            self.store.fatal_error = Some(error.to_string());
        }
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
                    self.store.toast_error = Some("Reaction timed out. Try again.".into());
                }
                PendingRequest::MarkRead {
                    chat_id,
                    previous_unread,
                } => {
                    self.store.restore_unread(&chat_id, previous_unread);
                    self.store.last_mark_read_id = None;
                    self.store.toast_error = Some("Mark as read timed out. Try again.".into());
                }
                PendingRequest::Messages { prepend: true, .. } => {
                    self.pending_prepend_anchor = None;
                    self.store.toast_error = Some("Loading messages timed out. Try again.".into());
                }
                PendingRequest::MessagesAfter { .. } => {
                    self.newer_load_failed = true;
                    self.store.toast_error =
                        Some("Loading newer messages timed out. Try again.".into());
                }
                PendingRequest::Logout => {
                    self.confirming_logout = false;
                    self.store.toast_error = Some("Logout timed out. Try again.".into());
                }
                PendingRequest::Search { .. } => {}
                // Best-effort presence broadcast; peers expire it on their own.
                PendingRequest::SetTyping => {}
                _ => {
                    self.store.toast_error = Some("Backend request timed out. Try again.".into());
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
                self.chat_view = if self.store.chat(&chat_id).is_some_and(|chat| chat.archived) {
                    ChatView::Archived
                } else {
                    ChatView::Inbox
                };
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
            RpcIncoming::Event(event) => self.handle_event(event),
            RpcIncoming::Exited(error) => {
                self.store.connection = ConnectionState::Failed;
                self.store.connection_detail = error;
                self.store.toast_error =
                    Some("Backend stopped. Restart Whatfast to reconnect.".into());
            }
            RpcIncoming::Invalid => {
                self.store.toast_error = Some("Ignored an invalid backend envelope".into())
            }
            RpcIncoming::Ignore => {}
        }
    }

    pub(crate) fn handle_response(
        &mut self,
        pending: PendingRequest,
        response: RpcResponse,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(result) = response.result else {
            self.store.toast_error = Some("Backend returned an empty response".into());
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
            _ => {}
        }
    }

    pub(crate) fn handle_event(&mut self, event: BackendEvent) {
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
            // Detailed sync/coverage UI is implemented by the primary Tauri
            // shell. The legacy behavioral reference keeps its coarse status.
            Some(backend_event::Event::SyncStatusChanged(_)) => {}
            Some(backend_event::Event::HistoryCoverageChanged(_)) => {}
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
            // legacy GPUI reference has nothing to refresh in response.
            Some(backend_event::Event::StickersChanged(_)) => {}
            // Pinned-message browsing is implemented by the primary Tauri
            // shell; the legacy GPUI behavioral reference has no surface.
            Some(backend_event::Event::PinnedMessagesChanged(_)) => {}
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
        self.emoji_target = match self.emoji_target {
            Some(EmojiTarget::Composer) => None,
            _ => Some(EmojiTarget::Composer),
        };
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
        };
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

    pub(crate) fn toggle_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = if cx.theme().is_dark() {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        Theme::change(mode, Some(window), cx);
        apply_theme_scale(self.ui_scale, window, cx);
        cx.notify();
    }

    pub(crate) fn set_ui_scale(&mut self, scale: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.ui_scale = normalized_ui_scale(scale);
        apply_theme_scale(self.ui_scale, window, cx);
        self.store.invalidate_all_message_heights();
        if let Err(error) = save_ui_scale(self.ui_scale) {
            self.store.toast_error = Some(format!("Could not save UI scale: {error}"));
        }
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

    pub(crate) fn toggle_chat_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chat_view = match self.chat_view {
            ChatView::Inbox => ChatView::Archived,
            ChatView::Archived => ChatView::Inbox,
        };
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
            for chat_id in self.store.chat_ids(self.chat_view).iter() {
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
        if self.store.chat(&chat_id).is_none() {
            return;
        }
        self.chat_view = if self.store.chat(&chat_id).is_some_and(|chat| chat.archived) {
            ChatView::Archived
        } else {
            ChatView::Inbox
        };
        record_recent_chat(&mut self.recent_chat_ids, &chat_id);
        self.load_selected_chat(chat_id.clone());
        let draft = self.chat_drafts.get(&chat_id).cloned().unwrap_or_default();
        self.replying_to_message_id = draft.reply_to_message_id;
        self.composer.update(cx, |input, cx| {
            input.set_value(draft.text, window, cx);
            input.focus(window, cx);
        });
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
        self.emoji_target = None;
        self.reaction_details = None;
        self.image_viewer = None;
        self.replying_to_message_id = None;
        self.chat_drafts.clear();
        self.recent_chat_ids.clear();
        self.chat_switcher = None;
        self.chat_info = None;
        self.chat_view = ChatView::Inbox;
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
            self.store.toast_error = Some("Choose a conversation before sending".into());
            return;
        };
        if self.store.connection != ConnectionState::Connected {
            self.store.toast_error = Some("Reconnect to WhatsApp before sending".into());
            return;
        }
        if let Err(error) = validate_text_message(&text) {
            self.store.toast_error = Some(error);
            return;
        }
        self.store.toast_error = None;
        let reply_to_message_id = self.replying_to_message_id.clone();
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

    pub(crate) fn choose_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.store.connection != ConnectionState::Connected
            || self.store.selected_chat_id.is_none()
        {
            return;
        }
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose an image to send".into()),
        });
        let view = cx.entity();
        cx.spawn_in(window, async move |_, window| {
            let path = selected.await.ok()?.ok()??.into_iter().next()?;
            window
                .update(|window, cx| view.update(cx, |this, cx| this.send_image(path, window, cx)))
                .ok()
        })
        .detach();
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

    pub(crate) fn send_image(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            self.store.toast_error = Some("Choose a conversation before sending".into());
            return;
        };
        if self.store.connection != ConnectionState::Connected {
            self.store.toast_error = Some("Reconnect to WhatsApp before sending".into());
            return;
        }
        let draft_text = self.composer.read(cx).value().to_string();
        let caption = draft_text.trim().to_string();
        if let Err(error) = validate_image_message(&path, &caption) {
            self.store.toast_error = Some(error);
            return;
        }
        self.store.toast_error = None;
        let reply_to_message_id = self.replying_to_message_id.clone();
        self.clear_active_draft();
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.request(
            rpc_request::Request::SendImage(whatfast_protocol::SendImageRequest {
                client_message_id: Uuid::new_v4().to_string(),
                chat_id,
                image_path: path.to_string_lossy().into_owned(),
                caption,
                reply_to_message_id: reply_to_message_id.clone().unwrap_or_default(),
                reply_to_chat_id: String::new(),
            }),
            PendingRequest::SendImage {
                // port: the same quirk as in `send_text`.
                chat_id: self.store.selected_chat_id.clone().unwrap_or_default(),
                draft_text,
                reply_to_message_id,
            },
        );
    }

    pub(crate) fn send_sticker(
        &mut self,
        prepared: sticker::PreparedSticker,
        chat_id: String,
        reply_to_message_id: Option<String>,
    ) {
        if self.store.connection != ConnectionState::Connected {
            self.store.toast_error = Some("Reconnect to WhatsApp before sending".into());
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
