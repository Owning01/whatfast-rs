//! Los borradores del compositor, en el disco.
//!
//! El estado de la ventana guarda un `ChatDraft` por conversación: el texto,
//! la cita y las menciones. Antes vivía solo en memoria y un cierre, una
//! actualización o un crash perdía todo lo escrito. Ahora se guarda en
//! `<data>/drafts.json`, con una escritura atómica y un debounce del llamador.

#[cfg(test)]
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::app::ChatDraft;

/// El archivo de los borradores del perfil.
fn path() -> PathBuf {
    #[cfg(test)]
    if let Some(path) = TEST_PATH.with(|path| path.borrow().clone()) {
        return path;
    }
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

// Los tests no leen ni escriben el perfil del usuario: cada uno apunta el
// módulo a un archivo desechable.
#[cfg(test)]
thread_local! {
    static TEST_PATH: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

#[cfg(test)]
fn use_test_file(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "whatfast-drafts-{label}-{}.json",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    TEST_PATH.with(|current| *current.borrow_mut() = Some(path.clone()));
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::MentionEntry;

    #[test]
    fn a_draft_round_trips_through_json() {
        let file = use_test_file("round-trip");
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
                ..ChatDraft::default()
            },
        );
        save(&drafts);
        assert_eq!(load(), drafts);
        let _ = std::fs::remove_file(file);
    }
}
