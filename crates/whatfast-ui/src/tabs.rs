//! The strip of tabs: the chats opened in a pane.
//!
//! port: `tauri/src/components/Tabs.tsx` (the component),
//! `tauri/src/state/workspace.ts` (the state of the panes) and
//! `tauri/src/state/tab-keyboard.ts` (the keyboard).
//!
//! The strip is a component of its own: the app owns an `Entity<TabStrip>`,
//! keeps the tabs and the workspace in sync and drops the entity in the
//! layout. Every gesture of the strip â€” a click, a close, a drop, a key â€” is
//! reported to the app through the handler the app installs with
//! [`TabStrip::on_action`]; the strip never changes the state of the app by
//! itself, but it does keep its own copy of the tabs of its pane up to date,
//! the way a browser keeps the tabs of its window.

use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, ClickEvent, Context,
    DragMoveEvent, ElementId, FocusHandle, FontWeight, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, ParentElement as _, Pixels, Render, ScrollHandle,
    StatefulInteractiveElement as _, Styled as _, Window, div, px, transparent_black,
};
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _};

use whatfast_core::tab_keyboard::{
    TabDirection, TabKeyboardCommand, TabKeyboardInput, tab_keyboard_command,
};
use whatfast_core::workspace::{self, Pane};

use crate::helpers::avatar_element;
use crate::tokens::normalized_ui_scale;

/// The width of the title of a tab, before the scale.
///
/// port: `.tab-title { font-size: calc(12.5px * var(--scale)) }`.
const TITLE_SIZE: f32 = 12.5;

/// The smallest width of a tab, before the scale.
///
/// port: `.tab-slot { min-width: calc(120px * var(--scale)) }`.
const TAB_MIN_WIDTH: f32 = 120.0;

/// The biggest width of a tab, before the scale.
///
/// port: `.tab-slot { max-width: calc(200px * var(--scale)) }`.
const TAB_MAX_WIDTH: f32 = 200.0;

/// The size of the avatar of a tab, before the scale.
///
/// port: `<Avatar ... size={16 * preferences.uiScale} />`.
const AVATAR_SIZE: f32 = 16.0;

/// The size of the badge of the unread messages, before the scale.
///
/// port: `.tab-badge { min-width: calc(16px * var(--scale)); height: ... }`.
const BADGE_SIZE: f32 = 16.0;

/// The size of the font of the badge, before the scale.
///
/// port: `.tab-badge { font-size: calc(9.5px * var(--scale)) }`.
const BADGE_FONT_SIZE: f32 = 9.5;

/// The size of the button that closes a tab, before the scale.
///
/// port: `.tab-close { width: calc(18px * var(--scale)); height: ... }`.
const CLOSE_SIZE: f32 = 18.0;

/// The size of the glyph that closes a tab, before the scale.
///
/// port: `<X size={13} />`.
const CLOSE_ICON_SIZE: f32 = 13.0;

/// The size of the button that opens a new tab, before the scale.
///
/// port: `.tab-add { width: calc(28px * var(--scale)); height: ... }`.
const ADD_SIZE: f32 = 28.0;

/// The size of the glyph that opens a new tab, before the scale.
///
/// port: `<Plus size={15} />`.
const ADD_ICON_SIZE: f32 = 15.0;

/// The gap between the tabs, before the scale.
///
/// port: `.tab-strip { gap: 2px }`.
const STRIP_GAP: f32 = 2.0;

/// The padding of the strip, before the scale.
///
/// port: `.tab-strip { padding: calc(5px * var(--scale)) 2px 0 }`.
const STRIP_PADDING: f32 = 5.0;

/// The biggest number of unread messages the badge shows.
///
/// port: `Math.min(chat()!.unreadCount, 99)`.
const BADGE_MAX: u32 = 99;

/// Cuanto tarda una pestaña cerrada en encogerse hasta desaparecer.
///
/// Es una salida: corre mas rapido que una apertura y no se hace esperar, para
/// que cerrar dos pestañas seguidas no se sienta lento.
const TAB_CLOSE_DURATION: Duration = Duration::from_millis(180);

/// One open chat of the strip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TabEntry {
    /// The id of the chat.
    pub(crate) chat_id: String,
    /// The title of the chat: the name, the phone number or "Chat".
    pub(crate) title: String,
    /// The path of the photo of the chat (empty when there is none).
    pub(crate) avatar_path: String,
    /// Whether the chat is a group: the avatar shows the people of the group
    /// and not the initials.
    pub(crate) group: bool,
    /// The number of unread messages of the chat.
    pub(crate) unread_count: u32,
}

impl TabEntry {
    /// The entry of a chat of the model.
    pub(crate) fn from_chat(chat: &whatfast_protocol::Chat) -> Self {
        Self {
            chat_id: chat.id.clone(),
            title: chat_title(chat),
            avatar_path: chat.avatar_path.clone(),
            group: chat.kind() == whatfast_protocol::ChatKind::Group,
            unread_count: chat.unread_count,
        }
    }
}

/// The title of a chat.
///
/// port: `chatName` of the oracle: the title of the chat, the phone number
/// written as WhatsApp writes it, or "Chat".
pub(crate) fn chat_title(chat: &whatfast_protocol::Chat) -> String {
    if !chat.title.is_empty() {
        chat.title.clone()
    } else if !chat.phone_number.is_empty() {
        whatfast_core::phone::format(&chat.phone_number)
    } else {
        "Chat".to_string()
    }
}

/// A tab that was closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TabClose {
    /// The chat that was closed.
    pub(crate) chat_id: String,
    /// The chat that takes the focus, when the pane has one left.
    pub(crate) next_chat_id: Option<String>,
}

/// A tab that changed its place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TabMove {
    /// The chat that moved.
    pub(crate) chat_id: String,
    /// The place it had.
    pub(crate) from_index: usize,
    /// The place it has.
    pub(crate) to_index: usize,
}

/// A chat that is being dragged inside the strip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TabDrag {
    /// The chat that is being dragged.
    pub(crate) chat_id: String,
    /// The place the chat would land on.
    pub(crate) target_index: usize,
}

/// What the strip asks the app to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TabAction {
    /// The key is not a gesture of the strip: the app lets it through.
    Unhandled,
    /// The app has to show another chat of the pane.
    Select { chat_id: String },
    /// The app has to close a tab.
    Close {
        chat_id: String,
        title: String,
        next_chat_id: Option<String>,
        message: String,
    },
    /// The app has to move a tab inside the pane.
    Reorder {
        chat_id: String,
        title: String,
        index: usize,
        direction: TabDirection,
        message: String,
    },
    /// The app has to move a tab to another pane.
    MoveToPane {
        chat_id: String,
        title: String,
        pane_id: String,
        index: usize,
        direction: TabDirection,
        message: String,
    },
    /// The app has to open the search of the chats, to open a new tab.
    Add,
    /// The gesture has nowhere to go: the message goes to the live region.
    Boundary { message: String },
}

/// The value that travels with the drag of a tab.
///
/// port: the payload of the drag of the oracle (`TabDragPayload`): the chat
/// that moves and the pane it comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TabDragPayload {
    chat_id: String,
    title: String,
}

/// The handler the gestures report to.
pub(crate) type TabActionHandler = Option<Rc<dyn Fn(TabAction, &mut Window, &mut App)>>;

/// Una pestaña que se esta cerrando.
///
/// Sale del riel en el acto, pero se dibuja un instante mas: se encoge hasta el
/// ancho cero para que las vecinas ocupen su lugar sin saltar.
struct ClosingTab {
    /// La pestaña, con los datos que tenia al cerrarse.
    entry: TabEntry,
    /// El lugar que ocupaba en el riel.
    index: usize,
    /// Si era la pestaña que se estaba mirando: el fantasma conserva su
    /// aspecto para que el cierre no cambie de color al empezar.
    active: bool,
    /// El ancho que tenia, en pixeles ya escalados.
    width: f32,
    /// Cuando empezo a encogerse.
    started_at: Instant,
    /// Numero de la animacion: dos cierres de la misma pestaña no comparten
    /// el estado del elemento animado.
    serial: usize,
}

/// The strip of tabs of a pane.
pub(crate) struct TabStrip {
    /// The id of the pane the strip belongs to.
    pane_id: String,
    /// The open chats, in order.
    tabs: Vec<TabEntry>,
    /// The chat that is showing.
    active_chat_id: String,
    /// The workspace the strip lives in: the keyboard needs the other panes to
    /// know where a tab can go.
    panes: Vec<Pane>,
    /// The chat that is being dragged.
    dragging: Option<TabDrag>,
    /// The place the dragged chat would land on.
    drop_index: Option<usize>,
    /// The handle of the focus of the strip: `None` when the strip was built
    /// without a window.
    focus_handle: Option<FocusHandle>,
    /// The scroll of the strip.
    scroll_handle: ScrollHandle,
    /// The scale of the interface.
    scale: f32,
    /// Las pestañas que se estan cerrando.
    closing: Vec<ClosingTab>,
    /// El ancho medido de cada pestaña abierta, en pixeles ya escalados.
    ///
    /// Lo escribe la medicion del riel en cada cuadro: el cierre arranca desde
    /// el ancho real y no desde el minimo, que daria un salto al empezar.
    widths: Rc<RefCell<HashMap<String, f32>>>,
    /// El numero de la proxima animacion de cierre.
    closing_serial: usize,
    /// The handler the gestures report to.
    on_action: TabActionHandler,
}

impl std::fmt::Debug for TabStrip {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TabStrip")
            .field("pane_id", &self.pane_id)
            .field("tabs", &self.tabs)
            .field("active_chat_id", &self.active_chat_id)
            .field("dragging", &self.dragging)
            .field("drop_index", &self.drop_index)
            .field("closing", &self.closing.len())
            .field("scale", &self.scale)
            .finish_non_exhaustive()
    }
}

impl TabStrip {
    /// The strip of the tabs of a pane, with the handles of a window.
    pub(crate) fn new(cx: &mut Context<Self>, pane_id: impl Into<String>, scale: f32) -> Self {
        Self {
            focus_handle: Some(cx.focus_handle()),
            ..Self::state(pane_id, scale)
        }
    }

    /// The strip without a window: the state alone.
    ///
    /// The state of the strip is the same with and without a window; the
    /// handles are the only thing that the window gives.
    pub(crate) fn state(pane_id: impl Into<String>, scale: f32) -> Self {
        Self {
            pane_id: pane_id.into(),
            tabs: Vec::new(),
            active_chat_id: String::new(),
            panes: Vec::new(),
            dragging: None,
            drop_index: None,
            focus_handle: None,
            scroll_handle: ScrollHandle::new(),
            scale: normalized_ui_scale(scale),
            closing: Vec::new(),
            widths: Rc::new(RefCell::new(HashMap::new())),
            closing_serial: 0,
            on_action: None,
        }
    }

    /// The handler the gestures report to.
    pub(crate) fn on_action(
        mut self,
        handler: impl Fn(TabAction, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_action = Some(Rc::new(handler));
        self
    }

    /// The place of a chat in the strip.
    pub(crate) fn index_of(&self, chat_id: &str) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.chat_id == chat_id)
    }

    /// Replaces the tabs of the strip.
    ///
    /// La lista de la app manda: una pestaña que ya no esta se cierra con la
    /// misma animacion que la de la X, aunque el cierre no haya pasado por
    /// [`TabStrip::close`] (un chat que se borra, por ejemplo).
    pub(crate) fn set_tabs(&mut self, tabs: Vec<TabEntry>, active_chat_id: impl Into<String>) {
        let gone = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, old)| !tabs.iter().any(|tab| tab.chat_id == old.chat_id))
            .map(|(index, old)| (index, old.chat_id.clone()))
            .collect::<Vec<_>>();
        for (index, chat_id) in gone {
            self.start_closing(&chat_id, index);
        }
        // Una pestaña que vuelve antes de terminar la animacion no deja
        // fantasma: se dibuja la pestaña nueva y nada mas.
        self.closing
            .retain(|closing| !tabs.iter().any(|tab| tab.chat_id == closing.entry.chat_id));
        self.tabs = tabs;
        self.active_chat_id = active_chat_id.into();
        // A tab that disappears while it is dragged has no mouse-up waiting:
        // cancel the drop instead of keeping a ghost in the strip forever.
        if self
            .dragging
            .as_ref()
            .is_some_and(|drag| !self.tabs.iter().any(|tab| tab.chat_id == drag.chat_id))
        {
            self.dragging = None;
            self.drop_index = None;
        }
        self.prune_closing();
    }

    /// Focuses a tab that is already open.
    pub(crate) fn select(&mut self, chat_id: &str) -> bool {
        self.index_of(chat_id).is_some_and(|_| {
            self.active_chat_id = chat_id.to_string();
            true
        })
    }

    /// Closes a tab.
    ///
    /// port: `closeTab` of the oracle: the neighbour to the right of the
    /// closed tab takes the focus, and the one to the left when there is no
    /// neighbour on the right.
    pub(crate) fn close(&mut self, chat_id: &str) -> Option<TabClose> {
        let index = self.index_of(chat_id)?;
        // El fantasma ocupa el lugar que la pestaña deja, mientras se encoge.
        self.start_closing(chat_id, index);
        let pane = workspace::close_tab(&self.to_pane(), chat_id);
        let next_chat_id = if pane.active_chat_id.is_empty() {
            None
        } else {
            Some(pane.active_chat_id.clone())
        };
        self.apply_pane(&pane);
        Some(TabClose {
            chat_id: chat_id.to_string(),
            next_chat_id,
        })
    }

    /// Moves a tab to a place of the strip.
    ///
    /// port: `moveTab` of the oracle: the tab leaves its place, lands on the
    /// new one and takes the focus.
    pub(crate) fn move_tab(&mut self, chat_id: &str, index: usize) -> Option<TabMove> {
        let from_index = self.index_of(chat_id)?;
        let panes = workspace::move_tab_between_panes(
            &[self.to_pane()],
            chat_id,
            &self.pane_id,
            &self.pane_id,
            index as i64,
        );
        let pane = panes.first()?;
        self.apply_pane(pane);
        let to_index = self.index_of(chat_id)?;
        Some(TabMove {
            chat_id: chat_id.to_string(),
            from_index,
            to_index,
        })
    }

    /// The chat that is being dragged.
    pub(crate) fn dragging(&self) -> Option<&str> {
        self.dragging.as_ref().map(|drag| drag.chat_id.as_str())
    }

    /// The pointer drags a chat over a place of the strip.
    ///
    /// port: `onDragOver` of the oracle: the tab follows the pointer and the
    /// strip knows where it would land.
    pub(crate) fn drag_over(&mut self, chat_id: &str, index: usize) -> bool {
        let index = index.min(self.tabs.len());
        let changed = self.drop_index != Some(index)
            || self.dragging.as_ref().map(|drag| drag.chat_id.as_str()) != Some(chat_id);
        self.dragging = Some(TabDrag {
            chat_id: chat_id.to_string(),
            target_index: index,
        });
        self.drop_index = Some(index);
        changed
    }

    /// The drag ends: the chat lands on the place the pointer chose.
    ///
    /// port: `handleDrop` of the oracle: the place of the drop is the one of
    /// the list without the tab that moves, and the tab lands there.
    pub(crate) fn commit_drag(&mut self) -> Option<TabMove> {
        let drag = self.dragging.take()?;
        let boundary = self.drop_index.take().unwrap_or(self.tabs.len());
        let chat_ids = self
            .tabs
            .iter()
            .map(|tab| tab.chat_id.clone())
            .collect::<Vec<_>>();
        let index = workspace::same_pane_drop_index(&chat_ids, &drag.chat_id, boundary as i64);
        self.move_tab(&drag.chat_id, index.max(0) as usize)
    }

    /// The drag ends without a move: the tab stays where it was.
    pub(crate) fn cancel_drag(&mut self) {
        self.dragging = None;
        self.drop_index = None;
    }

    /// The place a drop on a tab lands on.
    ///
    /// port: `dropIndexFor` of the oracle: the right half of the tab inserts
    /// the tab after it, the left half before it.
    pub(crate) fn drop_index_for(target_index: usize, right_half: bool) -> usize {
        if right_half {
            target_index + 1
        } else {
            target_index
        }
    }

    /// A key press of the strip.
    ///
    /// port: the `onKeyDown` of the oracle: the accessible commands come
    /// first â€” the close, the reorder and the move to the other pane â€” and
    /// the plain arrows move the focus between the tabs.
    pub(crate) fn handle_key(&mut self, input: &TabKeyboardInput, chat_id: &str) -> TabAction {
        if let Some(command) = tab_keyboard_command(input, &self.panes, &self.pane_id, chat_id) {
            return self.apply_command(command, chat_id);
        }
        self.move_selection(chat_id, &input.key)
    }

    /// The title of a chat of the strip.
    fn title_of(&self, chat_id: &str) -> String {
        self.tabs
            .iter()
            .find(|tab| tab.chat_id == chat_id)
            .map(|tab| tab.title.clone())
            .unwrap_or_default()
    }

    /// The pane of the strip, as the core sees it.
    fn to_pane(&self) -> Pane {
        Pane {
            id: self.pane_id.clone(),
            tab_chat_ids: self.tabs.iter().map(|tab| tab.chat_id.clone()).collect(),
            active_chat_id: self.active_chat_id.clone(),
        }
    }

    /// Rebuilds the strip from a pane of the core.
    fn apply_pane(&mut self, pane: &Pane) {
        let mut remaining = self.tabs.clone();
        let mut tabs = Vec::with_capacity(pane.tab_chat_ids.len());
        for chat_id in &pane.tab_chat_ids {
            if let Some(position) = remaining.iter().position(|tab| &tab.chat_id == chat_id) {
                tabs.push(remaining.remove(position));
            }
        }
        self.tabs = tabs;
        self.active_chat_id = pane.active_chat_id.clone();
    }

    /// The command of the keyboard, applied to the strip.
    fn apply_command(&mut self, command: TabKeyboardCommand, chat_id: &str) -> TabAction {
        let title = self.title_of(chat_id);
        match command {
            TabKeyboardCommand::Close => {
                let close = self.close(chat_id);
                TabAction::Close {
                    chat_id: chat_id.to_string(),
                    title: title.clone(),
                    next_chat_id: close.and_then(|close| close.next_chat_id),
                    message: format!("{title} closed"),
                }
            }
            TabKeyboardCommand::Reorder { index, direction } => {
                self.move_tab(chat_id, index);
                TabAction::Reorder {
                    chat_id: chat_id.to_string(),
                    title: title.clone(),
                    index,
                    direction,
                    message: format!(
                        "{title} moved {} to position {}",
                        direction_name(direction),
                        index + 1
                    ),
                }
            }
            TabKeyboardCommand::MovePane {
                pane_id,
                index,
                direction,
            } => {
                // The tab leaves this pane: the strip gives it up, and the app
                // does the same in the workspace.
                if self.panes.iter().any(|pane| pane.id == pane_id) {
                    self.close(chat_id);
                }
                TabAction::MoveToPane {
                    chat_id: chat_id.to_string(),
                    title: title.clone(),
                    pane_id,
                    index,
                    direction,
                    message: format!(
                        "{title} moved to the {} pane, position {}",
                        direction_name(direction),
                        index + 1
                    ),
                }
            }
            TabKeyboardCommand::Boundary { message } => TabAction::Boundary { message },
        }
    }

    /// The plain arrows move the focus between the tabs.
    ///
    /// port: `moveSelection` of the oracle: the focus wraps around the strip.
    fn move_selection(&mut self, chat_id: &str, key: &str) -> TabAction {
        if self.tabs.is_empty() {
            // port: the oracle swallows the arrows of an empty strip.
            return TabAction::Unhandled;
        }
        let current = self.index_of(chat_id).unwrap_or(0);
        let next = match key {
            "Home" => 0,
            "End" => self.tabs.len() - 1,
            "ArrowRight" => (current + 1) % self.tabs.len(),
            "ArrowLeft" => (current + self.tabs.len() - 1) % self.tabs.len(),
            _ => return TabAction::Unhandled,
        };
        let next_chat_id = self.tabs[next].chat_id.clone();
        self.active_chat_id = next_chat_id.clone();
        TabAction::Select {
            chat_id: next_chat_id,
        }
    }

    /// The strip of tabs.
    pub(crate) fn render(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        // Un fantasma que ya termino su animacion no se dibuja mas.
        self.prune_closing();
        let scale = self.scale;
        let mut strip = div()
            .id("tab-strip")
            .flex()
            .items_stretch()
            .gap(px(STRIP_GAP * scale))
            .px(px(STRIP_GAP * scale))
            .pt(px(STRIP_PADDING * scale))
            .overflow_x_scroll()
            .track_scroll(&self.scroll_handle)
            .on_key_down(cx.listener(|this, event, window, cx| {
                let chat_id = this.active_chat_id.clone();
                let input = keyboard_input(event);
                match this.handle_key(&input, &chat_id) {
                    TabAction::Unhandled => cx.propagate(),
                    action => {
                        this.report(action, window, cx);
                        // Un cierre con el teclado deja el mismo fantasma.
                        cx.notify();
                    }
                }
            }))
            .on_drag_move::<TabDragPayload>(cx.listener(
                |this: &mut TabStrip, event: &DragMoveEvent<TabDragPayload>, _window, cx| {
                    let Some(payload) = event.dragged_item().downcast_ref::<TabDragPayload>()
                    else {
                        return;
                    };
                    let position = event.event.position;
                    let bounds = event.bounds;
                    let inside = position.x >= bounds.origin.x
                        && position.x <= bounds.origin.x + bounds.size.width
                        && position.y >= bounds.origin.y
                        && position.y <= bounds.origin.y + bounds.size.height;
                    if !inside {
                        // The pointer left the strip: the tab stays where it was.
                        if this.dragging().is_some() {
                            this.cancel_drag();
                            cx.notify();
                        }
                        return;
                    }
                    // The pointer is over the strip: the tab that follows sets
                    // the exact place, and this one is the end of the list.
                    let index = this.tabs.len();
                    if this.drag_over(&payload.chat_id, index) {
                        cx.notify();
                    }
                },
            ))
            .on_drop::<TabDragPayload>(cx.listener(|this, _payload, window, cx| {
                if let Some(moved) = this.commit_drag() {
                    let title = this.title_of(&moved.chat_id);
                    let direction = if moved.to_index >= moved.from_index {
                        TabDirection::Right
                    } else {
                        TabDirection::Left
                    };
                    let action = TabAction::Reorder {
                        chat_id: moved.chat_id.clone(),
                        title: title.clone(),
                        index: moved.to_index,
                        direction,
                        message: format!(
                            "{title} moved {} to position {}",
                            direction_name(direction),
                            moved.to_index + 1
                        ),
                    };
                    this.report(action, window, cx);
                }
                cx.notify();
            }));

        if let Some(focus) = &self.focus_handle {
            strip = strip.track_focus(focus).focusable();
        }

        // Las pestañas y los fantasmas de las que se cierran van en el orden
        // del riel. El fantasma de un lugar va antes que la pestaña que hoy
        // ocupa ese lugar, porque era la que estaba a su izquierda.
        let mut slots = Vec::with_capacity(self.tabs.len() + self.closing.len());
        for (index, entry) in self.tabs.iter().enumerate() {
            slots.push((index, 1, self.render_tab(index, entry, cx).into_any_element()));
        }
        for closing in &self.closing {
            slots.push((closing.index, 0, self.render_closing_tab(closing, cx)));
        }
        slots.sort_by_key(|(index, rank, _)| (*index, *rank));
        for (_, _, slot) in slots {
            strip = strip.child(slot);
        }

        strip.child(self.render_add_button(cx))
    }

    /// A tab of the strip.
    fn render_tab(
        &self,
        index: usize,
        entry: &TabEntry,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let scale = self.scale;
        let theme = cx.theme();
        let background = theme.tab;
        let active_background = theme.tab_active;
        let foreground = theme.tab_foreground;
        let active_foreground = theme.tab_active_foreground;
        let hover = theme.muted;
        let border = theme.border;
        let accent = theme.accent;
        let accent_foreground = theme.accent_foreground;

        let active = entry.chat_id == self.active_chat_id;
        let dragging = self.dragging() == Some(entry.chat_id.as_str());
        let chat_id = entry.chat_id.clone();
        let click_chat_id = chat_id.clone();
        let aux_chat_id = chat_id.clone();
        let title = entry.title.clone();
        let payload = TabDragPayload {
            chat_id: chat_id.clone(),
            title: title.clone(),
        };

        // El ancho real de la pestaña, guardado para que el cierre arranque
        // justo desde ahi y no desde el minimo.
        let widths = self.widths.clone();
        let measured_id = chat_id.clone();

        div()
            .on_children_prepainted(move |bounds, _window, _cx| {
                if let Some(bounds) = bounds.first() {
                    widths
                        .borrow_mut()
                        .insert(measured_id.clone(), f32::from(bounds.size.width));
                }
            })
            .id(format!("tab-slot-{chat_id}"))
            .relative()
            .flex_1()
            .min_w(px(TAB_MIN_WIDTH * scale))
            .max_w(px(TAB_MAX_WIDTH * scale))
            .child(
                div()
                    .id(format!("tab-{chat_id}"))
                    .flex()
                    .w_full()
                    .h_full()
                    .items_center()
                    .gap(px(7. * scale))
                    .pl(px(9. * scale))
                    .pr(px(30. * scale))
                    .rounded_t(px(6. * scale))
                    .border(px(1.))
                    .border_b(px(0.))
                    .border_color(if active { border } else { transparent_black() })
                    .bg(if active {
                        active_background
                    } else {
                        background
                    })
                    .text_color(if active {
                        active_foreground
                    } else {
                        foreground
                    })
                    .cursor_pointer()
                    .when(dragging, |this| this.opacity(0.5))
                    .when(!active, |this| {
                        this.hover(|style| style.bg(hover).text_color(active_foreground))
                    })
                    .on_click(cx.listener(move |this, _event, window, cx| {
                        this.select(&click_chat_id);
                        let action = TabAction::Select {
                            chat_id: click_chat_id.clone(),
                        };
                        this.report(action, window, cx);
                        cx.notify();
                    }))
                    .on_aux_click(cx.listener(move |this, event, window, cx| {
                        // port: the oracle closes the tab with the middle
                        // button of the mouse.
                        let ClickEvent::Mouse(mouse) = event else {
                            return;
                        };
                        if mouse.down.button != MouseButton::Middle {
                            return;
                        }
                        this.close_and_report(&aux_chat_id, window, cx);
                        // El fantasma del cierre empieza en el proximo cuadro.
                        cx.notify();
                    }))
                    .on_drag(payload, |payload, _offset, _window, cx| {
                        cx.new(|_| TabDragPreview {
                            title: payload.title.clone(),
                        })
                    })
                    .on_drag_move::<TabDragPayload>(cx.listener(
                        move |this: &mut TabStrip,
                              event: &DragMoveEvent<TabDragPayload>,
                              _window,
                              cx| {
                            let Some(payload) =
                                event.dragged_item().downcast_ref::<TabDragPayload>()
                            else {
                                return;
                            };
                            let position = event.event.position;
                            let bounds = event.bounds;
                            if position.x < bounds.origin.x
                                || position.x > bounds.origin.x + bounds.size.width
                            {
                                return;
                            }
                            let right_half = position.x - bounds.origin.x > bounds.size.width / 2.;
                            let target = TabStrip::drop_index_for(index, right_half);
                            if this.drag_over(&payload.chat_id, target) {
                                cx.notify();
                            }
                        },
                    ))
                    .child(tab_avatar(entry, px(AVATAR_SIZE * scale), cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(TITLE_SIZE * scale))
                            .child(title),
                    )
                    .when(entry.unread_count > 0, |this| {
                        this.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .min_w(px(BADGE_SIZE * scale))
                                .h(px(BADGE_SIZE * scale))
                                .px(px(4. * scale))
                                .rounded_full()
                                .bg(accent)
                                .text_color(accent_foreground)
                                .text_size(px(BADGE_FONT_SIZE * scale))
                                .font_weight(FontWeight::BOLD)
                                .child(entry.unread_count.min(BADGE_MAX).to_string()),
                        )
                    }),
            )
            .child(self.render_close_button(entry, cx))
    }

    /// El fantasma de una pestaña que se cierra.
    ///
    /// Se dibuja con el ancho que tenia y se encoge hasta cero: la vecina de
    /// la derecha ocupa ese lugar cuadro a cuadro, sin saltar. La animacion
    /// respeta el movimiento reducido del sistema.
    fn render_closing_tab(&self, closing: &ClosingTab, cx: &mut Context<Self>) -> AnyElement {
        let scale = self.scale;
        let theme = cx.theme();
        let entry = &closing.entry;
        let width = closing.width;
        // El fantasma conserva el aspecto que la pestaña tenia: si era la que
        // se estaba mirando, no cambia de color justo al desaparecer.
        let background = if closing.active {
            theme.tab_active
        } else {
            theme.tab
        };
        let foreground = if closing.active {
            theme.tab_active_foreground
        } else {
            theme.tab_foreground
        };
        let border = if closing.active {
            theme.border
        } else {
            transparent_black()
        };
        // El contenido conserva su ancho mientras la caja se encoge: asi el
        // texto no se reacomoda y el cierre se ve como un recorte.
        let content = div()
            .flex()
            .flex_shrink_0()
            .w(px(width))
            .h_full()
            .items_center()
            .gap(px(7. * scale))
            .pl(px(9. * scale))
            .pr(px(30. * scale))
            .rounded_t(px(6. * scale))
            .border(px(1.))
            .border_b(px(0.))
            .border_color(border)
            .bg(background)
            .text_color(foreground)
            .child(tab_avatar(entry, px(AVATAR_SIZE * scale), cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(TITLE_SIZE * scale))
                    .child(entry.title.clone()),
            )
            .when(entry.unread_count > 0, |this| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .min_w(px(BADGE_SIZE * scale))
                        .h(px(BADGE_SIZE * scale))
                        .px(px(4. * scale))
                        .rounded_full()
                        .bg(theme.accent)
                        .text_color(theme.accent_foreground)
                        .text_size(px(BADGE_FONT_SIZE * scale))
                        .font_weight(FontWeight::BOLD)
                        .child(entry.unread_count.min(BADGE_MAX).to_string()),
                )
            });
        // Salida: arranca rapido y frena suave, la misma curva que el resto de
        // las transiciones de la app. La opacidad baja con el mismo reloj que
        // el ancho, asi la pestaña no queda translucida de mas.
        let animation =
            Animation::new(TAB_CLOSE_DURATION).with_easing(|t: f32| 1.0 - (1.0 - t).powi(2));
        div()
            .flex_none()
            .h_full()
            .overflow_hidden()
            .child(content)
            .with_animation(
                ElementId::named_usize("tab-closing", closing.serial),
                animation,
                move |slot, progress| {
                    let left = 1.0 - progress;
                    slot.w(px(width * left)).opacity(left)
                },
            )
            .into_any_element()
    }

    /// Aparta una pestaña del riel y la deja como fantasma mientras se cierra.
    fn start_closing(&mut self, chat_id: &str, index: usize) {
        let Some(entry) = self.tabs.get(index).cloned() else {
            return;
        };
        let width = self
            .widths
            .borrow()
            .get(chat_id)
            .copied()
            .unwrap_or(TAB_MIN_WIDTH * self.scale);
        self.closing_serial += 1;
        self.closing.push(ClosingTab {
            entry,
            index,
            active: self.active_chat_id == chat_id,
            width,
            started_at: Instant::now(),
            serial: self.closing_serial,
        });
    }

    /// Saca los fantasmas cuya animacion ya termino.
    fn prune_closing(&mut self) {
        self.closing
            .retain(|closing| closing.started_at.elapsed() < TAB_CLOSE_DURATION);
    }

    /// The button that closes a tab.
    fn render_close_button(&self, entry: &TabEntry, cx: &mut Context<Self>) -> impl IntoElement {
        let scale = self.scale;
        let theme = cx.theme();
        let foreground = theme.foreground;
        let hover = theme.muted;
        let chat_id = entry.chat_id.clone();

        div()
            .id(format!("tab-close-{chat_id}"))
            .aria_label(format!("Close {}", entry.title))
            .absolute()
            .top_0()
            .bottom_0()
            .right(px(6. * scale))
            .w(px(CLOSE_SIZE * scale))
            .flex()
            .items_center()
            .justify_center()
            .opacity(0.55)
            .cursor_pointer()
            .hover(|style| style.bg(hover).text_color(foreground))
            .on_click(cx.listener(move |this, _event, window, cx| {
                // El riel entrega el clic a cada caja que lo contiene, no solo
                // a la de arriba: sin esto, el clic de la X llegaba tambien a
                // la pestaña, que volvia a abrir el chat y deshacia el cierre.
                cx.stop_propagation();
                this.close_and_report(&chat_id, window, cx);
                // El fantasma del cierre empieza en el proximo cuadro.
                cx.notify();
            }))
            .child(Icon::new(IconName::Close).with_size(px(CLOSE_ICON_SIZE * scale)))
    }

    /// The button that opens a new tab.
    fn render_add_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let scale = self.scale;
        let theme = cx.theme();
        let foreground = theme.foreground;
        let muted = theme.muted_foreground;
        let hover = theme.muted;

        div()
            .id("tab-add")
            .flex_shrink_0()
            .self_center()
            .size(px(ADD_SIZE * scale))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6. * scale))
            .text_color(muted)
            .cursor_pointer()
            .hover(|style| style.bg(hover).text_color(foreground))
            .on_click(cx.listener(|this, _event, window, cx| {
                this.report(TabAction::Add, window, cx);
            }))
            .child(Icon::new(IconName::Plus).with_size(px(ADD_ICON_SIZE * scale)))
    }

    /// Closes a tab and tells the app about it.
    fn close_and_report(&mut self, chat_id: &str, window: &mut Window, cx: &mut App) {
        let title = self.title_of(chat_id);
        let Some(close) = self.close(chat_id) else {
            return;
        };
        let action = TabAction::Close {
            chat_id: close.chat_id,
            title: title.clone(),
            next_chat_id: close.next_chat_id,
            message: format!("{title} closed"),
        };
        self.report(action, window, cx);
    }

    /// Tells the app about a gesture of the strip.
    fn report(&self, action: TabAction, window: &mut Window, cx: &mut App) {
        if let Some(handler) = &self.on_action {
            handler(action, window, cx);
        }
    }
}

impl Render for TabStrip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        TabStrip::render(self, cx)
    }
}

/// The avatar of a tab.
///
/// port: the `Avatar` of the oracle: the photo of the chat, the initials of
/// the name, and the people of the group when the chat has no photo.
fn tab_avatar(entry: &TabEntry, size: Pixels, cx: &App) -> gpui::Div {
    if entry.group && entry.avatar_path.is_empty() {
        return div()
            .flex_shrink_0()
            .size(size)
            .rounded_full()
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center()
            .bg(cx.theme().secondary)
            .text_color(cx.theme().muted_foreground)
            .child(Icon::new(IconName::User).with_size(size * 0.55));
    }
    avatar_element(&entry.title, &entry.avatar_path, size, cx)
}

/// The floating copy of the tab that is being dragged.
struct TabDragPreview {
    title: String,
}

impl Render for TabDragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let background = theme.popover;
        let foreground = theme.popover_foreground;
        let border = theme.border;
        div()
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .border(px(1.))
            .border_color(border)
            .bg(background)
            .text_color(foreground)
            .text_sm()
            .child(self.title.clone())
    }
}

/// The name of a key, as the web calls it.
///
/// port: the oracle reads the `key` of the DOM event ("Delete",
/// "ArrowLeft", "ArrowRight", "Home", "End"); GPUI names the same keys
/// "delete", "left", "right", "home" and "end".
fn web_key(key: &str) -> String {
    match key {
        "delete" => "Delete".to_string(),
        "left" => "ArrowLeft".to_string(),
        "right" => "ArrowRight".to_string(),
        "up" => "ArrowUp".to_string(),
        "down" => "ArrowDown".to_string(),
        "home" => "Home".to_string(),
        "end" => "End".to_string(),
        "escape" => "Escape".to_string(),
        "enter" => "Enter".to_string(),
        "backspace" => "Backspace".to_string(),
        "tab" => "Tab".to_string(),
        other => other.to_string(),
    }
}

/// The gesture of a key press of the strip.
///
/// port: the `TabKeyboardInput` of the oracle: the key, the alt, the shift
/// and the platform modifier (the control or the command of the keyboard).
fn keyboard_input(event: &KeyDownEvent) -> TabKeyboardInput {
    let modifiers = event.keystroke.modifiers;
    TabKeyboardInput {
        key: web_key(&event.keystroke.key),
        alt_key: modifiers.alt,
        shift_key: modifiers.shift,
        platform_modifier: modifiers.control || modifiers.platform,
    }
}

/// The name of a direction, as the oracle writes it.
fn direction_name(direction: TabDirection) -> &'static str {
    match direction {
        TabDirection::Left => "left",
        TabDirection::Right => "right",
    }
}

