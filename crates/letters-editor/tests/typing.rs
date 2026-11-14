use letters_editor::{Clip, Editor, InputEvent, Platform};
use letters_model::{Block, BlockKind, Document};

#[derive(Default)]
struct Fake {
    clip: Option<Clip>,
    ids: u32,
}

impl Platform for Fake {
    fn now_ms(&self) -> u64 {
        0
    }
    fn clipboard_read(&mut self) -> Option<Clip> {
        self.clip.clone()
    }
    fn clipboard_write(&mut self, clip: Clip) {
        self.clip = Some(clip);
    }
    fn new_id(&mut self) -> String {
        self.ids += 1;
        format!("t{}", self.ids)
    }
}

#[test]
fn typing_inserts_at_the_caret() {
    let doc = Document { blocks: vec![Block::text("p", BlockKind::Paragraph, "")], ..Document::default() };
    let mut editor = Editor::new(doc);
    for text in ["Hel", "lo"] {
        editor.input(InputEvent::Insert(text.into()), &mut Fake::default());
    }
    assert_eq!(editor.doc().blocks[0].plain_text(), "Hello");
    assert_eq!(editor.selection().unwrap().head.offset, 5);
}
