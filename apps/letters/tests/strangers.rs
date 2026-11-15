//! A stranger's history is tried by the app run a second time, which is where a harmful one ends.

use std::path::Path;

use letters::strangers;
use letters_model::{Block, BlockKind, Document, Model, ReplicaId};

const APP: &str = env!("CARGO_BIN_EXE_letters");

/// A history that does no harm, of a document long enough that a pipe does not hold it at once.
fn sound() -> Vec<u8> {
    let line = |at: usize| {
        Block::text(format!("p{at}").as_str(), BlockKind::Paragraph, format!("Slate {at} of the roof, as laid."))
    };
    let model = Model::new(
        Document { blocks: (0..4000).map(line).collect(), ..Document::default() },
        ReplicaId::new("a device"),
    );
    model.save()
}

#[test]
fn a_history_stands_only_where_the_app_run_again_says_so() {
    let sound = sound();
    assert!(sound.len() > 1 << 16, "longer than a pipe holds: {}", sound.len());
    assert_eq!(strangers::check_by(Path::new(APP), &sound), Ok(()));

    let harmful: [(&str, &[u8]); 5] = [
        ("Loro panics on it", include_bytes!("../../../fuzz/corpus/history/a-history-loro-panics-on.bin")),
        ("Loro panics on it too", include_bytes!("../../../fuzz/corpus/history/a-second-history-loro-panics-on.bin")),
        ("its save cannot be read", include_bytes!("../../../fuzz/corpus/history/a-history-loro-cannot-write-out.bin")),
        ("it is no history", b"not a history"),
        ("it is cut short", &sound[..sound.len() / 2]),
    ];
    for (why, history) in harmful {
        assert!(strangers::check_by(Path::new(APP), history).is_err(), "{why}");
    }
}

#[test]
fn nothing_stands_where_the_app_cannot_be_run() {
    let nowhere = std::env::temp_dir().join("letters-strangers-no-such-app");
    assert!(strangers::check_by(&nowhere, &sound()).is_err());
    // A program that is not the app says nothing of the history, which is no word that it stands.
    assert!(strangers::check_by(&std::env::current_exe().unwrap(), &sound()).is_err());
}
