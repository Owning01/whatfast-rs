//! Port of `tauri/tests/file-drop.test.ts`.
//!
//! Every case of the oracle suite has its own test here, with the same name
//! translated to snake_case.

use whatfast_core::file_drop::{
    AttachmentKind, DroppedFileBatch, FileSendMode, classify_dropped_files,
};

const DOCUMENT_KIND: AttachmentKind = AttachmentKind::Document;
const VIDEO_KIND: AttachmentKind = AttachmentKind::Video;
const AUDIO_KIND: AttachmentKind = AttachmentKind::Audio;

fn paths(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

#[test]
fn homogeneous_image_drops_are_reviewed_as_photos_across_path_formats() {
    assert_eq!(
        classify_dropped_files(&paths(&["/home/me/one.PNG", r"C:\Users\me\two.webp",])),
        DroppedFileBatch {
            mode: FileSendMode::Image,
            attachment_kind: DOCUMENT_KIND,
        },
    );
}

#[test]
fn homogeneous_video_and_audio_drops_retain_their_media_kinds() {
    assert_eq!(
        classify_dropped_files(&paths(&["clip.mp4", "clip.MOV"])),
        DroppedFileBatch {
            mode: FileSendMode::Attachment,
            attachment_kind: VIDEO_KIND,
        },
    );
    assert_eq!(
        classify_dropped_files(&paths(&["voice.opus", "song.mp3"])),
        DroppedFileBatch {
            mode: FileSendMode::Attachment,
            attachment_kind: AUDIO_KIND,
        },
    );
}

#[test]
fn documents_and_mixed_drops_stay_together_as_one_document_batch() {
    for batch in [
        paths(&["report.pdf"]),
        paths(&["photo.jpg", "notes.docx"]),
        paths(&[".env"]),
    ] {
        assert_eq!(
            classify_dropped_files(&batch),
            DroppedFileBatch {
                mode: FileSendMode::Attachment,
                attachment_kind: DOCUMENT_KIND,
            },
        );
    }
}
