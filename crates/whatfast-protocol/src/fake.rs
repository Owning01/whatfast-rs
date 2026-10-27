//! The deterministic fake backend of the protocol.
//!
//! Port of the fake server of `desktop/src/bridge.rs` of the origin repo.
//! The origin kept the simulator inside its GPUI client: it answered the
//! fifty RPCs of the protocol with a deterministic dataset (ten thousand
//! chats, a thousand messages per chat, photos, stickers, reactions, polls
//! and newsletters) so the interface could be developed and tested without a
//! WhatsApp account and without the network.
//!
//! Here the simulator is a server of the protocol: it takes the requests of
//! the client, answers them with the same payloads and publishes the events
//! on the same channel the real sidecar uses. The dataset is deterministic
//! (indices and hashes, no clock and no randomness), so the visual QA of the
//! native UI has a stable reference.
//!
//! Three decisions of this port:
//!
//! - The crate does not invent paths: the media of the fake (the photos and
//!   the audio) is written under the profile directory the binary resolved
//!   and passes in, and the files are private to their owner (`0600`).
//! - The photos are written by a small PNG encoder of this module instead of
//!   by the `image` crate, because the crate may not add a dependency that
//!   is not authorized. The pixels are the ones of the origin.
//! - The dataset is a demonstration of the north of Argentina (decision
//!   asked by the owner, 2026-10): a roster of people with real names,
//!   avatars painted as portraits, photos painted as scenes, messages in
//!   Spanish and melodies instead of a plain tone. The dataset stays
//!   deterministic and the oracle tests follow the roster.

use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::transport::{BridgeMessage, PROTOCOL_VERSION};
use crate::{
    AttachmentContent, AttachmentKind, AuthStateResponse, BackendEvent, CallAcceptResponse,
    CallAudioResponse, CallHangupResponse, CallMuteResponse, CallPlaceResponse, CallRejectResponse,
    Chat, ChatKind, ChatParticipant, ChatUpserted, ConnectionState, Envelope,
    GetChatHistoryCoverageResponse, GetChatInfoResponse, GetHistoryOverviewResponse,
    GetIntegrityStatusResponse, GetMessageAttachmentResponse, GetMessageImageResponse,
    HelloResponse, HistoryCoverage, HistoryCoverageChanged, HistoryCoverageState, HistoryOverview,
    ImageContent, IntegrityStatus, LinkPreview, ListChatsResponse, ListMessagesAfterResponse,
    ListMessagesAroundResponse, ListMessagesResponse, ListNewsletterMessagesResponse,
    ListNewslettersResponse, ListPinnedMessagesResponse, ListStatusesResponse,
    ListStickersResponse, Message, MessageSearchResult, MessageStatus, MessageUpserted, Newsletter,
    OpenContactResponse, OpenMessageWindowResponse, PinnedMessage, PinnedMessagesChanged,
    PollContent, PollOption, PollVoter, Reaction, RpcError, RpcRequest, RpcResponse,
    SearchLocalResponse, SendAttachmentResponse, SendImageResponse, SendReactionResponse,
    SendStickerFromLibraryResponse, SendStickerResponse, SendTextResponse, Sticker, StickerPack,
    SubscribeNewsletterResponse, SyncPhase, SyncStatus, TextContent, backend_event, envelope,
    message as proto_message, rpc_request, rpc_response,
};

/// How many chats the fake has.
const TOTAL_CHATS: usize = 10_000;

/// How many messages a chat has in the default fixture.
const MESSAGE_TOTAL: usize = 1_000;

/// How many messages a chat has in the performance fixture.
const PERF_MESSAGE_TOTAL: usize = 2_000;

/// Every how many messages the dataset has a document.
///
/// The photo has its own stride (every 37 messages) and these four strides
/// are coprime between them and with the one of the photo, so no message of
/// the dataset carries two files.
const DOCUMENT_EVERY: usize = 59;

/// Every how many messages the dataset has a video.
const VIDEO_EVERY: usize = 73;

/// Every how many messages the dataset has an audio file.
const AUDIO_EVERY: usize = 67;

/// Every how many messages the dataset has a voice note.
const VOICE_NOTE_EVERY: usize = 97;

/// The name of the document of the dataset.
const DOCUMENT_FILE: &str = "informe.pdf";

/// The name of the video of the dataset.
const VIDEO_FILE: &str = "vacaciones.mp4";

/// The name of the audio file of the dataset.
const AUDIO_FILE: &str = "cancion.wav";

/// The name of the voice note of the dataset.
const VOICE_NOTE_FILE: &str = "nota-de-voz.wav";

// ============================================================================
// The roster of the demonstration
//
// The first screens of the list are people and groups of the north of
// Argentina, with the phones of a demonstration (the prefix 5501 does not
// exist: it plays the role of the 555 of the fiction of the United States).
// The chats beyond the roster combine the two pools of names, so every row
// of the ten thousand still reads like a real conversation.
// ============================================================================

/// A person of the roster.
struct Person {
    /// The name the contact is saved with.
    name: &'static str,
    /// The push name: the one the person wrote on their phone.
    push: &'static str,
    /// The phone in the format the interface shows.
    phone: &'static str,
    /// The digits of the JID, without the domain.
    digits: &'static str,
}

/// The roster of the direct conversations.
const PEOPLE: [Person; 24] = [
    Person {
        name: "Martina Ferreyra",
        push: "Marti ♡",
        phone: "+54 9 381 550-101",
        digits: "549381550101",
    },
    Person {
        name: "Bruno Salazar",
        push: "Bruno ⚽",
        phone: "+54 9 381 550-102",
        digits: "549381550102",
    },
    Person {
        name: "Lucía Cabrera",
        push: "Luci ✿",
        phone: "+54 9 381 550-103",
        digits: "549381550103",
    },
    Person {
        name: "Facundo Ibarra",
        push: "Facu 🍕",
        phone: "+54 9 381 550-104",
        digits: "549381550104",
    },
    Person {
        name: "Camila Rojas",
        push: "Cami 🌻",
        phone: "+54 9 381 550-105",
        digits: "549381550105",
    },
    Person {
        name: "Nicolás Villalba",
        push: "Nico 🎸",
        phone: "+54 9 381 550-106",
        digits: "549381550106",
    },
    Person {
        name: "Valentina Paz",
        push: "Vale 📚",
        phone: "+54 9 381 550-107",
        digits: "549381550107",
    },
    Person {
        name: "Santiago Maldonado",
        push: "Santi 🏍️",
        phone: "+54 9 381 550-108",
        digits: "549381550108",
    },
    Person {
        name: "Isabela Correa",
        push: "Isa 🎨",
        phone: "+54 9 381 550-109",
        digits: "549381550109",
    },
    Person {
        name: "Matías Aráoz",
        push: "Mati 🐧",
        phone: "+54 9 381 550-110",
        digits: "549381550110",
    },
    Person {
        name: "Julieta Ocampo",
        push: "Juli 🐱",
        phone: "+54 9 381 550-111",
        digits: "549381550111",
    },
    Person {
        name: "Tomás Chávez",
        push: "Tomás 🎮",
        phone: "+54 9 381 550-112",
        digits: "549381550112",
    },
    Person {
        name: "Agustina Vera",
        push: "Agus 💃",
        phone: "+54 9 381 550-113",
        digits: "549381550113",
    },
    Person {
        name: "Ramiro Nieva",
        push: "Rama 🛠️",
        phone: "+54 9 381 550-114",
        digits: "549381550114",
    },
    Person {
        name: "Elena Bazán",
        push: "Elena ☕",
        phone: "+54 9 381 550-115",
        digits: "549381550115",
    },
    Person {
        name: "Joaquín Zavalía",
        push: "Joaco 🏀",
        phone: "+54 9 381 550-116",
        digits: "549381550116",
    },
    Person {
        name: "Paulina Terán",
        push: "Pau 🌙",
        phone: "+54 9 381 550-117",
        digits: "549381550117",
    },
    Person {
        name: "Ignacio Colombres",
        push: "Nacho 🎧",
        phone: "+54 9 381 550-118",
        digits: "549381550118",
    },
    Person {
        name: "Delfina Soria",
        push: "Delfi 🦋",
        phone: "+54 9 381 550-119",
        digits: "549381550119",
    },
    Person {
        name: "Marcos Padilla",
        push: "Marcos 🥓",
        phone: "+54 9 381 550-120",
        digits: "549381550120",
    },
    Person {
        name: "Catalina Ruiz",
        push: "Cata 🎤",
        phone: "+54 9 381 550-121",
        digits: "549381550121",
    },
    Person {
        name: "Lautaro Benítez",
        push: "Lauti 🚴",
        phone: "+54 9 381 550-122",
        digits: "549381550122",
    },
    Person {
        name: "Renata Mendoza",
        push: "Renata 🌷",
        phone: "+54 9 381 550-123",
        digits: "549381550123",
    },
    Person {
        name: "Gonzalo Ledesma",
        push: "Gonza 🍟",
        phone: "+54 9 381 550-124",
        digits: "549381550124",
    },
];

/// The groups of the first screens of the list.
const GROUPS: [&str; 10] = [
    "Familia Bazán 🏡",
    "Fútbol de los martes ⚽",
    "Amigas del gimnasio 💪",
    "Cumple de Bruno 🎉",
    "5° 3° — Ex alumnos 🎓",
    "Asado del finde 🥩",
    "Coordi del trabajo 📋",
    "Vecinos del 3°B 🏢",
    "Viaje a Tafí 2026 🏔️",
    "Mates y cine 🎬🧉",
];

/// The description of each group of the pool.
const GROUP_DESCRIPTIONS: [&str; 10] = [
    "La familia, para coordinar cumpleaños y asados 🏡",
    "Se juega los martes a las 21, el que falta paga las medias ⚽",
    "Turno de las 7 de la mañana, frases motivacionales prohibidas 💪",
    "Para organizar el cumple de Bruno, sugerencias bienvenidas 🎉",
    "El grupo del colegio, puros recuerdos 🎓",
    "El asado del finde: cada uno trae lo suyo 🥩",
    "Avisos del trabajo, solo en horario laboral 📋",
    "Los vecinos del 3°B, temas del edificio 🏢",
    "Ahorramos para Tafí del Valle, la lista de gastos vive acá 🏔️",
    "Cine en mi casa el jueves, traigan pochoclos 🎬",
];

/// First names of the conversations beyond the roster.
const FIRSTS: [&str; 20] = [
    "Micaela",
    "Emiliano",
    "Belén",
    "Franco",
    "Antonella",
    "Leandro",
    "Josefina",
    "Bautista",
    "Milagros",
    "Esteban",
    "Antonina",
    "Salvador",
    "Pilar",
    "Alfredo",
    "Guadalupe",
    "Aníbal",
    "Amparo",
    "Benjamín",
    "Cristina",
    "Alvaro",
];

/// Last names of the conversations beyond the roster.
const LASTS: [&str; 20] = [
    "Peredo",
    "Garmendia",
    "Luna",
    "Quinteros",
    "Moyano",
    "Alurralde",
    "Piossek",
    "Gramajo",
    "Bulacio",
    "Cruz",
    "Alderete",
    "Pereyra",
    "Zamora",
    "Avellaneda",
    "Ríos",
    "Leguizamón",
    "Ordoñez",
    "Frías",
    "Noguera",
    "Vergara",
];

/// The messages the demonstration sends as you.
const MY_TEXTS: [&str; 14] = [
    "Listo, ahí te aviso 👍",
    "Jajaja no puede ser 😂",
    "Perfecto, gracias!",
    "Dale, nos vemos a las 8 entonces",
    "Te paso la dirección en un rato",
    "Ya salí de la oficina 🚗",
    "Buenísimo 😄",
    "Charlamos mañana que me voy a dormir 😴",
    "No te olvides de traer el mate 🧉",
    "Ok, entendido",
    "Marchando 🫡",
    "¿Cuánto les quedó por el alquiler?",
    "¡Feliz cumple! 🎂🎉",
    "¿Te queda lugar en el auto?",
];

/// The messages the demonstration answers with.
const THEIR_TEXTS: [&str; 14] = [
    "¡Hola! ¿Cómo andás?",
    "¿Viste el gol de anoche? Insolente 😱",
    "Mañana hay reunión a las 10, ¿estás?",
    "Mirá esta foto que me pasó mi hermana 😍",
    "¿Y el asado del finde se hace o no?",
    "Recién llego a casa 🏠",
    "Tengo una noticia para contarles 👀",
    "¿Me pasás el número de ese mecánico?",
    "Qué calor que hace hoy 🥵",
    "Ya le dije que sí, no insistas jaja",
    "Estoy por salir, ¿necesitás algo?",
    "Gracias por ayer de verdad 🙏",
    "Se cortó la luz acá otra vez 💡",
    "Dale, estoy con eso 📚",
];

/// The long message of the stride of nine: several lines to exercise the
/// variable-height virtualization.
const LONG_TEXT: &str = "Te paso el dato del cumple 🎉\nSábado, desde las 21\nEn lo de Male (Av. Alem 1234)\nTraé hielo si podés 🧊";

/// The texts the live loop streams into the first chat.
const LIVE_TEXTS: [&str; 8] = [
    "¿Alguno vio la llave del quincho?",
    "Ya estoy llegando, cinco minutos ⏱️",
    "Jajaja mirá lo que encontré 😂",
    "Mañana llevan los mates, yo pongo el azúcar 🧉",
    "Se suspende por lluvia ☔ aviso por acá",
    "Gracias por el aguante de ayer ❤️",
    "¿Alguien tiene el número de la pizzería?",
    "¡Buen finde gente! 🎉",
];

/// The posts of the channels of the demonstration.
const CHANNEL_POSTS: [&str; 6] = [
    "📊 Resumen de la semana: lo que no te podés perder",
    "🎉 Ya estamos sorteando dos entradas entre los suscriptores",
    "🆕 Nueva nota: cómo hacer que tu app abra en un parpadeo",
    "❓ Hilo de consultas abierto, pregunten lo que quieran",
    "🎁 Beneficio de siempre: 20% en la librería del centro",
    "📸 Detrás de escena de la última actualización",
];

/// The about texts of the direct contacts.
const ABOUT_TEXTS: [&str; 3] = [
    "¡Hola! Estoy usando Whatfast.",
    "Disponible 📱",
    "Dejá el mensaje que en un rato contesto 📝",
];

/// The verified names of the business contacts.
const VERIFIED_TEXTS: [&str; 3] = ["Panadería La Espiga", "Vivero El Patio", "Librería Nazca"];

/// The captions of the photos of the dataset, one per scene: the caption of
/// a photo and the scene it paints come from the same table.
const PHOTO_CAPTIONS: [&str; 8] = [
    "Atardecer en el dique 🌅",
    "Cafecito de la mañana ☕",
    "La michi de la vecina 😺",
    "Vista desde el cerro 🏔️",
    "Noche en San Miguel 🌃",
    "La pizza de anoche 🍕",
    "Flores del patio 🌸",
    "Tardeo en la playa 🏖️",
];

/// The "you" of the demonstration: the user the profile belongs to.
const ME_NAME: &str = "Octavio González";

/// The phone of the "you" of the demonstration.
const ME_PHONE: &str = "+54 9 381 550-100";

/// The digits of the JID of the "you" of the demonstration.
const ME_DIGITS: &str = "549381550100";

/// Whether the chat of the index is a group.
fn is_group_id(id: usize) -> bool {
    id.is_multiple_of(5)
}

/// The group index of a chat id.
fn group_index(id: usize) -> usize {
    id / 5
}

/// The title of the direct conversation of the index.
///
/// The first chats take the people of the roster; the ones beyond combine
/// the pools with strides coprime to the pool size, so two adjacent chats
/// never share a last name.
fn direct_title(id: usize) -> String {
    if id < PEOPLE.len() {
        PEOPLE[id].name.into()
    } else {
        format!(
            "{} {}",
            FIRSTS[(id * 11 + 3) % FIRSTS.len()],
            LASTS[(id * 13 + 7) % LASTS.len()]
        )
    }
}

/// The push name of the direct conversation of the index.
fn direct_push(id: usize) -> String {
    if id < PEOPLE.len() {
        PEOPLE[id].push.into()
    } else {
        FIRSTS[(id * 11 + 3) % FIRSTS.len()].into()
    }
}

/// The phone of the direct conversation of the index.
fn direct_phone(id: usize) -> String {
    if id < PEOPLE.len() {
        PEOPLE[id].phone.into()
    } else {
        format!("+54 9 381 6{:05}", id % 100_000)
    }
}

/// The digits of the JID of the direct conversation of the index.
fn direct_digits(id: usize) -> String {
    if id < PEOPLE.len() {
        PEOPLE[id].digits.into()
    } else {
        format!("5493816{:05}", id % 100_000)
    }
}

/// The title of the group of the index.
fn group_title(group: usize) -> String {
    if group < GROUPS.len() {
        GROUPS[group].into()
    } else {
        match group % 3 {
            0 => format!("Familia {}", LASTS[(group / 3) % LASTS.len()]),
            1 => format!("{} y amigos", FIRSTS[(group / 3) % FIRSTS.len()]),
            _ => format!("Grupo de {}", FIRSTS[(group * 7) % FIRSTS.len()]),
        }
    }
}

/// The description of the group of the index.
fn group_description(group: usize) -> String {
    if group < GROUP_DESCRIPTIONS.len() {
        GROUP_DESCRIPTIONS[group].into()
    } else {
        "Grupo de coordinación.".into()
    }
}

/// A member of a group: the roster gives the people, the group only decides
/// where the window of the roster starts.
fn group_member(group: usize, index: usize) -> &'static Person {
    &PEOPLE[(group * 7 + index * 3) % PEOPLE.len()]
}

/// The chat id as the number it carries, when it carries one.
fn chat_number(chat_id: &str) -> Option<usize> {
    chat_id
        .strip_prefix("chat-")
        .and_then(|value| value.parse::<usize>().ok())
}

/// Who signs the messages of a chat that are not yours.
///
/// In a group the members take turns by message; in a direct conversation
/// the person of the chat signs them all.
struct Sender {
    name: String,
    phone: String,
    jid: String,
}

/// Resolve who signs a message of a chat: a member of the group or the
/// person of the direct conversation.
fn chat_sender(chat_id: &str, message_id: usize) -> Sender {
    let Some(id) = chat_number(chat_id) else {
        let person = &PEOPLE[1];
        return Sender {
            name: person.name.into(),
            phone: person.phone.into(),
            jid: format!("{}@s.whatsapp.net", person.digits),
        };
    };
    let person = if is_group_id(id) {
        group_member(group_index(id), message_id % 12)
    } else if id < PEOPLE.len() {
        &PEOPLE[id]
    } else {
        // The conversations beyond the roster have no Person: their sender
        // carries the same name the title carries.
        return Sender {
            name: direct_title(id),
            phone: direct_phone(id),
            jid: format!("{}@s.whatsapp.net", direct_digits(id)),
        };
    };
    Sender {
        name: person.name.into(),
        phone: person.phone.into(),
        jid: format!("{}@s.whatsapp.net", person.digits),
    }
}

/// The text of a message of the dataset: the pool depends on the sender.
fn message_text(id: usize) -> &'static str {
    if id.is_multiple_of(2) {
        MY_TEXTS[(id / 2) % MY_TEXTS.len()]
    } else {
        THEIR_TEXTS[(id / 2) % THEIR_TEXTS.len()]
    }
}

/// The caption of the photo of the dataset: the photo index decides the
/// scene, and the caption matches the scene.
fn photo_caption(id: usize) -> &'static str {
    PHOTO_CAPTIONS[(id / 37) % PHOTO_CAPTIONS.len()]
}

/// How many participants a group has in the performance fixture.
const PERF_PARTICIPANTS: usize = 1_000;

/// The depth of the queue of the requests of the client.
const OUTGOING_CAPACITY: usize = 256;

/// The depth of the queue of the answers and the events of the fake.
const INCOMING_CAPACITY: usize = 1024;

/// The wait of the live loop between the pairs of events.
const LIVE_EVENT_INTERVAL: Duration = Duration::from_secs(4);

/// The wait of the live loop before the first pair of events.
const LIVE_EVENT_DELAY: Duration = Duration::from_secs(3);

/// Why the fake backend refused a request.
#[derive(Debug, thiserror::Error)]
pub enum FakeError {
    /// The queue of the requests is full or closed.
    #[error("the fake backend's request queue is full or closed")]
    QueueUnavailable,
    /// The request could not be registered in the table of the pending ones.
    #[error("the request could not be registered: {0}")]
    Request(#[from] crate::transport::TransportError),
}

/// The configuration of the fake backend.
#[derive(Clone, Debug)]
pub struct FakeConfig {
    /// The profile directory of the user.
    ///
    /// The fake writes its media under `<profile_dir>/media`: the crate does
    /// not invent paths, the binary passes the profile it resolved.
    pub profile_dir: PathBuf,
    /// The fake answers as an unpaired client and emits a pairing QR.
    pub pairing: bool,
    /// The performance fixture: the chats have more messages and the groups
    /// have more participants.
    pub performance_fixture: bool,
    /// Whether the live loop publishes a message and a chat update every
    /// four seconds once the handshake is done.
    pub live_events: bool,
}

impl FakeConfig {
    /// The configuration the origin read from the environment.
    ///
    /// `WHATFAST_FAKE_PAIRING` makes the fake answer as an unpaired client,
    /// `WHATFAST_PERF_FIXTURE` swaps the dataset for the performance one and
    /// `WHATFAST_FAKE_LIVE_EVENTS=0` turns the live loop off.
    pub fn from_env(profile_dir: impl Into<PathBuf>) -> Self {
        Self {
            profile_dir: profile_dir.into(),
            pairing: env::var_os("WHATFAST_FAKE_PAIRING").is_some(),
            performance_fixture: env::var_os("WHATFAST_PERF_FIXTURE").is_some(),
            live_events: env::var("WHATFAST_FAKE_LIVE_EVENTS").as_deref() != Ok("0"),
        }
    }

    /// The media tree of the profile: the only tree the asset protocol
    /// serves.
    pub fn media_dir(&self) -> PathBuf {
        self.profile_dir.join("media")
    }
}

/// A running fake backend: the same surface the sidecar transport exposes.
pub struct FakeBackend {
    outgoing: async_channel::Sender<Envelope>,
    /// The ordered stream of the answers and the events of the fake.
    pub incoming: async_channel::Receiver<BridgeMessage>,
}

impl FakeBackend {
    /// Start the fake backend with the configuration of the environment.
    pub fn start(profile_dir: impl Into<PathBuf>) -> Self {
        Self::with_config(FakeConfig::from_env(profile_dir))
    }

    /// Start the fake backend with an explicit configuration.
    pub fn with_config(config: FakeConfig) -> Self {
        let (outgoing, outgoing_rx) = async_channel::bounded(OUTGOING_CAPACITY);
        let (incoming_tx, incoming) = async_channel::bounded(INCOMING_CAPACITY);
        let event_sequence = Arc::new(Mutex::new(1_u64));
        let handshaken = Arc::new(AtomicBool::new(false));
        if config.live_events {
            let live_incoming = incoming_tx.clone();
            let live_sequence = event_sequence.clone();
            let live_handshaken = handshaken.clone();
            // A thread that cannot be spawned leaves the live loop out: the
            // client still gets the answers of the requests.
            let _ = thread::Builder::new()
                .name("fake-live-events".into())
                .spawn(move || fake_live_loop(live_incoming, live_sequence, live_handshaken));
        }
        let worker_sequence = event_sequence.clone();
        let worker_handshaken = handshaken.clone();
        // Same: a thread that cannot be spawned leaves the queue closed and
        // the client sees the backend as gone, like a sidecar that died.
        let _ = thread::Builder::new()
            .name("fake-backend".into())
            .spawn(move || {
                fake_loop(
                    outgoing_rx,
                    incoming_tx,
                    worker_sequence,
                    worker_handshaken,
                    config,
                )
            });
        Self { outgoing, incoming }
    }

    /// Queue a request for the fake backend.
    pub fn send(&self, request_id: u64, request: rpc_request::Request) -> Result<(), FakeError> {
        self.outgoing
            .try_send(Envelope {
                protocol_version: PROTOCOL_VERSION,
                request_id,
                body: Some(envelope::Body::Request(RpcRequest {
                    request: Some(request),
                })),
            })
            .map_err(|_| FakeError::QueueUnavailable)
    }

    /// Send a request under a deadline and remember it until it answers.
    ///
    /// The fake is a drop-in of the sidecar transport for the client: the
    /// table of the pending requests and the deadlines are the ones of the
    /// transport, so the UI can talk to the fake and to the sidecar with the
    /// same code.
    pub fn send_request(
        &self,
        tracker: &mut crate::transport::RequestTracker,
        class: crate::transport::RequestClass,
        request: rpc_request::Request,
        now: Duration,
    ) -> Result<u64, FakeError> {
        let request_id = tracker.begin(class, now)?;
        if let Err(error) = self.send(request_id, request) {
            // A request that could not be queued is not pending anymore.
            tracker.complete(request_id);
            return Err(error);
        }
        Ok(request_id)
    }
}

/// The coverage of the history of a chat, as the fake reports it.
pub fn fake_history_coverage(chat_id: String) -> HistoryCoverage {
    HistoryCoverage {
        chat_id,
        state: HistoryCoverageState::Unknown as i32,
        revision: 1,
        local_message_count: 1_000,
        oldest_message_timestamp_ms: 1_700_000_000_000,
        on_demand_messages_added: 0,
        last_requested_count: 0,
        last_received_count: 0,
        last_added_count: 0,
        last_requested_at_ms: 0,
        retry_after_ms: 0,
        detail: "Older WhatsApp history has not been checked".into(),
    }
}

/// The integrity of the local database, as the fake reports it.
pub fn fake_integrity_status() -> IntegrityStatus {
    IntegrityStatus {
        database_ok: true,
        detail: "Local database and derived chat state are consistent".into(),
        pending_inbound: 0,
        failed_inbound: 0,
        oldest_pending_at_ms: 0,
        last_message_persisted_at_ms: 1_700_000_000_000,
        last_reconciliation_at_ms: 1_700_000_000_000,
        inconsistent_chats: 0,
    }
}

/// Answer the requests of the client until the queue is closed.
///
/// The origin's fake was the simulator of the GPUI client: it took the
/// envelopes of the client, answered every request of the protocol and
/// published the events on the same stream.
pub fn fake_loop(
    outgoing: async_channel::Receiver<Envelope>,
    incoming: async_channel::Sender<BridgeMessage>,
    event_sequence: Arc<Mutex<u64>>,
    handshaken: Arc<AtomicBool>,
    config: FakeConfig,
) {
    let pairing = config.pairing;
    let performance_fixture = config.performance_fixture;
    let profile_dir = config.profile_dir.clone();
    let message_total = if performance_fixture {
        PERF_MESSAGE_TOTAL
    } else {
        MESSAGE_TOTAL
    };
    // In-memory pins so ListPinnedMessages reflects what SetMessagePin
    // stored, matching the real backend instead of always returning empty.
    // NOTE: pins live only for the lifetime of this fake client loop (per
    // fake backend instance) and are lost on restart; the real backend
    // persists them. This is intentional for the deterministic fixture.
    let mut pins: HashMap<String, Vec<PinnedMessage>> = HashMap::new();
    // In-memory chat settings so the fake mirrors the real backend: archive,
    // mute and block survive for the lifetime of the fake client loop and are
    // echoed back by the setter RPCs and by ListChats.
    let mut chat_settings: HashMap<String, Chat> = HashMap::new();
    while let Ok(envelope) = outgoing.recv_blocking() {
        let request_id = envelope.request_id;
        let Some(envelope::Body::Request(request)) = envelope.body else {
            continue;
        };
        let is_hello = matches!(
            request.request.as_ref(),
            Some(rpc_request::Request::Hello(_))
        );
        let result = match request.request {
            Some(rpc_request::Request::Hello(_)) => rpc_response::Result::Hello(HelloResponse {
                backend_version: "fake-0.1".into(),
                protocol_version: PROTOCOL_VERSION,
            }),
            Some(rpc_request::Request::GetAuthState(_)) => {
                rpc_response::Result::AuthState(AuthStateResponse {
                    paired: !pairing,
                    logged_in: !pairing,
                    own_user_id: "me@s.whatsapp.net".into(),
                    connection_state: if pairing {
                        ConnectionState::Pairing as i32
                    } else {
                        ConnectionState::Connected as i32
                    },
                })
            }
            Some(rpc_request::Request::StartPairing(_)) => {
                emit_fake_event(
                    &incoming,
                    &event_sequence,
                    backend_event::Event::PairingQr(crate::PairingQr {
                        code: "2@WHATFAST-FAKE-PAIRING-CODE".into(),
                        expires_at_ms: 4_102_444_800_000,
                    }),
                );
                rpc_response::Result::StartPairing(crate::StartPairingResponse { started: true })
            }
            Some(rpc_request::Request::GetSyncStatus(_)) => {
                rpc_response::Result::SyncStatus(crate::SyncStatusResponse {
                    status: Some(SyncStatus {
                        phase: SyncPhase::Complete as i32,
                        revision: 1,
                        chats_processed: 10_000,
                        messages_processed: message_total as u64,
                        messages_received: message_total as u64,
                        messages_decoded: message_total as u64,
                        messages_failed: 0,
                        whatsapp_progress: 100,
                        started_at_ms: 0,
                        completed_at_ms: 0,
                        detail: "Up to date".into(),
                    }),
                })
            }
            Some(rpc_request::Request::GetChatHistoryCoverage(request)) => {
                rpc_response::Result::GetChatHistoryCoverage(GetChatHistoryCoverageResponse {
                    coverage: Some(fake_history_coverage(request.chat_id)),
                })
            }
            Some(rpc_request::Request::RequestOlderHistory(request)) => {
                let mut coverage = fake_history_coverage(request.chat_id);
                coverage.state = HistoryCoverageState::Requesting as i32;
                coverage.detail = "Requesting 50 older messages from WhatsApp".into();
                let mut completed = coverage.clone();
                completed.state = HistoryCoverageState::Exhausted as i32;
                completed.revision += 1;
                completed.detail = "WhatsApp returned no older messages".into();
                emit_fake_event(
                    &incoming,
                    &event_sequence,
                    backend_event::Event::HistoryCoverageChanged(HistoryCoverageChanged {
                        coverage: Some(completed),
                    }),
                );
                rpc_response::Result::RequestOlderHistory(crate::RequestOlderHistoryResponse {
                    coverage: Some(coverage),
                })
            }
            Some(rpc_request::Request::GetHistoryOverview(_)) => {
                rpc_response::Result::GetHistoryOverview(GetHistoryOverviewResponse {
                    overview: Some(HistoryOverview {
                        local_message_count: message_total as u64,
                        chats_with_messages: 10_000,
                        chats_checked: 0,
                        chats_unknown: 10_000,
                        chats_without_anchor: 0,
                    }),
                })
            }
            Some(rpc_request::Request::GetIntegrityStatus(_)) => {
                rpc_response::Result::GetIntegrityStatus(GetIntegrityStatusResponse {
                    status: Some(fake_integrity_status()),
                })
            }
            Some(rpc_request::Request::RepairLocalCache(_)) => {
                rpc_response::Result::RepairLocalCache(crate::RepairLocalCacheResponse {
                    status: Some(fake_integrity_status()),
                })
            }
            Some(rpc_request::Request::ListChats(request)) => {
                let start = request.cursor.parse::<usize>().unwrap_or_default();
                let limit = request.limit.min(100) as usize;
                let total = TOTAL_CHATS;
                let chats = (start..(start + limit).min(total))
                    .map(fake_chat)
                    .map(|mut chat| {
                        if let Some(settings) = chat_settings.get(&chat.id) {
                            chat.pinned = settings.pinned;
                            chat.archived = settings.archived;
                            chat.muted = settings.muted;
                            chat.blocked = settings.blocked;
                            chat.mute_until_ms = settings.mute_until_ms;
                        }
                        chat
                    })
                    .collect();
                let next = if start + limit < total {
                    (start + limit).to_string()
                } else {
                    String::new()
                };
                rpc_response::Result::ListChats(ListChatsResponse {
                    chats,
                    total_count: total as u64,
                    next_cursor: next,
                })
            }
            Some(rpc_request::Request::ListMessages(request)) => {
                let end = if request.before_timestamp_ms == 0 {
                    message_total
                } else {
                    request.before_timestamp_ms as usize
                };
                let start = end.saturating_sub(request.limit as usize);
                let messages = (start..end)
                    .map(|id| fake_message(&request.chat_id, id, &profile_dir))
                    .collect();
                rpc_response::Result::ListMessages(ListMessagesResponse {
                    messages,
                    has_more: start > 0,
                })
            }
            Some(rpc_request::Request::OpenMessageWindow(request)) => {
                let anchor = message_total.saturating_sub(50);
                let start = anchor.saturating_sub(25);
                let end = (anchor + 26).min(message_total);
                rpc_response::Result::OpenMessageWindow(OpenMessageWindowResponse {
                    messages: (start..end)
                        .map(|id| fake_message(&request.chat_id, id, &profile_dir))
                        .collect(),
                    has_older: start > 0,
                    has_newer: end < message_total,
                    first_unread_message_id: format!("message-{anchor}"),
                })
            }
            Some(rpc_request::Request::ListMessagesAfter(request)) => {
                let start = request
                    .after_message_id
                    .strip_prefix("message-")
                    .and_then(|value| value.parse::<usize>().ok())
                    .map_or(0, |id| id + 1);
                let end = (start + request.limit as usize).min(message_total);
                rpc_response::Result::ListMessagesAfter(ListMessagesAfterResponse {
                    messages: (start..end)
                        .map(|id| fake_message(&request.chat_id, id, &profile_dir))
                        .collect(),
                    has_more: end < message_total,
                })
            }
            Some(rpc_request::Request::SearchLocal(request)) => {
                let query = request.query.to_lowercase();
                let contacts = (0..8)
                    .filter_map(|id| {
                        let chat = fake_chat(id);
                        (chat.title.to_lowercase().contains(&query)
                            || chat.phone_number.contains(&query))
                        .then_some(crate::ContactSearchResult {
                            contact_jid: format!("{}@s.whatsapp.net", direct_digits(id)),
                            chat_id: chat.id,
                            display_name: chat.title,
                            secondary_name: chat.push_name,
                            phone_number: chat.phone_number,
                        })
                    })
                    .take(8)
                    .collect();
                let groups = (0..40)
                    .filter(|id: &usize| id.is_multiple_of(5))
                    .map(fake_chat)
                    .filter(|chat| chat.title.to_lowercase().contains(&query))
                    .take(6)
                    .collect();
                let messages = if query.chars().count() >= 3 {
                    (message_total.saturating_sub(25)..message_total)
                        .map(|id| {
                            let chat = fake_chat(id % 20);
                            MessageSearchResult {
                                chat_id: chat.id.clone(),
                                message_id: format!("message-{id}"),
                                chat_title: chat.title.clone(),
                                sender_name: if id.is_multiple_of(2) {
                                    "Vos"
                                } else {
                                    PEOPLE[1].name
                                }
                                .into(),
                                timestamp_ms: id as i64,
                                snippet: format!("“{}” — coincide con “{query}”", message_text(id)),
                                kind: "text".into(),
                                archived: chat.archived,
                                chat: Some(chat),
                            }
                        })
                        .take(20)
                        .collect()
                } else {
                    Vec::new()
                };
                rpc_response::Result::SearchLocal(SearchLocalResponse {
                    contacts,
                    groups,
                    messages,
                })
            }
            Some(rpc_request::Request::OpenContact(request)) => {
                // The contact of the JID opens its chat: the roster answers
                // by the digits of the JID, the way the real sidecar does.
                let digits = request.contact_jid.split('@').next().unwrap_or("");
                let known = PEOPLE.iter().position(|person| person.digits == digits);
                let chat = match known {
                    Some(person) => fake_chat(person),
                    None => {
                        let mut chat = fake_chat(10_001);
                        chat.title = "Nuevo contacto".into();
                        chat
                    }
                };
                rpc_response::Result::OpenContact(OpenContactResponse { chat: Some(chat) })
            }
            Some(rpc_request::Request::ListMessagesAround(request)) => {
                let anchor = request
                    .message_id
                    .strip_prefix("message-")
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(500);
                let start = anchor.saturating_sub(25);
                let end = (anchor + 26).min(message_total);
                rpc_response::Result::ListMessagesAround(ListMessagesAroundResponse {
                    messages: (start..end)
                        .map(|id| fake_message(&request.chat_id, id, &profile_dir))
                        .collect(),
                    has_older: start > 0,
                    has_newer: end < message_total,
                    anchor_message_id: request.message_id,
                })
            }
            Some(rpc_request::Request::GetChatAvatar(request)) => {
                // The avatar of a person is their portrait; the one of a
                // group is the bunch of people it is made of.
                let picture = match chat_number(&request.chat_id) {
                    Some(id) if is_group_id(id) => {
                        ensure_fake_group_avatar(&profile_dir, group_index(id))
                    }
                    Some(id) => ensure_fake_avatar(&profile_dir, id % PEOPLE.len()),
                    None => {
                        let seed = chat_seed(&request.chat_id);
                        ensure_fake_avatar(&profile_dir, seed % PEOPLE.len())
                    }
                };
                rpc_response::Result::GetChatAvatar(crate::GetChatAvatarResponse {
                    chat_id: request.chat_id,
                    avatar_path: picture.to_string_lossy().into_owned(),
                })
            }
            Some(rpc_request::Request::GetParticipantAvatar(request)) => {
                // Every participant carries the face of the roster: stable
                // for their JID, the way a real profile photo is.
                let seed = chat_seed(&request.participant_id) % PEOPLE.len();
                let picture = ensure_fake_avatar(&profile_dir, seed);
                rpc_response::Result::GetParticipantAvatar(crate::GetParticipantAvatarResponse {
                    participant_id: request.participant_id,
                    avatar_path: picture.to_string_lossy().into_owned(),
                })
            }
            Some(rpc_request::Request::GetChatInfo(request)) => {
                let id = request
                    .chat_id
                    .strip_prefix("chat-")
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                let chat = fake_chat(id);
                let group = chat.kind == ChatKind::Group as i32;
                let group_number = group_index(id);
                let participants: Vec<ChatParticipant> = if group {
                    (0..if performance_fixture {
                        PERF_PARTICIPANTS
                    } else {
                        12
                    })
                        .map(|index| {
                            if index == 3 {
                                // The "you" of the demonstration is a member
                                // of every group, like a real account.
                                ChatParticipant {
                                    participant_id: format!("{ME_DIGITS}@s.whatsapp.net"),
                                    display_name: ME_NAME.into(),
                                    phone_number: ME_PHONE.into(),
                                    is_admin: false,
                                    is_super_admin: false,
                                    is_me: true,
                                }
                            } else {
                                let member = group_member(group_number, index);
                                ChatParticipant {
                                    participant_id: format!("{}@s.whatsapp.net", member.digits),
                                    display_name: member.name.into(),
                                    phone_number: member.phone.into(),
                                    is_admin: index < 2,
                                    is_super_admin: index == 0,
                                    is_me: false,
                                }
                            }
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                rpc_response::Result::GetChatInfo(GetChatInfoResponse {
                    address: if group {
                        format!("1203630{id:05}@g.us")
                    } else {
                        format!("{}@s.whatsapp.net", direct_digits(id))
                    },
                    about: if group {
                        String::new()
                    } else {
                        ABOUT_TEXTS[id % ABOUT_TEXTS.len()].into()
                    },
                    verified_name: if !group && id % 3 == 0 {
                        format!("{} {id}", VERIFIED_TEXTS[(id / 3) % VERIFIED_TEXTS.len()])
                    } else {
                        String::new()
                    },
                    description: if group {
                        group_description(group_number)
                    } else {
                        String::new()
                    },
                    created_at_ms: if group { 1_712_345_678_000 } else { 0 },
                    created_by: if group {
                        group_member(group_number, 0).name.into()
                    } else {
                        String::new()
                    },
                    participant_count: participants.len() as u32,
                    participants,
                    announce_only: group && id % 10 == 0,
                    locked: group && id % 15 == 0,
                    disappearing_timer_seconds: if group && id % 20 == 0 {
                        7 * 24 * 3600
                    } else {
                        0
                    },
                    is_community: false,
                    join_approval_required: group && id % 25 == 0,
                    chat: Some(chat),
                })
            }
            Some(rpc_request::Request::SendText(request)) => {
                rpc_response::Result::SendText(SendTextResponse {
                    message: Some(Message {
                        id: request.client_message_id,
                        chat_id: request.chat_id,
                        sender_id: "me@s.whatsapp.net".into(),
                        sender_name: "You".into(),
                        from_me: true,
                        timestamp_ms: 1_900_000_000_000,
                        status: MessageStatus::Sent as i32,
                        content: Some(proto_message::Content::Text(TextContent {
                            text: request.text,
                            link_preview: None,
                        })),
                        reply_to_message_id: request.reply_to_message_id,
                        reply_to_chat_id: request.reply_to_chat_id,
                        ..Default::default()
                    }),
                })
            }
            Some(rpc_request::Request::ForwardMessage(request)) => {
                rpc_response::Result::ForwardMessage(crate::ForwardMessageResponse {
                    message: Some(Message {
                        id: request.client_message_id,
                        chat_id: request.target_chat_id,
                        sender_id: "me@s.whatsapp.net".into(),
                        sender_name: "You".into(),
                        from_me: true,
                        timestamp_ms: 1_900_000_000_000,
                        status: MessageStatus::Sent as i32,
                        forwarding_score: 1,
                        content: Some(proto_message::Content::Text(TextContent {
                            text: format!("Forwarded message {}", request.message_id),
                            link_preview: None,
                        })),
                        ..Default::default()
                    }),
                })
            }
            Some(rpc_request::Request::EditMessage(request)) => {
                rpc_response::Result::EditMessage(crate::EditMessageResponse {
                    message: Some(Message {
                        id: request.message_id,
                        chat_id: request.chat_id,
                        sender_id: "me@s.whatsapp.net".into(),
                        sender_name: "You".into(),
                        from_me: true,
                        timestamp_ms: 1_900_000_000_000,
                        status: MessageStatus::Sent as i32,
                        edited: true,
                        content: Some(proto_message::Content::Text(TextContent {
                            text: request.text,
                            link_preview: None,
                        })),
                        ..Default::default()
                    }),
                })
            }
            Some(rpc_request::Request::DeleteMessage(request)) => {
                let revoked_message = Message {
                    id: request.message_id.clone(),
                    chat_id: request.chat_id.clone(),
                    from_me: true,
                    timestamp_ms: 1_900_000_000_000,
                    status: MessageStatus::Sent as i32,
                    revoked: true,
                    ..Default::default()
                };
                emit_fake_event(
                    &incoming,
                    &event_sequence,
                    backend_event::Event::MessageUpserted(MessageUpserted {
                        message: Some(revoked_message.clone()),
                    }),
                );
                rpc_response::Result::DeleteMessage(crate::DeleteMessageResponse {
                    message: Some(revoked_message),
                })
            }
            Some(rpc_request::Request::ListStatuses(_)) => {
                // The states of two people of the roster, with their phones.
                let mut statuses = Vec::new();
                for (index, person) in [&PEOPLE[16], &PEOPLE[22]].into_iter().enumerate() {
                    let messages = (0..2)
                        .map(|message_index| {
                            let mut message = fake_message(
                                "status@broadcast",
                                1_500 - index * 50 - message_index,
                                &profile_dir,
                            );
                            message.sender_id = format!("{}@s.whatsapp.net", person.digits);
                            message.sender_name = person.name.into();
                            message.sender_phone_number = person.phone.into();
                            message
                        })
                        .collect();
                    statuses.push(crate::StatusGroup {
                        sender_id: format!("{}@s.whatsapp.net", person.digits),
                        sender_name: person.name.into(),
                        sender_phone_number: person.phone.into(),
                        messages,
                    });
                }
                rpc_response::Result::ListStatuses(ListStatusesResponse {
                    chat_id: "status@broadcast".into(),
                    groups: statuses,
                })
            }
            Some(rpc_request::Request::SubscribeNewsletter(request)) => {
                // Truncate by chars, never by bytes: byte slicing (`&s[..n]`)
                // panics when n lands inside a multibyte char (emoji, CJK).
                let short_key: String = request.invite_key.chars().take(12).collect();
                let display_key: String = request.invite_key.chars().take(24).collect();
                let jid = format!("{short_key}@newsletter");
                rpc_response::Result::SubscribeNewsletter(SubscribeNewsletterResponse {
                    newsletter: Some(Newsletter {
                        jid: jid.clone(),
                        name: format!("Canal {display_key}"),
                        subscribed_at_ms: 1_900_000_000_000,
                        muted: false,
                    }),
                })
            }
            Some(rpc_request::Request::ListNewsletters(_)) => {
                rpc_response::Result::ListNewsletters(ListNewslettersResponse {
                    newsletters: vec![
                        Newsletter {
                            jid: "news@newsletter".into(),
                            name: "Whatfast Novedades 📢".into(),
                            subscribed_at_ms: 1_900_000_000_000,
                            muted: false,
                        },
                        Newsletter {
                            jid: "dev@newsletter".into(),
                            name: "Fútbol de Primera ⚽".into(),
                            subscribed_at_ms: 1_890_000_000_000,
                            muted: false,
                        },
                    ],
                })
            }
            Some(rpc_request::Request::ListNewsletterMessages(request)) => {
                let channel = match request.jid.as_str() {
                    "news@newsletter" => "Whatfast Novedades 📢",
                    "dev@newsletter" => "Fútbol de Primera ⚽",
                    _ => "Canal",
                };
                let messages = (0..8)
                    .map(|index| {
                        let mut message = fake_message(&request.jid, 1_000 - index, &profile_dir);
                        message.chat_id = request.jid.clone();
                        message.sender_id = request.jid.clone();
                        message.sender_name = channel.into();
                        // A channel posts announcements, not chat: the post
                        // replaces the text of the dataset.
                        message.content = Some(proto_message::Content::Text(TextContent {
                            text: CHANNEL_POSTS[index % CHANNEL_POSTS.len()].into(),
                            link_preview: None,
                        }));
                        message
                    })
                    .collect();
                rpc_response::Result::ListNewsletterMessages(ListNewsletterMessagesResponse {
                    messages,
                })
            }
            Some(rpc_request::Request::UnsubscribeNewsletter(_)) => {
                rpc_response::Result::UnsubscribeNewsletter(crate::UnsubscribeNewsletterResponse {})
            }
            Some(rpc_request::Request::SetChatPin(request)) => {
                let chat = chat_settings
                    .entry(request.chat_id.clone())
                    .or_insert_with(|| fixture_chat(&request.chat_id, None));
                chat.pinned = request.pinned;
                rpc_response::Result::SetChatPin(crate::SetChatPinResponse {
                    chat: Some(fixture_chat(&request.chat_id, Some(chat))),
                })
            }
            Some(rpc_request::Request::SetChatArchive(request)) => {
                let chat = chat_settings
                    .entry(request.chat_id.clone())
                    .or_insert_with(|| fixture_chat(&request.chat_id, None));
                chat.archived = request.archived;
                rpc_response::Result::SetChatArchive(crate::SetChatArchiveResponse {
                    chat: Some(fixture_chat(&request.chat_id, Some(chat))),
                })
            }
            Some(rpc_request::Request::SetChatMute(request)) => {
                let chat = chat_settings
                    .entry(request.chat_id.clone())
                    .or_insert_with(|| fixture_chat(&request.chat_id, None));
                chat.mute_until_ms = request.mute_until_ms;
                chat.muted = request.mute_until_ms > 0;
                rpc_response::Result::SetChatMute(crate::SetChatMuteResponse {
                    chat: Some(fixture_chat(&request.chat_id, Some(chat))),
                })
            }
            Some(rpc_request::Request::SetChatBlock(request)) => {
                let chat = chat_settings
                    .entry(request.chat_id.clone())
                    .or_insert_with(|| fixture_chat(&request.chat_id, None));
                chat.blocked = request.blocked;
                rpc_response::Result::SetChatBlock(crate::SetChatBlockResponse {
                    chat: Some(fixture_chat(&request.chat_id, Some(chat))),
                })
            }
            Some(rpc_request::Request::SendImage(request)) => {
                rpc_response::Result::SendImage(SendImageResponse {
                    message: Some(Message {
                        id: request.client_message_id,
                        chat_id: request.chat_id,
                        sender_id: "me@s.whatsapp.net".into(),
                        sender_name: "You".into(),
                        from_me: true,
                        timestamp_ms: 1_900_000_000_000,
                        status: MessageStatus::Sent as i32,
                        content: Some(proto_message::Content::Image(ImageContent {
                            caption: request.caption,
                            mime_type: String::new(),
                            local_path: request.image_path.clone(),
                            thumbnail_path: request.image_path,
                            downloadable: true,
                            ..Default::default()
                        })),
                        reply_to_message_id: request.reply_to_message_id,
                        reply_to_chat_id: request.reply_to_chat_id,
                        ..Default::default()
                    }),
                })
            }
            Some(rpc_request::Request::SendSticker(request)) => {
                rpc_response::Result::SendSticker(SendStickerResponse {
                    message: Some(Message {
                        id: request.client_message_id,
                        chat_id: request.chat_id,
                        sender_id: "me@s.whatsapp.net".into(),
                        sender_name: "You".into(),
                        from_me: true,
                        timestamp_ms: 1_900_000_000_000,
                        status: MessageStatus::Sent as i32,
                        content: Some(proto_message::Content::Image(ImageContent {
                            mime_type: "image/webp".into(),
                            sticker: true,
                            width: 512,
                            height: 512,
                            ..Default::default()
                        })),
                        reply_to_message_id: request.reply_to_message_id,
                        reply_to_chat_id: request.reply_to_chat_id,
                        ..Default::default()
                    }),
                })
            }
            Some(rpc_request::Request::GetMessageImage(request)) => {
                let numeric_id = request
                    .message_id
                    .strip_prefix("message-")
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                let picture = ensure_fake_image(&profile_dir, numeric_id);
                let path = picture.to_string_lossy().into_owned();
                rpc_response::Result::GetMessageImage(GetMessageImageResponse {
                    chat_id: request.chat_id,
                    message_id: request.message_id,
                    image_path: path.clone(),
                    thumbnail_path: path,
                })
            }
            Some(rpc_request::Request::GetMessageAttachment(request)) => {
                // The file of the message: the one that the message carries.
                // A message without a file has nothing to download.
                let local_path = request
                    .message_id
                    .strip_prefix("message-")
                    .and_then(|value| value.parse::<usize>().ok())
                    .and_then(fake_attachment_of)
                    .map(|attachment| fake_attachment_path(attachment, &profile_dir))
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_default();
                rpc_response::Result::GetMessageAttachment(GetMessageAttachmentResponse {
                    chat_id: request.chat_id,
                    message_id: request.message_id,
                    local_path,
                })
            }
            Some(rpc_request::Request::SendAttachment(request)) => fake_send_attachment(request),
            Some(rpc_request::Request::ListStickers(_)) => {
                rpc_response::Result::ListStickers(ListStickersResponse {
                    packs: vec![
                        StickerPack {
                            id: "favorites".into(),
                            name: "Favourites".into(),
                            stickers: vec![
                                fake_sticker("fav-1", true),
                                fake_sticker("fav-2", true),
                            ],
                        },
                        StickerPack {
                            id: "recent".into(),
                            name: "Recently used".into(),
                            stickers: vec![fake_sticker("recent-1", false)],
                        },
                    ],
                })
            }
            Some(rpc_request::Request::SendStickerFromLibrary(request)) => {
                if request.sticker_id.is_empty() {
                    fake_invalid_argument("client_message_id, chat_id and sticker_id are required")
                } else {
                    rpc_response::Result::SendStickerFromLibrary(SendStickerFromLibraryResponse {
                        message: Some(Message {
                            id: request.client_message_id,
                            chat_id: request.chat_id,
                            sender_id: "me@s.whatsapp.net".into(),
                            sender_name: "You".into(),
                            from_me: true,
                            timestamp_ms: 1_900_000_000_000,
                            status: MessageStatus::Sent as i32,
                            content: Some(proto_message::Content::Image(ImageContent {
                                mime_type: "image/webp".into(),
                                sticker: true,
                                width: 512,
                                height: 512,
                                ..Default::default()
                            })),
                            reply_to_message_id: request.reply_to_message_id,
                            reply_to_chat_id: request.reply_to_chat_id,
                            ..Default::default()
                        }),
                    })
                }
            }
            Some(rpc_request::Request::SendReaction(request)) => {
                let removed = request.emoji.is_empty();
                rpc_response::Result::SendReaction(SendReactionResponse {
                    reaction: Some(Reaction {
                        chat_id: request.chat_id,
                        message_id: request.message_id,
                        sender_id: "me@s.whatsapp.net".into(),
                        emoji: request.emoji,
                        timestamp_ms: 1_900_000_000_000,
                        from_me: true,
                        sender_name: "You".into(),
                        ..Default::default()
                    }),
                    removed,
                })
            }
            Some(rpc_request::Request::RepairRecentReactions(request)) => {
                rpc_response::Result::RepairRecentReactions(crate::RepairRecentReactionsResponse {
                    chat_id: request.chat_id,
                    requested: false,
                    attempts: 0,
                })
            }
            Some(rpc_request::Request::CreatePoll(request)) => {
                let mut message = fake_message(&request.chat_id, 1_001, &profile_dir);
                message.from_me = true;
                message.content = Some(proto_message::Content::Poll(PollContent {
                    question: request.question,
                    options: request
                        .options
                        .into_iter()
                        .map(|name| PollOption {
                            name,
                            vote_count: 0,
                            selected_by_me: false,
                            voters: Vec::new(),
                        })
                        .collect(),
                    selectable_options_count: request.selectable_options_count,
                    total_voters: 0,
                }));
                rpc_response::Result::CreatePoll(crate::CreatePollResponse {
                    message: Some(message),
                })
            }
            Some(rpc_request::Request::VotePoll(request)) => {
                let mut message = fake_message(&request.chat_id, 1_001, &profile_dir);
                message.id = request.poll_message_id;
                message.content = Some(proto_message::Content::Poll(PollContent {
                    question: "Fake poll".into(),
                    options: request
                        .selected_options
                        .into_iter()
                        .map(|name| PollOption {
                            name,
                            vote_count: 1,
                            selected_by_me: true,
                            voters: vec![PollVoter {
                                user_id: "me@s.whatsapp.net".into(),
                                display_name: "You".into(),
                                from_me: true,
                                ..Default::default()
                            }],
                        })
                        .collect(),
                    selectable_options_count: 1,
                    total_voters: 1,
                }));
                rpc_response::Result::VotePoll(crate::VotePollResponse {
                    message: Some(message),
                })
            }
            Some(rpc_request::Request::SetMessagePin(request)) => {
                let stored = pins.entry(request.chat_id.clone()).or_default();
                if request.pinned {
                    if !stored
                        .iter()
                        .any(|pin| pin.message_id == request.message_id)
                    {
                        let numeric_id = request
                            .message_id
                            .strip_prefix("message-")
                            .and_then(|value| value.parse::<usize>().ok())
                            .unwrap_or(0);
                        let mut message = fake_message(&request.chat_id, numeric_id, &profile_dir);
                        message.id = request.message_id.clone();
                        stored.push(PinnedMessage {
                            message_id: request.message_id.clone(),
                            pinned_at_ms: 1_900_000_000_000,
                            pinned_by: "me@s.whatsapp.net".into(),
                            message: Some(message),
                            available: true,
                        });
                    }
                } else {
                    stored.retain(|pin| pin.message_id != request.message_id);
                }
                emit_fake_event(
                    &incoming,
                    &event_sequence,
                    backend_event::Event::PinnedMessagesChanged(PinnedMessagesChanged {
                        chat_id: request.chat_id,
                    }),
                );
                rpc_response::Result::SetMessagePin(crate::SetMessagePinResponse {})
            }
            Some(rpc_request::Request::ListPinnedMessages(request)) => {
                rpc_response::Result::ListPinnedMessages(ListPinnedMessagesResponse {
                    pins: pins.get(&request.chat_id).cloned().unwrap_or_default(),
                })
            }
            Some(rpc_request::Request::MarkRead(_)) => {
                rpc_response::Result::MarkRead(crate::MarkReadResponse {})
            }
            Some(rpc_request::Request::SetTyping(_)) => {
                rpc_response::Result::SetTyping(crate::SetTypingResponse {})
            }
            Some(rpc_request::Request::Logout(_)) => {
                rpc_response::Result::Logout(crate::LogoutResponse {})
            }
            Some(rpc_request::Request::Reconnect(_)) => {
                rpc_response::Result::Reconnect(crate::ReconnectResponse { started: true })
            }
            Some(rpc_request::Request::Shutdown(_)) => {
                rpc_response::Result::Shutdown(crate::ShutdownResponse {})
            }
            // The fake has no tray and no agents database: a draft is
            // acknowledged, the list is empty and a decision says so.
            Some(rpc_request::Request::DraftPropose(_)) => {
                rpc_response::Result::DraftPropose(crate::DraftProposeResponse {
                    draft_id: 1,
                    state: "proposed".into(),
                })
            }
            Some(rpc_request::Request::DraftList(_)) => {
                rpc_response::Result::DraftList(crate::DraftListResponse { drafts: Vec::new() })
            }
            Some(rpc_request::Request::DraftDecide(_)) => {
                rpc_response::Result::DraftDecide(crate::DraftDecideResponse {
                    draft: None,
                    error: "the fake backend has no drafts".into(),
                })
            }
            Some(rpc_request::Request::DraftFinish(_)) => {
                rpc_response::Result::DraftFinish(crate::DraftFinishResponse { draft: None })
            }
            Some(rpc_request::Request::GuideInstall(_)) => {
                rpc_response::Result::GuideInstall(crate::GuideInstallResponse {
                    installed: false,
                    target: String::new(),
                    backup: String::new(),
                })
            }
            Some(rpc_request::Request::GuideRemove(_)) => {
                rpc_response::Result::GuideRemove(crate::GuideRemoveResponse { removed: false })
            }
            Some(rpc_request::Request::GuideStatus(_)) => {
                rpc_response::Result::GuideStatus(crate::GuideStatusResponse {
                    installed: false,
                    target: String::new(),
                })
            }
            // The fake runs no harness: the fast mode answers with the echo of
            // the question, which is what the screen shows without an agent.
            Some(rpc_request::Request::AgentAsk(request)) => {
                rpc_response::Result::AgentAsk(crate::AgentAskResponse {
                    agent_id: request.agent_id,
                    text: format!("(fake) {}", request.question),
                    ms: 1,
                })
            }
            // The fake keeps no table of automations: the list is empty and a
            // change is acknowledged without a row.
            Some(rpc_request::Request::ScheduleList(_)) => {
                rpc_response::Result::ScheduleList(crate::ScheduleListResponse {
                    schedules: Vec::new(),
                })
            }
            Some(rpc_request::Request::SchedulePut(request)) => {
                rpc_response::Result::SchedulePut(crate::SchedulePutResponse {
                    schedule: Some(crate::Schedule {
                        id: 1,
                        agent_id: request.agent_id,
                        chat_id: request.chat_id,
                        chat_title: String::new(),
                        kind: request.kind,
                        at_ms: request.at_ms,
                        cron: request.cron,
                        prompt: request.prompt,
                        output: request.output,
                        window: request.window,
                        next_at_ms: request.at_ms,
                        paused: false,
                    }),
                    error: String::new(),
                })
            }
            Some(rpc_request::Request::SchedulePause(request)) => {
                rpc_response::Result::SchedulePause(crate::SchedulePauseResponse {
                    schedule: None,
                    error: if request.paused {
                        String::new()
                    } else {
                        "the fake backend has no automations".into()
                    },
                })
            }
            Some(rpc_request::Request::ScheduleRemove(_)) => {
                rpc_response::Result::ScheduleRemove(crate::ScheduleRemoveResponse {
                    removed: true,
                })
            }
            None => continue,
            // v22 calls: the demonstration answers the control plane so the
            // interface can exercise the flow without a linked device.
            Some(rpc_request::Request::CallPlace(_)) => {
                rpc_response::Result::CallPlace(CallPlaceResponse {
                    call_id: "fake-call".into(),
                })
            }
            Some(rpc_request::Request::CallAccept(_)) => {
                rpc_response::Result::CallAccept(CallAcceptResponse {})
            }
            Some(rpc_request::Request::CallReject(_)) => {
                rpc_response::Result::CallReject(CallRejectResponse {})
            }
            Some(rpc_request::Request::CallHangup(_)) => {
                rpc_response::Result::CallHangup(CallHangupResponse {})
            }
            Some(rpc_request::Request::CallMute(_)) => {
                rpc_response::Result::CallMute(CallMuteResponse {})
            }
            Some(rpc_request::Request::CallAudio(_)) => {
                rpc_response::Result::CallAudio(CallAudioResponse {})
            }
        };
        if incoming
            .send_blocking(BridgeMessage::Envelope(Box::new(Envelope {
                protocol_version: PROTOCOL_VERSION,
                request_id,
                body: Some(envelope::Body::Response(RpcResponse {
                    result: Some(result),
                })),
            })))
            .is_err()
        {
            return;
        }
        if is_hello {
            // The real backend only exposes events after the Hello response has
            // been written, so the deterministic backend must keep the same
            // subscription boundary.
            handshaken.store(true, Ordering::Release);
        }
    }
}

/// The error the fake answers with when a request is malformed.
fn fake_invalid_argument(message: impl Into<String>) -> rpc_response::Result {
    rpc_response::Result::Error(RpcError {
        code: "invalid_argument".into(),
        message: message.into(),
        retryable: false,
    })
}

/// A sticker of the library of the fake.
pub fn fake_sticker(id: &str, favorite: bool) -> Sticker {
    Sticker {
        id: id.into(),
        local_path: String::new(),
        mime_type: "image/webp".into(),
        animated: false,
        width: 512,
        height: 512,
        favorite,
        last_used_ms: 1_900_000_000_000,
        source_chat_id: String::new(),
        source_message_id: String::new(),
    }
}

/// The answer of a request that sends an attachment.
///
/// The origin validated the attachment here and answered with an error of
/// the protocol when the request did not make sense.
pub fn fake_send_attachment(request: crate::SendAttachmentRequest) -> rpc_response::Result {
    let Ok(kind) = AttachmentKind::try_from(request.kind) else {
        return fake_invalid_argument("unknown attachment kind");
    };
    if kind == AttachmentKind::Unspecified {
        return fake_invalid_argument("attachment kind is required");
    }
    if request.voice_note && kind != AttachmentKind::Audio {
        return fake_invalid_argument("voice_note is only valid for audio attachments");
    }
    if kind == AttachmentKind::Audio && !request.caption.is_empty() {
        return fake_invalid_argument("audio messages do not support captions");
    }
    let (kind_name, default_mime) = match kind {
        AttachmentKind::Document => ("document", "application/octet-stream"),
        AttachmentKind::Video => ("video", "video/mp4"),
        AttachmentKind::Audio => ("audio", "audio/ogg"),
        // The kind was validated above: the arm is here to keep the match
        // exhaustive without a panic.
        AttachmentKind::Unspecified => {
            return fake_invalid_argument("attachment kind is required");
        }
    };
    // The type of the file comes from its extension: the fake does not open
    // the file, but the interface shows the type next to the name.
    let mime_type = if request.voice_note {
        // A voice note is always Ogg Opus: that is what the protocol asks
        // for.
        "audio/ogg; codecs=opus".to_string()
    } else {
        fake_mime_of(&request.file_path, default_mime).to_string()
    };
    let file_name = Path::new(&request.file_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("attachment")
        .to_owned();
    rpc_response::Result::SendAttachment(SendAttachmentResponse {
        message: Some(Message {
            id: request.client_message_id,
            chat_id: request.chat_id,
            sender_id: "me@s.whatsapp.net".into(),
            sender_name: "You".into(),
            from_me: true,
            timestamp_ms: 1_900_000_000_000,
            status: MessageStatus::Sent as i32,
            content: Some(proto_message::Content::Attachment(AttachmentContent {
                kind: kind_name.into(),
                caption: request.caption,
                mime_type,
                file_name,
                // Match the real backend's ownership boundary: a selected
                // source path is never exposed as a renderable cache asset.
                local_path: String::new(),
                file_size: fake_file_size(&request.file_path),
                voice_note: request.voice_note,
                downloadable: true,
                ..Default::default()
            })),
            reply_to_message_id: request.reply_to_message_id,
            reply_to_chat_id: request.reply_to_chat_id,
            ..Default::default()
        }),
    })
}

/// The type of a file, guessed from its extension.
///
/// The fake does not open the file: the extension is enough to fill the type
/// that the interface shows next to the name.
fn fake_mime_of<'a>(file_path: &str, fallback: &'a str) -> &'a str {
    let extension = Path::new(file_path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "zip" => "application/zip",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "mp3" => "audio/mpeg",
        "ogg" | "opus" => "audio/ogg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => fallback,
    }
}

/// The size of a file on disk, zero when it is not there.
fn fake_file_size(file_path: &str) -> u64 {
    std::fs::metadata(file_path)
        .map(|meta| meta.len())
        .unwrap_or_default()
}

/// Materialize a real PCM WAV once per profile so fake-mode audio and
/// voice notes have a playable file to point the inline player at. The file
/// lives under the managed media tree, which is the only path the asset
/// protocol serves.
pub fn ensure_fake_audio(profile_dir: &Path) -> PathBuf {
    ensure_fake_audio_named(profile_dir, AUDIO_FILE)
}

/// Materialize a real PCM WAV with the name that the dataset uses.
///
/// The song is the audio of the dataset and the hum is the voice note: two
/// melodies, playable by the reader of the system and by the inline player
/// of the interface, written without another dependency.
pub fn ensure_fake_audio_named(profile_dir: &Path, name: &str) -> PathBuf {
    let media_dir = profile_dir.join("media");
    let path = media_dir.join(name);
    if path.exists() {
        return path;
    }
    if ensure_media_dir(&media_dir).is_err() {
        return path;
    }
    let wav = if name == VOICE_NOTE_FILE {
        wav::encode_hum()
    } else {
        wav::encode_song()
    };
    if std::fs::write(&path, wav).is_ok() {
        let _ = restrict_permissions(&path);
    }
    path
}

/// Materialize a real document once per profile, so the document of the
/// dataset opens with the reader of the system instead of failing.
///
/// The file is a PDF of one page: it starts with the header of the version
/// 1.4 and ends with the marker of the end of the file.
pub fn ensure_fake_document(profile_dir: &Path) -> PathBuf {
    let media_dir = profile_dir.join("media");
    let path = media_dir.join(DOCUMENT_FILE);
    if path.exists() {
        return path;
    }
    if ensure_media_dir(&media_dir).is_err() {
        return path;
    }
    if std::fs::write(&path, pdf::encode_one_page("Informe de Whatfast")).is_ok() {
        let _ = restrict_permissions(&path);
    }
    path
}

/// Materialize a real video once per profile, so the video of the dataset
/// opens with the player of the system instead of failing.
///
/// A full movie is not something the fake can build: the file carries the
/// header of the format and a chunk of data, which is what the system needs
/// to recognize it as a video.
pub fn ensure_fake_video(profile_dir: &Path) -> PathBuf {
    let media_dir = profile_dir.join("media");
    let path = media_dir.join(VIDEO_FILE);
    if path.exists() {
        return path;
    }
    if ensure_media_dir(&media_dir).is_err() {
        return path;
    }
    if std::fs::write(&path, mp4::encode_stub(4_096)).is_ok() {
        let _ = restrict_permissions(&path);
    }
    path
}

/// The file of the attachment, written under the media tree of the profile.
fn fake_attachment_path(attachment: FakeAttachment, profile_dir: &Path) -> PathBuf {
    match attachment {
        FakeAttachment::Document => ensure_fake_document(profile_dir),
        FakeAttachment::Video => ensure_fake_video(profile_dir),
        FakeAttachment::Audio => ensure_fake_audio_named(profile_dir, AUDIO_FILE),
        FakeAttachment::VoiceNote => ensure_fake_audio_named(profile_dir, VOICE_NOTE_FILE),
    }
}

/// The content of the attachment of a message of the dataset.
///
/// The file is real: it lives under the media tree of the profile, so the
/// interface can open it, play it or download it.
fn fake_attachment_content(
    attachment: FakeAttachment,
    profile_dir: &Path,
) -> proto_message::Content {
    let path = fake_attachment_path(attachment, profile_dir);
    let (kind, caption, mime_type, file_name, duration_seconds, width, height, voice_note) =
        match attachment {
            FakeAttachment::Document => (
                "document",
                "El informe",
                "application/pdf",
                DOCUMENT_FILE,
                0,
                0,
                0,
                false,
            ),
            FakeAttachment::Video => (
                "video",
                "El viaje",
                "video/mp4",
                VIDEO_FILE,
                12,
                1280,
                720,
                false,
            ),
            FakeAttachment::Audio => (
                "audio",
                "",
                "audio/wav",
                AUDIO_FILE,
                wav::song_seconds(),
                0,
                0,
                false,
            ),
            FakeAttachment::VoiceNote => (
                "audio",
                "",
                "audio/wav",
                VOICE_NOTE_FILE,
                wav::hum_seconds(),
                0,
                0,
                true,
            ),
        };
    proto_message::Content::Attachment(AttachmentContent {
        kind: kind.into(),
        caption: caption.into(),
        mime_type: mime_type.into(),
        file_name: file_name.into(),
        local_path: path.to_string_lossy().into_owned(),
        file_size: std::fs::metadata(&path)
            .map(|meta| meta.len())
            .unwrap_or_default(),
        width,
        height,
        duration_seconds,
        voice_note,
        downloadable: true,
        ..Default::default()
    })
}

/// The kind of file that the dataset puts in a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FakeAttachment {
    /// A document: a PDF that the reader of the system opens.
    Document,
    /// A video: an MP4 that the player of the system opens.
    Video,
    /// An audio file: a song that the player of the system opens.
    Audio,
    /// A voice note: the audio of a message recorded with the phone.
    VoiceNote,
}

/// The file that the message of the dataset carries, if it carries one.
///
/// The index decides it, the same way the photo does: the history has a file
/// of another kind every few messages, so the interface can draw the
/// document, the video, the audio and the voice note.
fn fake_attachment_of(id: usize) -> Option<FakeAttachment> {
    if id == 0 {
        return None;
    }
    if id.is_multiple_of(DOCUMENT_EVERY) {
        Some(FakeAttachment::Document)
    } else if id.is_multiple_of(VIDEO_EVERY) {
        Some(FakeAttachment::Video)
    } else if id.is_multiple_of(AUDIO_EVERY) {
        Some(FakeAttachment::Audio)
    } else if id.is_multiple_of(VOICE_NOTE_EVERY) {
        Some(FakeAttachment::VoiceNote)
    } else {
        None
    }
}

/// Materialize a real picture on disk, so the media path (thumbnail, full
/// viewer, gallery) is exercised by the fake backend instead of showing an
/// endless loading placeholder.
///
/// The origin drew one gradient for every photo. Here every photo is a
/// scene: the caption and the scene come from the same table, so the
/// message "Atardecer en el dique" carries a sunset.
pub fn ensure_fake_image(profile_dir: &Path, seed: usize) -> PathBuf {
    let media_dir = profile_dir.join("media");
    // The v2 in the name keeps the media of the previous dataset (the
    // gradients of the origin) from shadowing the new art in profiles that
    // already ran the old fake: the media is written once per profile.
    let path = media_dir.join(format!("fake-image-v2-{seed}.png"));
    if path.exists() {
        return path;
    }
    if ensure_media_dir(&media_dir).is_err() {
        return path;
    }
    let (width, height) = (320_u32, 240_u32);
    let scene = art::Scene::of_index((seed / 37) % PHOTO_CAPTIONS.len());
    let pixels = art::scene(scene, seed, width, height);
    let png = png::encode_rgb(width, height, &pixels);
    if std::fs::write(&path, png).is_ok() {
        let _ = restrict_permissions(&path);
    }
    path
}

/// The picture of the avatar of a conversation.
///
/// The reference draws a photo in the row of every conversation, so the
/// backend of the demonstration gives one to each chat: the portrait of the
/// person of the roster, painted by the art of this module. The seed is the
/// person, not the chat: the same face for the same person everywhere.
pub fn ensure_fake_avatar(profile_dir: &Path, seed: usize) -> PathBuf {
    let media_dir = profile_dir.join("media");
    let path = media_dir.join(format!("fake-avatar-v2-{seed}.png"));
    if path.exists() {
        return path;
    }
    if ensure_media_dir(&media_dir).is_err() {
        return path;
    }
    let (width, height) = (128_u32, 128_u32);
    let pixels = art::portrait(seed, width, height);
    let png = png::encode_rgb(width, height, &pixels);
    if std::fs::write(&path, png).is_ok() {
        let _ = restrict_permissions(&path);
    }
    path
}

/// The picture of the avatar of a group: the bunch of people it is made
/// of, the way a group photo is.
pub fn ensure_fake_group_avatar(profile_dir: &Path, seed: usize) -> PathBuf {
    let media_dir = profile_dir.join("media");
    let path = media_dir.join(format!("fake-avatar-group-{seed}.png"));
    if path.exists() {
        return path;
    }
    if ensure_media_dir(&media_dir).is_err() {
        return path;
    }
    let (width, height) = (128_u32, 128_u32);
    let pixels = art::group(seed, width, height);
    let png = png::encode_rgb(width, height, &pixels);
    if std::fs::write(&path, png).is_ok() {
        let _ = restrict_permissions(&path);
    }
    path
}

/// The number that decides the picture of a conversation.
fn chat_seed(chat_id: &str) -> usize {
    chat_id
        .strip_prefix("chat-")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_else(|| {
            chat_id.bytes().fold(0usize, |total, byte| {
                total.wrapping_mul(31).wrapping_add(byte as usize)
            })
        })
}

/// The drawings of the demonstration: the portraits of the avatars and the
/// scenes of the photos, painted pixel by pixel with the encoders of this
/// module (the crate may not depend on `image`).
///
/// Everything is flat color and gradients with a pinch of grain: the style
/// of an avatar illustration, deterministic in the seed like the rest of
/// the dataset.
mod art {
    /// An RGB color.
    type Rgb = [u8; 3];

    /// The scenes a photo of the dataset can be.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum Scene {
        /// A sunset over the water, with the sun going down behind hills.
        Sunset,
        /// A cup of coffee on a wooden table, with steam over it.
        Coffee,
        /// A cat sleeping on a couch.
        Cat,
        /// Green mountains under a blue sky.
        Mountains,
        /// The buildings of downtown at night, with lit windows.
        City,
        /// A pizza on its board.
        Pizza,
        /// Flowers of the patio.
        Flowers,
        /// A beach at the afternoon, with a palm.
        Beach,
    }

    impl Scene {
        /// The scene of the photo of the dataset: the table of captions and
        /// the enum share the order.
        pub(super) fn of_index(index: usize) -> Self {
            match index % 8 {
                0 => Self::Sunset,
                1 => Self::Coffee,
                2 => Self::Cat,
                3 => Self::Mountains,
                4 => Self::City,
                5 => Self::Pizza,
                6 => Self::Flowers,
                _ => Self::Beach,
            }
        }
    }

    /// A canvas of RGB pixels.
    pub(super) struct Canvas {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    }

    impl Canvas {
        /// A canvas of the size of the picture, black before painting.
        pub(super) fn new(width: u32, height: u32) -> Self {
            Self {
                width,
                height,
                pixels: vec![0; (width as usize) * (height as usize) * 3],
            }
        }

        /// Paint one pixel, ignoring the ones outside the canvas.
        fn set(&mut self, x: i32, y: i32, color: Rgb) {
            if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
                return;
            }
            let offset = ((y as u32 * self.width + x as u32) * 3) as usize;
            self.pixels[offset..offset + 3].copy_from_slice(&color);
        }

        /// Paint one pixel over what is already there, by an alpha.
        fn blend(&mut self, x: i32, y: i32, color: Rgb, alpha: f32) {
            if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
                return;
            }
            let offset = ((y as u32 * self.width + x as u32) * 3) as usize;
            for (channel, tone) in color.iter().enumerate() {
                let old = f32::from(self.pixels[offset + channel]);
                let new = f32::from(*tone);
                let mixed = old * (1.0 - alpha) + new * alpha;
                self.pixels[offset + channel] = mixed.round().clamp(0.0, 255.0) as u8;
            }
        }

        /// Paint an axis-aligned rectangle.
        pub(super) fn fill_rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: Rgb) {
            let y_from = y0.floor().max(0.0) as i32;
            let y_to = y1.ceil().min(self.height as f32) as i32;
            let x_from = x0.floor().max(0.0) as i32;
            let x_to = x1.ceil().min(self.width as f32) as i32;
            for y in y_from..y_to {
                for x in x_from..x_to {
                    self.set(x, y, color);
                }
            }
        }

        /// Paint a filled circle.
        pub(super) fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: Rgb) {
            self.fill_ellipse(cx, cy, radius, radius, color);
        }

        /// Paint a circle over what is already there, by an alpha.
        pub(super) fn blend_circle(
            &mut self,
            cx: f32,
            cy: f32,
            radius: f32,
            color: Rgb,
            alpha: f32,
        ) {
            for y in ((cy - radius).floor() as i32)..=(cy + radius).ceil() as i32 {
                for x in ((cx - radius).floor() as i32)..=(cx + radius).ceil() as i32 {
                    let dx = x as f32 - cx;
                    let dy = y as f32 - cy;
                    if dx * dx + dy * dy <= radius * radius {
                        self.blend(x, y, color, alpha);
                    }
                }
            }
        }

        /// Paint a filled ellipse.
        pub(super) fn fill_ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, color: Rgb) {
            for y in ((cy - ry).floor() as i32)..=(cy + ry).ceil() as i32 {
                for x in ((cx - rx).floor() as i32)..=(cx + rx).ceil() as i32 {
                    let dx = (x as f32 - cx) / rx;
                    let dy = (y as f32 - cy) / ry;
                    if dx * dx + dy * dy <= 1.0 {
                        self.set(x, y, color);
                    }
                }
            }
        }

        /// Paint the whole canvas with a vertical gradient.
        pub(super) fn vertical_gradient(&mut self, top: Rgb, bottom: Rgb) {
            for y in 0..self.height {
                let t = y as f32 / self.height as f32;
                let color = mix(top, bottom, t);
                for x in 0..self.width {
                    self.set(x as i32, y as i32, color);
                }
            }
        }

        /// Paint the whole canvas with a horizontal gradient.
        #[allow(dead_code)]
        pub(super) fn horizontal_gradient(&mut self, left: Rgb, right: Rgb) {
            for x in 0..self.width {
                let t = x as f32 / self.width as f32;
                let color = mix(left, right, t);
                for y in 0..self.height {
                    self.set(x as i32, y as i32, color);
                }
            }
        }

        /// The pixels of the canvas, row by row, RGB.
        pub(super) fn into_pixels(self) -> Vec<u8> {
            self.pixels
        }

        /// Add film grain: a pinch of noise so the flat colors read like a
        /// photo instead of a vector drawing.
        pub(super) fn grain(&mut self, seed: usize, amount: u8) {
            for y in 0..self.height {
                for x in 0..self.width {
                    let hash = x.wrapping_mul(0x9e37_79b1)
                        ^ y.wrapping_mul(0x85eb_ca77)
                        ^ ((seed as u32).wrapping_mul(0xc2b2_ae3d).rotate_left(13));
                    let span = u32::from(amount) * 2 + 1;
                    let delta = (hash % span) as i32 - i32::from(amount);
                    let offset = ((y * self.width + x) * 3) as usize;
                    for channel in 0..3 {
                        let value = i32::from(self.pixels[offset + channel]) + delta;
                        self.pixels[offset + channel] = value.clamp(0, 255) as u8;
                    }
                }
            }
        }
    }

    /// A range of hills: its base height and the bells of its peaks.
    type Ridge = (f32, [(f32, f32, f32); 3]);

    /// The linear mix of two colors.
    fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        [
            (f32::from(a[0]) * (1.0 - t) + f32::from(b[0]) * t)
                .round()
                .clamp(0.0, 255.0) as u8,
            (f32::from(a[1]) * (1.0 - t) + f32::from(b[1]) * t)
                .round()
                .clamp(0.0, 255.0) as u8,
            (f32::from(a[2]) * (1.0 - t) + f32::from(b[2]) * t)
                .round()
                .clamp(0.0, 255.0) as u8,
        ]
    }

    /// The skin tones of the roster, from the lightest to the darkest.
    const SKINS: [Rgb; 5] = [
        [245, 218, 190],
        [231, 194, 159],
        [207, 163, 122],
        [175, 126, 88],
        [148, 101, 69],
    ];

    /// The hair colors, natural ones and one dyed red for the fun of it.
    const HAIRS: [Rgb; 6] = [
        [43, 32, 27],
        [22, 20, 20],
        [93, 62, 37],
        [138, 100, 58],
        [72, 72, 78],
        [164, 64, 52],
    ];

    /// The colors of the shirts.
    const SHIRTS: [Rgb; 7] = [
        [66, 122, 190],
        [48, 148, 114],
        [201, 122, 54],
        [122, 82, 152],
        [188, 74, 84],
        [72, 130, 178],
        [94, 96, 106],
    ];

    /// The soft colors of the background of a portrait.
    const BACKDROPS: [Rgb; 8] = [
        [214, 232, 222],
        [232, 224, 208],
        [216, 226, 238],
        [238, 218, 222],
        [224, 216, 234],
        [218, 234, 224],
        [236, 226, 210],
        [210, 224, 230],
    ];

    /// Paint one bust (head, neck and shoulders) at a place, with the face
    /// of the seed and the size of `radius`. The details scale with the
    /// radius, so the same drawing serves the avatar of a person and the
    /// three small heads of a group.
    fn bust(canvas: &mut Canvas, cx: f32, cy: f32, radius: f32, seed: usize) {
        let skin = SKINS[seed % SKINS.len()];
        let skin_shadow = mix(skin, [0, 0, 0], 0.18);
        let hair = HAIRS[(seed / 5) % HAIRS.len()];
        let shirt = SHIRTS[(seed * 3) % SHIRTS.len()];
        // The haircut: a cap for everyone, and one of three extras.
        let style = (seed / 7) % 4;
        // The hair that falls behind the head, for the long haircut.
        if style == 1 {
            canvas.fill_circle(cx, cy, radius * 1.16, hair);
            canvas.fill_rect(
                cx - radius * 1.12,
                cy,
                cx - radius * 0.82,
                cy + radius * 3.1,
                hair,
            );
            canvas.fill_rect(
                cx + radius * 0.82,
                cy,
                cx + radius * 1.12,
                cy + radius * 3.1,
                hair,
            );
        }
        // Shoulders: a wide ellipse that enters from below.
        canvas.fill_ellipse(cx, cy + radius * 2.7, radius * 1.75, radius * 1.9, shirt);
        // Neck.
        canvas.fill_rect(
            cx - radius * 0.3,
            cy + radius * 0.72,
            cx + radius * 0.3,
            cy + radius * 1.9,
            skin_shadow,
        );
        // Head.
        canvas.fill_circle(cx, cy, radius, skin);
        // The bun of the third haircut.
        if style == 2 {
            canvas.fill_circle(cx, cy - radius * 1.18, radius * 0.42, hair);
        }
        // The cap of hair: a disc a bit higher, then the face again on top,
        // which leaves the fringe. The bald haircut keeps almost none.
        let cap_scale = if style == 3 { 1.04 } else { 1.14 };
        canvas.fill_circle(cx, cy - radius * 0.22, radius * cap_scale, hair);
        canvas.fill_circle(cx, cy + radius * 0.04, radius * 1.0, skin);
        // Ears.
        canvas.fill_circle(cx - radius, cy + radius * 0.08, radius * 0.16, skin);
        canvas.fill_circle(cx + radius, cy + radius * 0.08, radius * 0.16, skin);
        // Eyes.
        let eye = [40, 34, 30];
        canvas.fill_circle(cx - radius * 0.34, cy + radius * 0.02, radius * 0.09, eye);
        canvas.fill_circle(cx + radius * 0.34, cy + radius * 0.02, radius * 0.09, eye);
        // Eyebrows: thin arcs a bit above the eyes.
        let brow = mix(hair, [0, 0, 0], 0.15);
        canvas.fill_rect(
            cx - radius * 0.52,
            cy - radius * 0.30,
            cx - radius * 0.18,
            cy - radius * 0.23,
            brow,
        );
        canvas.fill_rect(
            cx + radius * 0.18,
            cy - radius * 0.30,
            cx + radius * 0.52,
            cy - radius * 0.23,
            brow,
        );
        // The smile: a short parabola under the eyes, corners up.
        let step = radius * 0.03;
        let mut dx = -radius * 0.30;
        while dx <= radius * 0.30 {
            let dy = radius * 0.46 - dx * dx / (radius * 0.60);
            canvas.set((cx + dx) as i32, (cy + dy) as i32, eye);
            canvas.set((cx + dx) as i32, (cy + dy + 1.0) as i32, eye);
            dx += step.max(1.0);
        }
        // Blush.
        canvas.blend_circle(
            cx - radius * 0.52,
            cy + radius * 0.22,
            radius * 0.16,
            [232, 124, 116],
            0.30,
        );
        canvas.blend_circle(
            cx + radius * 0.52,
            cy + radius * 0.22,
            radius * 0.16,
            [232, 124, 116],
            0.30,
        );
    }

    /// The portrait of the avatar of a person.
    pub(super) fn portrait(seed: usize, width: u32, height: u32) -> Vec<u8> {
        let mut canvas = Canvas::new(width, height);
        let backdrop = BACKDROPS[seed % BACKDROPS.len()];
        canvas.vertical_gradient(mix(backdrop, [255, 255, 255], 0.25), backdrop);
        let w = width as f32;
        let h = height as f32;
        bust(&mut canvas, w * 0.5, h * 0.46, w * 0.24, seed);
        canvas.grain(seed, 3);
        canvas.into_pixels()
    }

    /// The avatar of a group: three people together, the way a group photo
    /// is.
    pub(super) fn group(seed: usize, width: u32, height: u32) -> Vec<u8> {
        let mut canvas = Canvas::new(width, height);
        let backdrop = BACKDROPS[(seed + 3) % BACKDROPS.len()];
        canvas.vertical_gradient(mix(backdrop, [255, 255, 255], 0.25), backdrop);
        let w = width as f32;
        let h = height as f32;
        bust(&mut canvas, w * 0.30, h * 0.58, w * 0.17, seed * 3 + 1);
        bust(&mut canvas, w * 0.70, h * 0.58, w * 0.17, seed * 5 + 2);
        bust(&mut canvas, w * 0.50, h * 0.44, w * 0.20, seed * 7 + 3);
        canvas.grain(seed, 3);
        canvas.into_pixels()
    }

    /// The pixels of a photo of the dataset: the scene, painted.
    pub(super) fn scene(kind: Scene, seed: usize, width: u32, height: u32) -> Vec<u8> {
        let mut canvas = Canvas::new(width, height);
        match kind {
            Scene::Sunset => sunset(&mut canvas, seed),
            Scene::Coffee => coffee(&mut canvas, seed),
            Scene::Cat => cat(&mut canvas, seed),
            Scene::Mountains => mountains(&mut canvas, seed),
            Scene::City => city(&mut canvas, seed),
            Scene::Pizza => pizza(&mut canvas, seed),
            Scene::Flowers => flowers(&mut canvas, seed),
            Scene::Beach => beach(&mut canvas, seed),
        }
        canvas.grain(seed + 1, 5);
        canvas.into_pixels()
    }

    /// The silhouette of a mountain range: for every column, the height is
    /// the sum of a few bell curves, so the ridge rolls like a real one.
    ///
    /// The base and the rises are fractions of the canvas height; the
    /// centers and the widths of the bells are pixels, and the return is a
    /// row of the canvas.
    fn ridge(canvas: &Canvas, x: f32, base: f32, bells: &[(f32, f32, f32)]) -> f32 {
        let mut height = base;
        for &(center, width, rise) in bells {
            let dx = (x - center) / width;
            height -= rise * (-dx * dx).exp();
        }
        height.clamp(0.0, 1.0) * canvas.height as f32
    }

    /// Fill a column from a height to the bottom of the canvas.
    fn column_from(canvas: &mut Canvas, x: i32, from_y: f32, color: Rgb) {
        for y in from_y.round().max(0.0) as i32..canvas.height as i32 {
            canvas.set(x, y, color);
        }
    }

    /// The sunset: the sky goes from violet to orange, the sun touches the
    /// water and the hills keep the last light.
    fn sunset(canvas: &mut Canvas, seed: usize) {
        let w = canvas.width as f32;
        let h = canvas.height as f32;
        let water = h * 0.76;
        for y in 0..canvas.height {
            let t = y as f32 / water;
            let color = mix(
                mix([46, 26, 74], [214, 94, 58], t * t),
                [247, 171, 89],
                t * t * t,
            );
            for x in 0..canvas.width {
                canvas.set(x as i32, y as i32, color);
            }
        }
        let sun_x = w * (0.62 + (seed % 3) as f32 * 0.06);
        let sun_y = water * 0.78;
        for radius in [64.0, 46.0, 34.0] {
            canvas.blend_circle(sun_x, sun_y, radius, [255, 196, 120], 0.22);
        }
        canvas.fill_circle(sun_x, sun_y, 26.0, [255, 224, 156]);
        // The water, with the reflection of the sun as a wavy column.
        for y in water as i32..canvas.height as i32 {
            let depth = (y as f32 - water) / (h - water);
            for x in 0..canvas.width {
                let color = mix([58, 36, 80], [30, 18, 44], depth);
                canvas.set(x as i32, y, color);
            }
        }
        for y in water as i32..canvas.height as i32 {
            let ripple = ((y as f32 * 0.7 + (seed % 5) as f32) * 0.35).sin() * 4.0;
            let half = 26.0 - (y - water as i32) as f32 * 0.18;
            if half > 0.0 {
                for x in ((sun_x - half + ripple) as i32)..=((sun_x + half + ripple) as i32) {
                    let alpha = 0.55 - (y - water as i32) as f32 * 0.02;
                    if alpha > 0.0 {
                        canvas.blend(x, y, [255, 206, 130], alpha);
                    }
                }
            }
        }
        // Two ranges of hills, one behind the other.
        let far = [
            (w * 0.25, w * 0.30, 0.13),
            (w * 0.78, w * 0.38, 0.11),
            (w * 1.25, w * 0.30, 0.10),
        ];
        let near = [
            (w * 0.05, w * 0.26, 0.10),
            (w * 0.60, w * 0.32, 0.14),
            (w * 1.10, w * 0.28, 0.09),
        ];
        for x in 0..canvas.width as i32 {
            column_from(canvas, x, ridge(canvas, x as f32, 0.68, &far), [72, 48, 96]);
        }
        for x in 0..canvas.width as i32 {
            column_from(
                canvas,
                x,
                ridge(canvas, x as f32, 0.80, &near),
                [40, 26, 58],
            );
        }
    }

    /// The coffee: a cup on a wooden table, with steam over it.
    fn coffee(canvas: &mut Canvas, seed: usize) {
        let w = canvas.width as f32;
        let h = canvas.height as f32;
        canvas.vertical_gradient([238, 228, 214], [216, 202, 184]);
        let table = h * 0.66;
        for y in table as i32..canvas.height as i32 {
            let t = (y as f32 - table) / (h - table);
            let color = mix([150, 106, 62], [112, 76, 44], t);
            for x in 0..canvas.width {
                canvas.set(x as i32, y, color);
            }
        }
        // The grain of the wood: darker lines that follow the table.
        for i in 0..6 {
            let y = table + (i as f32 + 0.5) * (h - table) / 6.0;
            for x in 0..canvas.width as i32 {
                let wave = ((x as f32 * 0.11 + i as f32 * 3.0 + (seed % 4) as f32).sin() * 2.0)
                    .round() as i32;
                canvas.set(x, y as i32 + wave, [96, 64, 36]);
            }
        }
        let cx = w * 0.44;
        let cy = h * 0.47 + (seed % 3) as f32 * 4.0;
        // Saucer.
        canvas.fill_ellipse(cx, cy + 26.0, 74.0, 20.0, [232, 226, 216]);
        // Cup and coffee, seen from above and a bit to the side.
        canvas.fill_ellipse(cx, cy, 58.0, 40.0, [246, 242, 236]);
        canvas.fill_ellipse(cx, cy, 47.0, 31.0, [92, 58, 34]);
        canvas.fill_ellipse(cx, cy - 4.0, 40.0, 25.0, [118, 78, 46]);
        // The handle of the cup.
        for angle in (-70..=70).step_by(6) {
            let rad = angle as f32 * std::f32::consts::PI / 180.0;
            let x = cx + 58.0 + rad.cos() * -14.0;
            let y = cy + rad.sin() * 20.0;
            canvas.fill_circle(x, y, 6.5, [246, 242, 236]);
        }
        // The steam: two wavy threads over the cup.
        for thread in 0..2 {
            let base_x = cx - 12.0 + thread as f32 * 22.0;
            for step in 0..40 {
                let t = step as f32 / 40.0;
                let x = base_x + ((t * 6.0 + thread as f32 + (seed % 3) as f32).sin()) * 7.0;
                let y = cy - 46.0 - t * 64.0;
                canvas.blend_circle(x, y, 3.4 - t * 1.6, [255, 255, 255], 0.34 * (1.0 - t));
            }
        }
    }

    /// The cat: sleeping on the couch, curled up with the tail around.
    fn cat(canvas: &mut Canvas, seed: usize) {
        let w = canvas.width as f32;
        let h = canvas.height as f32;
        let couch = if seed.is_multiple_of(2) {
            [96, 108, 138]
        } else {
            [126, 96, 108]
        };
        canvas.vertical_gradient(mix(couch, [255, 255, 255], 0.30), couch);
        let seat = h * 0.64;
        canvas.fill_rect(0.0, seat, w, h, mix(couch, [0, 0, 0], 0.22));
        canvas.fill_rect(
            0.0,
            seat - 6.0,
            w,
            seat + 4.0,
            mix(couch, [255, 255, 255], 0.12),
        );
        let fur = if seed.is_multiple_of(3) {
            [222, 184, 136]
        } else {
            [236, 226, 218]
        };
        let fur_dark = mix(fur, [90, 66, 50], 0.45);
        let cx = w * 0.52;
        let cy = seat - 26.0;
        // The body, curled up.
        canvas.fill_ellipse(cx - 26.0, cy + 12.0, 62.0, 34.0, fur);
        // The tail around the body.
        for step in 0..44 {
            let t = step as f32 / 44.0;
            let angle = std::f32::consts::PI * (0.15 + t * 0.95);
            let x = cx - 26.0 + angle.cos() * (58.0 + t * 4.0);
            let y = cy + 14.0 + angle.sin() * (26.0 + t * 3.0);
            canvas.fill_circle(x, y, 9.0 - t * 3.0, fur_dark);
        }
        // The head over the body.
        canvas.fill_circle(cx + 34.0, cy - 10.0, 30.0, fur);
        // The ears.
        for side in [-1.0, 1.0] {
            let ear_x = cx + 34.0 + side * 17.0;
            let ear_y = cy - 34.0;
            canvas.fill_circle(ear_x, ear_y, 8.0, fur);
            canvas.fill_circle(
                ear_x + side * 2.0,
                ear_y + 2.0,
                4.5,
                mix(fur, [200, 120, 120], 0.5),
            );
        }
        // The stripes of the fur.
        for i in 0..4 {
            let x = cx - 64.0 + i as f32 * 26.0;
            canvas.fill_ellipse(x, cy - 12.0, 5.0, 16.0, fur_dark);
        }
        // Closed eyes and nose: the cat sleeps.
        let ink = [64, 48, 40];
        for side in [-1.0, 1.0] {
            for dx in -6..=6 {
                let dy = (dx * dx) as f32 / 10.0;
                canvas.set(
                    (cx + 34.0 + side * 12.0 + dx as f32) as i32,
                    (cy - 10.0 + dy) as i32,
                    ink,
                );
            }
        }
        canvas.fill_circle(cx + 34.0, cy + 1.0, 3.0, [212, 116, 110]);
        // Whiskers.
        for side in [-1.0, 1.0] {
            for i in 0..3 {
                let y = cy - 2.0 + i as f32 * 6.0;
                canvas.set(
                    (cx + 34.0 + side * (36.0 + i as f32 * 3.0)) as i32,
                    y as i32,
                    ink,
                );
                canvas.set(
                    (cx + 34.0 + side * (38.0 + i as f32 * 3.0)) as i32,
                    y as i32 + 2,
                    ink,
                );
            }
        }
        // A cushion under the cat.
        canvas.fill_ellipse(
            cx - 20.0,
            seat + 6.0,
            84.0,
            16.0,
            mix(couch, [255, 255, 255], 0.2),
        );
    }

    /// The mountains: three ranges, one behind the other, under a blue sky.
    fn mountains(canvas: &mut Canvas, seed: usize) {
        let w = canvas.width as f32;
        let h = canvas.height as f32;
        canvas.vertical_gradient([168, 206, 238], [232, 242, 248]);
        canvas.blend_circle(w * 0.82, h * 0.16, 30.0, [255, 244, 200], 0.85);
        let ranges: [Ridge; 3] = [
            (
                0.62,
                [
                    (w * 0.30, w * 0.34, 0.22),
                    (w * 0.95, w * 0.40, 0.18),
                    (w * 1.45, w * 0.34, 0.20),
                ],
            ),
            (
                0.74,
                [
                    (w * 0.10, w * 0.30, 0.18),
                    (w * 0.62, w * 0.36, 0.22),
                    (w * 1.20, w * 0.32, 0.16),
                ],
            ),
            (
                0.88,
                [
                    (w * 0.38, w * 0.42, 0.16),
                    (w * 0.98, w * 0.46, 0.13),
                    (w * -0.10, w * 0.40, 0.12),
                ],
            ),
        ];
        let colors = [
            ([148, 172, 200], [220, 230, 240]),
            ([104, 132, 160], [196, 212, 226]),
            ([74, 104, 82], [130, 158, 118]),
        ];
        let seed_offset = (seed % 4) as f32 * 0.015;
        for (index, (base, bells)) in ranges.iter().enumerate() {
            let base = base + seed_offset * (index as f32 + 1.0);
            for x in 0..canvas.width as i32 {
                let top = ridge(canvas, x as f32, base, bells);
                for y in top.round() as i32..canvas.height as i32 {
                    let t =
                        ((y as f32 - top) / (canvas.height as f32 - top).max(1.0)).clamp(0.0, 1.0);
                    let color = mix(colors[index].0, colors[index].1, t);
                    canvas.set(x, y, color);
                }
            }
        }
    }

    /// The city at night: buildings with lit windows under a crescent moon.
    fn city(canvas: &mut Canvas, seed: usize) {
        let w = canvas.width as f32;
        let h = canvas.height as f32;
        canvas.vertical_gradient([10, 14, 34], [52, 40, 84]);
        // The stars: a deterministic scatter of small dots.
        for i in 0..40 {
            let x = (i as u32).wrapping_mul(97 + (seed as u32 % 13)) % canvas.width;
            let y = (i as u32).wrapping_mul(61 + (seed as u32 % 7)) % (canvas.height / 3);
            if !(i + seed).is_multiple_of(3) {
                canvas.set(x as i32, y as i32, [224, 228, 244]);
            }
        }
        // The moon: a disc with another disc taken out of it.
        let moon_x = w * 0.78;
        let moon_y = h * 0.18;
        canvas.fill_circle(moon_x, moon_y, 20.0, [244, 240, 220]);
        canvas.fill_circle(moon_x + 9.0, moon_y - 5.0, 17.0, [16, 20, 44]);
        // The buildings: bands of heights, with windows on a grid.
        let mut x = 4.0;
        let mut index = 0;
        while x < w {
            let width = 26.0 + ((index as u32 * 31 + seed as u32) % 22) as f32;
            let top = h * (0.30 + (((index as u32 * 53 + seed as u32) % 40) as f32 / 100.0));
            let tone = 26 + ((index as u32 * 17 + seed as u32) % 14) as u8;
            let body = [tone, tone + 6, tone + 22];
            canvas.fill_rect(x, top, x + width, h, body);
            // The lit windows: a third of the grid, deterministic.
            let mut wy = top + 8.0;
            while wy < h - 10.0 {
                let mut wx = x + 6.0;
                while wx < x + width - 6.0 {
                    let lit = ((wx as u32 * 7 + wy as u32 * 13 + seed as u32) % 5) < 2;
                    let color = if lit {
                        [252, 222, 138]
                    } else {
                        [tone - 6, tone, tone + 14]
                    };
                    canvas.fill_rect(wx, wy, wx + 5.0, wy + 6.0, color);
                    wx += 11.0;
                }
                wy += 13.0;
            }
            x += width + 5.0;
            index += 1;
        }
    }

    /// The pizza: on its wooden board, with pepperoni and basil.
    fn pizza(canvas: &mut Canvas, seed: usize) {
        let w = canvas.width as f32;
        let h = canvas.height as f32;
        canvas.vertical_gradient([242, 234, 222], [220, 208, 190]);
        canvas.fill_ellipse(w * 0.5, h * 0.52, w * 0.46, h * 0.42, [172, 122, 74]);
        canvas.fill_ellipse(w * 0.5, h * 0.52, w * 0.44, h * 0.40, [200, 150, 98]);
        canvas.fill_ellipse(w * 0.5, h * 0.52, w * 0.36, h * 0.32, [232, 190, 130]);
        canvas.fill_ellipse(w * 0.5, h * 0.52, w * 0.33, h * 0.29, [244, 216, 150]);
        // Pepperoni.
        let pepperoni = [
            (-0.18, -0.12),
            (0.14, -0.16),
            (-0.02, 0.06),
            (0.20, 0.12),
            (-0.22, 0.14),
            (0.02, -0.02),
            (-0.06, 0.20),
        ];
        for (dx, dy) in pepperoni {
            canvas.fill_circle(w * (0.5 + dx), h * (0.52 + dy), w * 0.038, [186, 58, 46]);
            canvas.fill_circle(w * (0.5 + dx), h * (0.52 + dy), w * 0.030, [206, 78, 60]);
        }
        // Basil.
        for (dx, dy) in [(0.08, -0.05), (-0.12, 0.08), (0.16, -0.18), (-0.02, 0.18)] {
            canvas.fill_ellipse(
                w * (0.5 + dx),
                h * (0.52 + dy),
                w * 0.020,
                w * 0.013,
                [84, 140, 66],
            );
        }
        // A knife next to the board, for the scale of a real photo.
        let knife_y = h * 0.92;
        canvas.fill_rect(
            w * 0.12,
            knife_y - 4.0,
            w * 0.52,
            knife_y + 4.0,
            [208, 210, 214],
        );
        canvas.fill_rect(
            w * 0.52,
            knife_y - 6.0,
            w * 0.70,
            knife_y + 6.0,
            [88, 60, 38],
        );
        if seed.is_multiple_of(2) {
            canvas.fill_circle(w * 0.5, h * 0.52, w * 0.05, [244, 216, 150]);
        }
    }

    /// The flowers of the patio: three of them, each with its petals.
    fn flowers(canvas: &mut Canvas, seed: usize) {
        let w = canvas.width as f32;
        let h = canvas.height as f32;
        canvas.vertical_gradient([206, 224, 190], [168, 198, 158]);
        // The wall behind, with its bricks.
        for y in (0..canvas.height).step_by(18) {
            for x in 0..canvas.width {
                canvas.set(
                    x as i32,
                    y as i32,
                    mix([168, 198, 158], [255, 255, 255], 0.10),
                );
            }
        }
        let blooms = [
            (w * 0.26, h * 0.70, [222, 88, 96]),
            (w * 0.52, h * 0.46, [240, 180, 70]),
            (w * 0.78, h * 0.74, [158, 96, 186]),
        ];
        for (index, &(x, y, petal)) in blooms.iter().enumerate() {
            // The stem.
            for t in 0..40 {
                let ty = y + t as f32 * (h * 0.008);
                let tx = x
                    + ((t as f32 * 0.18 + index as f32 * 2.0 + (seed % 3) as f32).sin())
                        * w
                        * 0.006;
                canvas.fill_circle(tx, ty, w * 0.008, [72, 128, 66]);
            }
            // The petals around the center.
            for petal_index in 0..6 {
                let angle = std::f32::consts::TAU * petal_index as f32 / 6.0 + index as f32 * 0.5;
                canvas.fill_circle(
                    x + angle.cos() * w * 0.052,
                    y + angle.sin() * w * 0.052,
                    w * 0.034,
                    petal,
                );
            }
            canvas.fill_circle(x, y, w * 0.026, [250, 214, 92]);
            // A couple of leaves on the stem.
            canvas.fill_ellipse(
                x - w * 0.05,
                y + h * 0.10,
                w * 0.040,
                w * 0.018,
                [88, 148, 76],
            );
            canvas.fill_ellipse(
                x + w * 0.05,
                y + h * 0.18,
                w * 0.040,
                w * 0.018,
                [88, 148, 76],
            );
        }
    }

    /// The beach: the sea, the sand and a palm at the left.
    fn beach(canvas: &mut Canvas, seed: usize) {
        let w = canvas.width as f32;
        let h = canvas.height as f32;
        let horizon = h * 0.44;
        for y in 0..canvas.height {
            let t = y as f32 / horizon.max(1.0);
            let color = mix([110, 190, 226], [196, 232, 244], t.min(1.0));
            for x in 0..canvas.width {
                canvas.set(x as i32, y as i32, color);
            }
        }
        canvas.blend_circle(w * 0.80, h * 0.16, 26.0, [255, 246, 200], 0.9);
        for radius in [40.0, 54.0] {
            canvas.blend_circle(w * 0.80, h * 0.16, radius, [255, 246, 200], 0.20);
        }
        // The sea, with a lighter line where it meets the sand.
        for y in horizon as i32..(h * 0.66) as i32 {
            let t = (y as f32 - horizon) / (h * 0.22);
            let color = mix([36, 148, 164], [92, 196, 200], t);
            for x in 0..canvas.width {
                canvas.set(x as i32, y, color);
            }
        }
        for x in 0..canvas.width as i32 {
            let wave = ((x as f32 * 0.09 + (seed % 6) as f32).sin() * 3.0).round() as i32;
            canvas.set(x, h as i32 * 66 / 100 + wave, [230, 246, 248]);
        }
        // The sand.
        for y in (h * 0.66) as i32..canvas.height as i32 {
            let t = (y as f32 - h * 0.66) / (h * 0.34);
            let color = mix([238, 216, 168], [216, 188, 138], t);
            for x in 0..canvas.width {
                canvas.set(x as i32, y, color);
            }
        }
        // The palm: a trunk that bends and five leaves at the top.
        let top_x = w * 0.17;
        let top_y = h * 0.10;
        for t in 0..30 {
            let t = t as f32 / 30.0;
            let x = w * 0.08 + t * (top_x - w * 0.08);
            let y = h * 0.72 - t * (h * 0.62);
            canvas.fill_circle(x + t * t * 8.0, y, 9.0 - t * 4.0, [110, 78, 48]);
        }
        for index in 0..5 {
            let angle = std::f32::consts::PI * (0.15 + index as f32 * 0.18);
            for t in 0..26 {
                let t = t as f32 / 26.0;
                let reach = w * 0.16 * t;
                let droop = t * t * h * 0.10;
                let x = top_x + angle.cos() * reach;
                let y = top_y + angle.sin() * reach * 0.4 + droop;
                canvas.fill_circle(x, y, 7.0 - t * 4.5, [58, 128, 74]);
            }
        }
        // A coconut.
        canvas.fill_circle(top_x + 6.0, top_y + 12.0, 6.0, [94, 62, 38]);
    }
}

/// Create the media tree of the profile, private to its owner.
fn ensure_media_dir(media_dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(media_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(media_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Restrict a file of the profile to its owner: the media is private.
#[cfg(unix)]
fn restrict_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

/// Restrict a file of the profile to its owner: the media is private.
///
/// Windows has no mode bits: the profile directory is already per-user
/// (`%LOCALAPPDATA%`), so the inherited ACL is the boundary.
#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Publish an event of the fake on the stream of the client.
pub fn emit_fake_event(
    incoming: &async_channel::Sender<BridgeMessage>,
    sequence: &Mutex<u64>,
    event: backend_event::Event,
) -> bool {
    // A poisoned lock means a thread died holding it: the counter is still
    // usable, so the stream keeps going.
    let mut sequence = match sequence.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let envelope = Envelope {
        protocol_version: PROTOCOL_VERSION,
        request_id: 0,
        body: Some(envelope::Body::Event(BackendEvent {
            sequence: *sequence,
            event: Some(event),
        })),
    };
    *sequence = sequence.saturating_add(1);
    incoming
        .send_blocking(BridgeMessage::Envelope(Box::new(envelope)))
        .is_ok()
}

/// The live loop of the fake: it publishes a message and the derived chat
/// update once the handshake is done.
pub fn fake_live_loop(
    incoming: async_channel::Sender<BridgeMessage>,
    sequence: Arc<Mutex<u64>>,
    handshaken: Arc<AtomicBool>,
) {
    while !handshaken.load(Ordering::Acquire) {
        if incoming.is_closed() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    // The deterministic fixture also exercises the real-time path: incoming
    // messages appear at the bottom while the chat row moves to the top.
    thread::sleep(LIVE_EVENT_DELAY);
    for counter in 1_u64.. {
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis() as i64);
        let (message_event, chat_event) = fake_live_event_pair(counter, timestamp_ms);
        // Match the real backend: publish the stored message before its derived
        // chat preview/unread-count update.
        if !emit_fake_event(&incoming, &sequence, message_event)
            || !emit_fake_event(&incoming, &sequence, chat_event)
        {
            return;
        }
        thread::sleep(LIVE_EVENT_INTERVAL);
    }
}

/// The pair of events of one incoming message: the message and the chat it
/// updates.
pub fn fake_live_event_pair(
    counter: u64,
    timestamp_ms: i64,
) -> (backend_event::Event, backend_event::Event) {
    // The live message lands on the first chat, a group: a member that is
    // not you writes, the way a real conversation wakes up.
    let member = group_member(0, ((counter.saturating_sub(1)) % 12) as usize);
    let text = LIVE_TEXTS[((counter.saturating_sub(1)) % LIVE_TEXTS.len() as u64) as usize];
    let message = Message {
        id: format!("live-{counter}"),
        chat_id: "chat-0".into(),
        sender_id: format!("{}@s.whatsapp.net", member.digits),
        sender_name: member.name.into(),
        sender_phone_number: member.phone.into(),
        timestamp_ms,
        status: MessageStatus::Delivered as i32,
        content: Some(proto_message::Content::Text(TextContent {
            text: text.into(),
            link_preview: None,
        })),
        ..Default::default()
    };
    let mut chat = fake_chat(0);
    chat.last_message_preview = message
        .content
        .as_ref()
        .and_then(|content| match content {
            proto_message::Content::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .unwrap_or_default();
    chat.last_message_timestamp_ms = timestamp_ms;
    chat.unread_count = 1;
    (
        backend_event::Event::MessageUpserted(MessageUpserted {
            message: Some(message),
        }),
        backend_event::Event::ChatUpserted(ChatUpserted { chat: Some(chat) }),
    )
}

/// A chat of the dataset.
///
/// The dataset is deterministic: the index of the chat decides every field,
/// so the visual QA of the UI has a stable reference.
/// The chat of the fixture for an id, with the flags the client changed.
///
/// The setters return the whole chat, the way the real sidecar does: a client
/// that replaces the row with a partial one loses the title and the avatar.
pub fn fixture_chat(chat_id: &str, settings: Option<&Chat>) -> Chat {
    let number = chat_id
        .strip_prefix("chat-")
        .and_then(|rest| rest.parse::<usize>().ok());
    let mut chat = match number {
        Some(number) => fake_chat(number),
        None => Chat {
            id: chat_id.to_string(),
            kind: ChatKind::Direct as i32,
            ..Default::default()
        },
    };
    if let Some(settings) = settings {
        chat.pinned = settings.pinned;
        chat.archived = settings.archived;
        chat.muted = settings.muted;
        chat.blocked = settings.blocked;
        chat.mute_until_ms = settings.mute_until_ms;
    }
    chat
}

pub fn fake_chat(id: usize) -> Chat {
    let group = is_group_id(id);
    Chat {
        id: format!("chat-{id}"),
        kind: if group {
            ChatKind::Group as i32
        } else {
            ChatKind::Direct as i32
        },
        title: if group {
            group_title(group_index(id))
        } else {
            direct_title(id)
        },
        // The preview is a message of the conversation, the one the row
        // would show: the pool depends on the sender, like the messages.
        last_message_preview: message_text(id).into(),
        last_message_timestamp_ms: fake_timestamp_ms(id),
        unread_count: (id.is_multiple_of(7)) as u32 * ((id % 4) + 1) as u32,
        pinned: id < 3,
        phone_number: if !group {
            direct_phone(id)
        } else {
            String::new()
        },
        contact_name: if !group {
            direct_title(id)
        } else {
            String::new()
        },
        push_name: if !group {
            direct_push(id)
        } else {
            String::new()
        },
        ..Default::default()
    }
}

/// The moment the fake counts as "now".
///
/// The history of the fake goes back from this moment, so the app shows
/// real hours and real days (and the separator of the day has something to
/// separate). The moment is taken once, so the times of the messages do not
/// move while the app is open.
fn fake_now_ms() -> i64 {
    static NOW: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *NOW.get_or_init(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_millis() as i64)
            .unwrap_or(0)
    })
}

/// The gap between two messages of the fake, in milliseconds.
///
/// Ninety seconds: with a thousand messages the history spans a bit more
/// than a day, so the app shows "Today" and "Yesterday".
const FAKE_STEP_MS: i64 = 90_000;

/// The moment of a message of the dataset.
///
/// The newest message is the one of now, and the older ones go back.
fn fake_timestamp_ms(id: usize) -> i64 {
    let total = MESSAGE_TOTAL.max(id + 1) as i64;
    fake_now_ms() - (total - id as i64) * FAKE_STEP_MS
}

/// A message of the dataset.
///
/// Every field is decided by the index of the message: the sender, the
/// content, the photo, the reaction and the quote. The senders are the
/// people of the roster: in a group the members take turns, in a direct
/// conversation the person of the chat signs the half that is not yours.
pub fn fake_message(chat_id: &str, id: usize, profile_dir: &Path) -> Message {
    let sender = chat_sender(chat_id, id);
    let from_me = id.is_multiple_of(2);
    Message {
        id: format!("message-{id}"),
        chat_id: chat_id.into(),
        sender_id: if from_me {
            "me@s.whatsapp.net".into()
        } else {
            sender.jid
        },
        sender_name: if from_me { "You".into() } else { sender.name },
        sender_phone_number: if from_me { String::new() } else { sender.phone },
        from_me,
        timestamp_ms: fake_timestamp_ms(id),
        status: MessageStatus::Read as i32,
        content: Some(if id.is_multiple_of(37) && id > 0 {
            // A real picture on disk, so the media path (thumbnail, full
            // viewer, gallery) is exercised by the fake backend instead of
            // showing an endless loading placeholder. The caption names the
            // scene the picture paints.
            let picture = ensure_fake_image(profile_dir, id);
            let path = picture.to_string_lossy().into_owned();
            proto_message::Content::Image(ImageContent {
                caption: photo_caption(id).into(),
                mime_type: "image/png".into(),
                local_path: path.clone(),
                thumbnail_path: path,
                width: 320,
                height: 240,
                file_size: std::fs::metadata(&picture)
                    .map(|meta| meta.len())
                    .unwrap_or(0),
                downloadable: true,
                ..Default::default()
            })
        } else if let Some(attachment) = fake_attachment_of(id) {
            // A real file on disk, so the media path (document, video, audio
            // and voice note) is exercised by the fake backend instead of
            // showing a message with nothing to open.
            fake_attachment_content(attachment, profile_dir)
        } else {
            proto_message::Content::Text(TextContent {
                text: if id.is_multiple_of(9) {
                    LONG_TEXT
                } else {
                    message_text(id)
                }
                .into(),
                link_preview: (id.is_multiple_of(13)).then(|| LinkPreview {
                    url: "https://www.lagaceta.com.ar/".into(),
                    title: "La Gaceta — Las noticias de Tucumán".into(),
                    description: "El diario de Tucumán, actualizado durante todo el día.".into(),
                    ..Default::default()
                }),
            })
        }),
        reactions: if id.is_multiple_of(11) {
            // The reaction of the friend of the chat on the message: the
            // same person that signs the conversation.
            let reactor = chat_sender(chat_id, 11);
            vec![Reaction {
                chat_id: chat_id.into(),
                message_id: format!("message-{id}"),
                sender_id: reactor.jid,
                emoji: "🐈".into(),
                timestamp_ms: id as i64,
                sender_name: reactor.name,
                sender_phone_number: reactor.phone,
                ..Default::default()
            }]
        } else {
            Vec::new()
        },
        reply_to_message_id: if id > 0 && id.is_multiple_of(13) {
            format!("message-{}", id - 1)
        } else {
            String::new()
        },
        ..Default::default()
    }
}

/// A small PNG writer: the fake needs real image files on disk and the
/// crate may not depend on `image`.
///
/// The container is a PNG with the "Sub" filter and a fixed-Huffman deflate
/// stream. The pixels are the ones of the origin; only the bytes of the
/// container differ from the ones the `image` crate would write.
mod png {
    /// The signature of a PNG file.
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

    /// The bytes per pixel of a truecolor image.
    const BYTES_PER_PIXEL: usize = 3;

    /// Write an RGB image as a PNG.
    pub fn encode_rgb(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(pixels.len() / 2 + 128);
        out.extend_from_slice(&SIGNATURE);
        let mut header = Vec::with_capacity(13);
        header.extend_from_slice(&width.to_be_bytes());
        header.extend_from_slice(&height.to_be_bytes());
        // 8 bits per channel, truecolor, deflate, adaptive filtering and no
        // interlacing.
        header.extend_from_slice(&[8, 2, 0, 0, 0]);
        write_chunk(&mut out, b"IHDR", &header);
        let raw = filter_rows(width, height, pixels);
        write_chunk(&mut out, b"IDAT", &zlib_compress(&raw));
        write_chunk(&mut out, b"IEND", &[]);
        out
    }

    /// Filter the rows of the image with the "Sub" filter: every byte minus
    /// the byte of the same channel of the pixel on the left.
    ///
    /// The gradient of the fake is smooth, so the filter leaves long runs of
    /// zeroes and the deflate stream below shrinks the file a lot.
    fn filter_rows(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
        let stride = width as usize * BYTES_PER_PIXEL;
        let mut raw = Vec::with_capacity((stride + 1) * height as usize);
        for y in 0..height as usize {
            raw.push(1);
            let row = &pixels[y * stride..(y + 1) * stride];
            for (index, byte) in row.iter().enumerate() {
                let left = if index >= BYTES_PER_PIXEL {
                    row[index - BYTES_PER_PIXEL]
                } else {
                    0
                };
                raw.push(byte.wrapping_sub(left));
            }
        }
        raw
    }

    /// Wrap the filtered rows in a zlib stream.
    fn zlib_compress(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len() / 4 + 16);
        // Deflate with a 32K window and the fastest level: the header is
        // valid because 0x7801 is a multiple of 31.
        out.push(0x78);
        out.push(0x01);
        deflate_fixed(data, &mut out);
        out.extend_from_slice(&adler32(data).to_be_bytes());
        out
    }

    /// Write the data as a single fixed-Huffman deflate block.
    ///
    /// The encoder knows one trick: a run of identical bytes becomes a
    /// literal and a length/distance pair with the distance one, which is
    /// what the filtered gradient needs.
    fn deflate_fixed(data: &[u8], out: &mut Vec<u8>) {
        let mut writer = BitWriter::new(out);
        writer.write_bits(1, 1); // The last block.
        writer.write_bits(1, 2); // The fixed Huffman codes.
        let mut index = 0;
        while index < data.len() {
            let mut run = 1;
            while index + run < data.len() && data[index + run] == data[index] && run < 258 {
                run += 1;
            }
            if run >= 4 {
                // The first byte of the run is a literal: a match with the
                // distance one copies the byte before it, so the run needs
                // its own byte before the match repeats it.
                write_literal(&mut writer, data[index]);
                write_length(&mut writer, run - 1);
                write_distance(&mut writer, 1);
                index += run;
            } else {
                for offset in 0..run {
                    write_literal(&mut writer, data[index + offset]);
                }
                index += run;
            }
        }
        write_symbol(&mut writer, 256); // The end of the block.
        writer.flush();
    }

    /// The base of every length of the deflate format.
    const LENGTH_BASE: [u16; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
        131, 163, 195, 227, 258,
    ];

    /// The extra bits of every length of the deflate format.
    const LENGTH_EXTRA: [u8; 29] = [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
    ];

    /// The base of every distance of the deflate format.
    const DISTANCE_BASE: [u16; 30] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
        2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
    ];

    /// The extra bits of every distance of the deflate format.
    const DISTANCE_EXTRA: [u8; 30] = [
        0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
        13, 13,
    ];

    /// Write a literal byte with the fixed Huffman codes.
    fn write_literal(writer: &mut BitWriter, byte: u8) {
        write_symbol(writer, u16::from(byte));
    }

    /// Write a symbol with the fixed Huffman codes of the deflate format.
    fn write_symbol(writer: &mut BitWriter, symbol: u16) {
        match symbol {
            0..=143 => writer.write_code(0x30 + u32::from(symbol), 8),
            144..=255 => writer.write_code(0x190 + u32::from(symbol) - 144, 9),
            256..=279 => writer.write_code(u32::from(symbol) - 256, 7),
            _ => writer.write_code(0xc0 + u32::from(symbol) - 280, 8),
        }
    }

    /// Write a length with the fixed Huffman codes.
    fn write_length(writer: &mut BitWriter, length: usize) {
        let mut code = 0;
        while code + 1 < LENGTH_BASE.len() && LENGTH_BASE[code + 1] as usize <= length {
            code += 1;
        }
        let extra = LENGTH_EXTRA[code];
        let bits = extra as u32;
        write_symbol(writer, 257 + code as u16);
        if bits > 0 {
            writer.write_bits((length - LENGTH_BASE[code] as usize) as u32, bits);
        }
    }

    /// Write a distance with the fixed Huffman codes.
    fn write_distance(writer: &mut BitWriter, distance: usize) {
        let mut code = 0;
        while code + 1 < DISTANCE_BASE.len() && DISTANCE_BASE[code + 1] as usize <= distance {
            code += 1;
        }
        let extra = DISTANCE_EXTRA[code];
        let bits = extra as u32;
        writer.write_code(code as u32, 5);
        if bits > 0 {
            writer.write_bits((distance - DISTANCE_BASE[code] as usize) as u32, bits);
        }
    }

    /// The writer of the bits of a deflate stream.
    struct BitWriter<'a> {
        out: &'a mut Vec<u8>,
        buffer: u32,
        bits: u32,
    }

    impl<'a> BitWriter<'a> {
        fn new(out: &'a mut Vec<u8>) -> Self {
            Self {
                out,
                buffer: 0,
                bits: 0,
            }
        }

        /// Write the bits of a value, the least significant one first: the
        /// extra bits of the deflate format.
        fn write_bits(&mut self, value: u32, count: u32) {
            self.buffer |= value << self.bits;
            self.bits += count;
            while self.bits >= 8 {
                self.out.push((self.buffer & 0xff) as u8);
                self.buffer >>= 8;
                self.bits -= 8;
            }
        }

        /// Write a Huffman code, the most significant bit first.
        fn write_code(&mut self, code: u32, count: u32) {
            for bit in (0..count).rev() {
                self.write_bits((code >> bit) & 1, 1);
            }
        }

        /// Write the last byte of the stream.
        fn flush(&mut self) {
            if self.bits > 0 {
                self.out.push((self.buffer & 0xff) as u8);
                self.buffer = 0;
                self.bits = 0;
            }
        }
    }

    /// Write a chunk of a PNG file: its length, its kind, its data and the
    /// CRC of the kind and the data.
    fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let mut crc_input = Vec::with_capacity(4 + data.len());
        crc_input.extend_from_slice(kind);
        crc_input.extend_from_slice(data);
        out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    }

    /// The CRC-32 of the chunks of a PNG file.
    fn crc32(data: &[u8]) -> u32 {
        let mut table = [0_u32; 256];
        for (index, entry) in table.iter_mut().enumerate() {
            let mut value = index as u32;
            for _ in 0..8 {
                value = if value & 1 != 0 {
                    0xedb88320 ^ (value >> 1)
                } else {
                    value >> 1
                };
            }
            *entry = value;
        }
        let mut crc = 0xffff_ffff_u32;
        for byte in data {
            crc = table[((crc ^ u32::from(*byte)) & 0xff) as usize] ^ (crc >> 8);
        }
        crc ^ 0xffff_ffff
    }

    /// The checksum of the zlib stream.
    fn adler32(data: &[u8]) -> u32 {
        let mut low = 1_u32;
        let mut high = 0_u32;
        for byte in data {
            low = (low + u32::from(*byte)) % 65521;
            high = (high + low) % 65521;
        }
        (high << 16) | low
    }
}

/// A small PCM WAV writer: the fake needs a real audio file on disk and the
/// crate may not depend on an encoder.
///
/// The two waves of the dataset are melodies instead of a plain tone: a
/// song of arpeggios for the audio message and a hum with vibrato for the
/// voice note. Both are mono of sixteen bits at 22 050 hertz: files that
/// every player of the system opens and that the inline player of the
/// interface really plays.
mod wav {
    /// The sample rate of both melodies.
    const SAMPLE_RATE: u32 = 22_050;

    /// The tempo of the song, in seconds per eighth note.
    const SONG_BEAT: f32 = 0.3125;

    /// The tempo of the hum, in seconds per beat.
    const HUM_BEAT: f32 = 0.35;

    /// The frequency of a note, in semitones from the A4 of 440 hertz.
    fn hz(semi: f32) -> f32 {
        440.0 * (2.0f32).powf(semi / 12.0)
    }

    /// One note of a melody: when it starts and how long it lasts in beats,
    /// its pitch in semitones from A4, how loud it is and whether the
    /// pitch wobbles like a voice.
    struct Note {
        start: f32,
        beats: f32,
        semi: f32,
        gain: f32,
        vibrato: bool,
    }

    /// The notes of the song: four chords of arpeggio (C, G, Am, F) and a
    /// bass that sustains each one.
    fn song_notes() -> Vec<Note> {
        const TRIADS: [[f32; 3]; 4] = [
            [-9.0, -5.0, -2.0],   // C:  C4, E4, G4
            [-14.0, -10.0, -7.0], // G:  G3, B3, D4
            [-12.0, -9.0, -5.0],  // Am: A3, C4, E4
            [-16.0, -12.0, -9.0], // F:  F3, A3, C4
        ];
        const BASSES: [f32; 4] = [-21.0, -26.0, -24.0, -28.0];
        const PATTERN: [usize; 8] = [0, 1, 2, 1, 0, 1, 2, 1];
        let mut notes = Vec::with_capacity(32);
        for (chord, triad) in TRIADS.iter().enumerate() {
            let start = chord as f32 * 8.0;
            notes.push(Note {
                start,
                beats: 8.0,
                semi: BASSES[chord],
                gain: 0.13,
                vibrato: false,
            });
            for (eighth, note) in PATTERN.iter().enumerate() {
                notes.push(Note {
                    start: start + eighth as f32,
                    beats: 1.0,
                    semi: triad[*note],
                    gain: 0.20,
                    vibrato: false,
                });
            }
        }
        notes
    }

    /// The notes of the voice note: a hum with vibrato, the phrase of a
    /// tune every ear knows, the way one hums into a voice message.
    fn hum_notes() -> Vec<Note> {
        const PHRASE: [(f32, f32); 16] = [
            (-5.0, 1.0),
            (-7.0, 1.0),
            (-9.0, 1.0),
            (-7.0, 1.0),
            (-5.0, 1.0),
            (-5.0, 1.0),
            (-5.0, 2.0),
            (-7.0, 1.0),
            (-7.0, 1.0),
            (-7.0, 2.0),
            (-5.0, 1.0),
            (-2.0, 1.0),
            (-2.0, 2.0),
            (-5.0, 1.0),
            (-7.0, 1.0),
            (-9.0, 4.0),
        ];
        let mut start = 0.0_f32;
        PHRASE
            .iter()
            .map(|&(semi, beats)| {
                let note = Note {
                    start,
                    beats,
                    semi,
                    gain: 0.24,
                    vibrato: true,
                };
                start += beats;
                note
            })
            .collect()
    }

    /// Render the notes into samples: every note adds its wave with an
    /// envelope over the others, and the sum is normalized so nothing
    /// clips. The melody lasts exactly the whole seconds the card of the
    /// attachment declares, so the player and the message agree.
    fn render(notes: &[Note], beat: f32) -> Vec<f32> {
        let total = seconds(notes, beat) as usize * SAMPLE_RATE as usize;
        let mut mix = vec![0.0_f32; total];
        for note in notes {
            let start_at = (note.start * beat * SAMPLE_RATE as f32).round() as usize;
            let length = (note.beats * beat * SAMPLE_RATE as f32).round() as usize;
            let frequency = hz(note.semi);
            let duration = note.beats * beat;
            let mut phase = 0.0_f32;
            for index in 0..length {
                let time = index as f32 / SAMPLE_RATE as f32;
                let wobble = if note.vibrato {
                    1.0 + 0.006 * (std::f32::consts::TAU * 5.5 * time).sin()
                } else {
                    1.0
                };
                phase += std::f32::consts::TAU * frequency * wobble / SAMPLE_RATE as f32;
                let attack = (time / 0.012).min(1.0);
                let release = ((duration - time) / 0.09).clamp(0.0, 1.0);
                let envelope = attack * release * (1.0 - 0.30 * time / duration);
                let wave = phase.sin() + 0.45 * (2.0 * phase).sin() + 0.18 * (3.0 * phase).sin();
                mix[start_at + index] += wave * envelope * note.gain;
            }
        }
        let loudest = mix
            .iter()
            .fold(0.0_f32, |top, sample| top.max(sample.abs()));
        if loudest > 0.0 {
            let scale = 0.82 / loudest;
            for sample in &mut mix {
                *sample *= scale;
            }
        }
        mix
    }

    /// The seconds a melody lasts, for the card of the attachment.
    fn seconds(notes: &[Note], beat: f32) -> u32 {
        let end = notes
            .iter()
            .map(|note| note.start + note.beats)
            .fold(0.0_f32, f32::max);
        (end * beat).ceil() as u32
    }

    /// The song of the audio message.
    pub fn encode_song() -> Vec<u8> {
        pcm(&render(&song_notes(), SONG_BEAT))
    }

    /// The seconds the song lasts.
    pub fn song_seconds() -> u32 {
        seconds(&song_notes(), SONG_BEAT)
    }

    /// The hum of the voice note.
    pub fn encode_hum() -> Vec<u8> {
        pcm(&render(&hum_notes(), HUM_BEAT))
    }

    /// The seconds the hum lasts.
    pub fn hum_seconds() -> u32 {
        seconds(&hum_notes(), HUM_BEAT)
    }

    /// Wrap the samples in the container of a mono PCM WAV.
    fn pcm(samples: &[f32]) -> Vec<u8> {
        let mut data = Vec::with_capacity(samples.len() * 2);
        for sample in samples {
            let value = (sample * i16::MAX as f32) as i16;
            data.extend_from_slice(&value.to_le_bytes());
        }
        let mut out = Vec::with_capacity(44 + data.len());
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
        out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }
}

/// A small PDF writer: the fake needs a real document on disk and the crate
/// may not depend on a library of documents.
///
/// The file is the header of the version 1.4, five objects (the catalogue,
/// the pages, the page, the text of the page and the font), the table of the
/// cross references and the marker of the end of the file.
mod pdf {
    /// The header of a PDF file of the version 1.4.
    const HEADER: &str = "%PDF-1.4\n";

    /// Write a document of one page with a line of text.
    pub fn encode_one_page(text: &str) -> Vec<u8> {
        let content = format!("BT /F1 18 Tf 72 760 Td ({}) Tj ET\n", escape(text));
        let mut out = Vec::new();
        out.extend_from_slice(HEADER.as_bytes());
        // Every object of the document, with the offset where it starts.
        let mut offsets = Vec::new();
        offsets.push(out.len());
        out.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        offsets.push(out.len());
        out.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        offsets.push(out.len());
        out.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
              /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>\nendobj\n",
        );
        offsets.push(out.len());
        out.extend_from_slice(
            format!("4 0 obj\n<< /Length {} >>\nstream\n", content.len()).as_bytes(),
        );
        out.extend_from_slice(content.as_bytes());
        out.extend_from_slice(b"endstream\nendobj\n");
        offsets.push(out.len());
        out.extend_from_slice(
            b"5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
        );
        // The table of the cross references: the free entry of the head and
        // the offset of every object.
        let table = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n", offsets.len() + 1).as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for offset in &offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{table}\n%%EOF\n",
                offsets.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    /// Escape the parentheses and the backslashes of the text of the page.
    fn escape(text: &str) -> String {
        text.replace('\\', r"\\")
            .replace('(', r"\(")
            .replace(')', r"\)")
    }
}

/// A small MP4 writer: the fake needs a real video file on disk and the
/// crate may not depend on an encoder.
///
/// A movie is not something the fake can build: the file carries the box of
/// the type of the format (its brands) and a box of data, which is what the
/// system needs to recognize it as a video and to open it with its player.
mod mp4 {
    /// The brands that the header of the file declares.
    const BRANDS: [&[u8]; 4] = [b"isom", b"iso2", b"avc1", b"mp41"];

    /// Write the header of the format and a chunk of data of the given size.
    pub fn encode_stub(payload: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(payload + 64);
        // The box of the type: the size, the name, the brand and the version.
        let size = 8 + 4 + 4 + BRANDS.iter().map(|brand| brand.len()).sum::<usize>();
        out.extend_from_slice(&(size as u32).to_be_bytes());
        out.extend_from_slice(b"ftyp");
        out.extend_from_slice(BRANDS[0]);
        out.extend_from_slice(&0x0000_0200u32.to_be_bytes());
        for brand in &BRANDS[1..] {
            out.extend_from_slice(brand);
        }
        // The box of the data of the video.
        out.extend_from_slice(&((payload + 8) as u32).to_be_bytes());
        out.extend_from_slice(b"mdat");
        out.resize(out.len() + payload, 0);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A private directory of the test run for the media of the fake.
    fn profile_dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("whatfast-fake-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create the profile of the test");
        path
    }

    #[test]
    fn the_dataset_carries_a_file_of_every_kind() {
        let profile = profile_dir("attachments");
        let cases = [
            (DOCUMENT_EVERY * 16, "document", DOCUMENT_FILE, false),
            (VIDEO_EVERY * 13, "video", VIDEO_FILE, false),
            (AUDIO_EVERY * 14, "audio", AUDIO_FILE, false),
            (VOICE_NOTE_EVERY * 10, "audio", VOICE_NOTE_FILE, true),
        ];
        for (id, kind, file_name, voice_note) in cases {
            let message = fake_message("chat-0", id, &profile);
            let Some(proto_message::Content::Attachment(attachment)) = message.content else {
                panic!("the message {id} carries a file");
            };
            assert_eq!(attachment.kind, kind);
            assert_eq!(attachment.file_name, file_name);
            assert_eq!(attachment.voice_note, voice_note);
            assert!(attachment.downloadable);
            assert!(attachment.file_size > 0);
            let path = PathBuf::from(&attachment.local_path);
            assert!(path.is_file(), "the file of the message {id} is on disk");
            assert!(path.starts_with(profile.join("media")));
        }
    }

    #[test]
    fn the_files_of_the_dataset_are_the_ones_of_the_formats() {
        let profile = profile_dir("formats");
        let document = ensure_fake_document(&profile);
        let bytes = std::fs::read(&document).expect("the document of the fake");
        assert!(bytes.starts_with(b"%PDF-1.4"));
        assert!(bytes.ends_with(b"%%EOF\n"));

        let video = ensure_fake_video(&profile);
        let bytes = std::fs::read(&video).expect("the video of the fake");
        assert_eq!(&bytes[4..8], b"ftyp");
        assert_eq!(&bytes[8..12], b"isom");

        let audio = ensure_fake_audio_named(&profile, AUDIO_FILE);
        let bytes = std::fs::read(&audio).expect("the audio of the fake");
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
    }

    #[test]
    fn only_the_messages_of_the_strides_carry_a_file() {
        assert!(fake_attachment_of(0).is_none());
        assert!(fake_attachment_of(1).is_none());
        assert_eq!(
            fake_attachment_of(DOCUMENT_EVERY * 16),
            Some(FakeAttachment::Document)
        );
        assert_eq!(
            fake_attachment_of(VIDEO_EVERY * 13),
            Some(FakeAttachment::Video)
        );
        assert_eq!(
            fake_attachment_of(AUDIO_EVERY * 14),
            Some(FakeAttachment::Audio)
        );
        assert_eq!(
            fake_attachment_of(VOICE_NOTE_EVERY * 10),
            Some(FakeAttachment::VoiceNote)
        );
        // The photo has its own stride: no message carries a photo and a
        // file at the same time.
        for id in (0..PERF_MESSAGE_TOTAL).filter(|id| id % 37 == 0) {
            assert!(
                fake_attachment_of(id).is_none(),
                "the message {id} carries the photo and a file"
            );
        }
    }

    #[test]
    fn the_audio_of_the_dataset_is_a_playable_melody() {
        let profile = profile_dir("melodies");
        let song = ensure_fake_audio_named(&profile, AUDIO_FILE);
        let bytes = std::fs::read(&song).expect("the song of the fake");
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        // The header: 22 050 hertz, mono, sixteen bits.
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            22_050
        );
        let frames = u32::from_le_bytes(bytes[40..44].try_into().unwrap()) / 2;
        assert_eq!(frames / 22_050, wav::song_seconds());

        let hum = ensure_fake_audio_named(&profile, VOICE_NOTE_FILE);
        let bytes = std::fs::read(&hum).expect("the voice note of the fake");
        let frames = u32::from_le_bytes(bytes[40..44].try_into().unwrap()) / 2;
        assert_eq!(frames / 22_050, wav::hum_seconds());
    }

    #[test]
    fn the_captions_of_the_photos_name_the_scenes() {
        // The caption of a photo and the scene it paints come from the same
        // table: the caption of a sunset never paints a pizza. The first
        // photo of the dataset is the message 37, the second scene of the
        // table.
        assert_eq!(photo_caption(37), "Cafecito de la mañana ☕");
        assert_eq!(photo_caption(37 * 6), "Flores del patio 🌸");
        // The eighth photo is the first scene again: the table cycles with
        // the photo index.
        assert_eq!(photo_caption(37 * 8), "Atardecer en el dique 🌅");
    }

    #[test]
    fn the_titles_of_the_first_chats_are_people_and_groups() {
        // The first screens of the list read like a real agenda: the family
        // group first, then the people of the roster.
        assert_eq!(fake_chat(0).title, "Familia Bazán 🏡");
        assert_eq!(fake_chat(1).title, "Bruno Salazar");
        assert_eq!(fake_chat(5).title, "Fútbol de los martes ⚽");
        assert_eq!(fake_chat(9).title, "Matías Aráoz");
        // Beyond the roster the names still combine like real ones.
        let beyond = fake_chat(101);
        assert!(
            beyond.title.contains(' '),
            "the title beyond the roster is a full name: {}",
            beyond.title
        );
    }
}
