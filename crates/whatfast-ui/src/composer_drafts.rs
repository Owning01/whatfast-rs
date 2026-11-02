//! Los borradores del compositor, en el disco.
//!
//! El estado de la ventana guarda un `ChatDraft` por conversación: el texto,
//! la cita y las menciones. Antes vivía solo en memoria y un cierre, una
//! actualización o un crash perdía todo lo escrito. Ahora se guarda en
//! `<data>/drafts.json`, con una escritura atómica y un debounce del llamador.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::app::ChatDraft;

/// El archivo de los borradores del perfil.
fn path() -> PathBuf {
    whatfast_model::paths::data_dir().join("drafts.json")
}

/// Los borradores que quedaron la última vez. Un archivo que falta o que no
/// se puede leer deja una lista vacía: el compositor no depende del disco.
pub(crate) fn load() -> HashMap<String, ChatDraft> {
    let Ok(bytes) = std::fs::read(path()) else {
        return HashMap::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Escribe los borradores en el archivo, con un temporal y un rename.
pub(crate) fn save(drafts: &HashMap<String, ChatDraft>) {
    let Ok(bytes) = serde_json::to_vec(drafts) else {
        return;
    };
    let path = path();
    if let Some(parent) = path.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return;
    }
    let temporary = path.with_extension("json.tmp");
    if std::fs::write(&temporary, bytes).is_ok() {
        let _ = std::fs::rename(&temporary, &path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::MentionEntry;

    #[test]
    fn a_draft_round_trips_through_json() {
        let mut drafts = HashMap::new();
        drafts.insert(
            "chat@s.whatsapp.net".to_string(),
            ChatDraft {
                text: "hola".to_string(),
                reply_to_message_id: Some("m1".to_string()),
                mentions: vec![MentionEntry {
                    display_name: "Ana".to_string(),
                    jid: "ana@s.whatsapp.net".to_string(),
                }],
                editing_message_id: None,
            },
        );
        let bytes = serde_json::to_vec(&drafts).unwrap();
        let back: HashMap<String, ChatDraft> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back, drafts);
    }
}
