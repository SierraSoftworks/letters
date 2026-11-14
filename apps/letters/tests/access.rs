use accesskit::{Action, ActionData, ActionRequest, Node, NodeId, Role, TextSelection, Toggled, TreeId, TreeUpdate};
use letters::access::{self, Request};
use letters_model::{Block, BlockKind, Document, ListKind, Pos, Selection, Tone};

fn document() -> Document {
    let mut callout = Block::new("c", BlockKind::Callout { tone: Tone::Info });
    callout.body = letters_model::Body::Blocks(vec![Block::text("c1", BlockKind::Paragraph, "Reply by Friday.")]);
    Document {
        language: "en-GB".into(),
        blocks: vec![
            Block::text("h", BlockKind::Heading { level: 2 }, "Rates"),
            Block::text("p", BlockKind::Paragraph, "Café opens at 9"),
            Block::text("t", BlockKind::ListItem { list: ListKind::Task { checked: true }, depth: 0 }, "Done"),
            callout,
            Block::new("d", BlockKind::Divider),
        ],
        ..Document::default()
    }
}

fn node(update: &TreeUpdate, id: NodeId) -> &Node {
    &update.nodes.iter().find(|(node, _)| *node == id).unwrap_or_else(|| panic!("no node {id:?}")).1
}

/// The nodes under the document, in reading order.
fn blocks(update: &TreeUpdate) -> Vec<&Node> {
    fn walk<'a>(update: &'a TreeUpdate, id: NodeId, out: &mut Vec<&'a Node>) {
        let found = node(update, id);
        out.push(found);
        for child in found.children() {
            walk(update, *child, out);
        }
    }
    let mut out = Vec::new();
    for child in node(update, access::DOCUMENT).children() {
        walk(update, *child, &mut out);
    }
    out
}

#[test]
fn a_screen_reader_can_read_the_document_in_order() {
    let update = access::tree("Rates", &document(), None);
    assert_eq!(update.tree.as_ref().unwrap().root, access::WINDOW);
    assert_eq!(node(&update, access::WINDOW).label(), Some("Rates"));
    assert_eq!(node(&update, access::DOCUMENT).language(), Some("en-GB"));

    let read: Vec<(Role, Option<&str>)> = blocks(&update).iter().map(|node| (node.role(), node.value())).collect();
    let expected = [
        (Role::Heading, None),
        (Role::TextRun, Some("Rates\n")),
        (Role::Paragraph, None),
        (Role::TextRun, Some("Café opens at 9\n")),
        (Role::ListItem, None),
        (Role::TextRun, Some("Done\n")),
        (Role::Note, None),
        (Role::Paragraph, None),
        (Role::TextRun, Some("Reply by Friday.\n")),
        (Role::Splitter, None),
    ];
    assert_eq!(read, expected);

    let all = blocks(&update);
    assert_eq!(all[0].level(), Some(2));
    assert_eq!(all[4].toggled(), Some(Toggled::True));
}

#[test]
fn every_node_is_reachable_and_ids_are_stable() {
    let update = access::tree("Rates", &document(), None);
    let mut ids: Vec<NodeId> = update.nodes.iter().map(|(id, _)| *id).collect();
    let count = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), count, "node ids are unique");
    assert_eq!(blocks(&update).len() + 2, count, "every node hangs off the window");

    let again = access::tree("Rates", &document(), None);
    assert_eq!(update.nodes, again.nodes);
}

#[test]
fn text_runs_describe_their_characters_and_words() {
    let update = access::tree("Rates", &document(), None);
    let run = blocks(&update)[3];
    let value = run.value().unwrap();
    assert_eq!(run.character_lengths().iter().map(|length| usize::from(*length)).sum::<usize>(), value.len());
    assert_eq!(run.character_lengths().len(), value.chars().count());
    let words: Vec<String> =
        run.word_starts().iter().map(|start| value.chars().skip(usize::from(*start)).take(2).collect()).collect();
    assert_eq!(words, ["Ca", "op", "at", "9\n"]);
}

/// Selections survive the trip to AccessKit and back, including positions after a multi-byte character.
#[test]
fn selections_round_trip_through_the_tree() {
    let doc = document();
    let cases = [
        Selection::caret(Pos::new("p", 0)),
        Selection::caret(Pos::new("p", 5)),
        Selection::caret(Pos::new("p", "Café opens at 9".len())),
        Selection { anchor: Pos::new("h", 2), head: Pos::new("c1", 5) },
        Selection { anchor: Pos::new("t", 4), head: Pos::new("p", 3) },
    ];
    for selection in cases {
        let update = access::tree("Rates", &doc, Some(&selection));
        let exposed: TextSelection = *node(&update, access::DOCUMENT).text_selection().expect("a selection");
        let request = ActionRequest {
            action: Action::SetTextSelection,
            target_tree: TreeId::ROOT,
            target_node: access::DOCUMENT,
            data: Some(ActionData::SetTextSelection(exposed)),
        };
        assert_eq!(access::request(&doc, &request), Some(Request::Select(selection)));
    }
}

#[test]
fn a_selection_that_points_nowhere_is_left_out() {
    let doc = document();
    for pos in [Pos::new("gone", 0), Pos::new("p", 4), Pos::new("p", 999)] {
        let update = access::tree("Rates", &doc, Some(&Selection::caret(pos.clone())));
        assert!(node(&update, access::DOCUMENT).text_selection().is_none(), "{pos:?}");
    }
}

#[test]
fn requests_become_edits_or_are_ignored() {
    let doc = document();
    let request =
        |action, data| ActionRequest { action, target_tree: TreeId::ROOT, target_node: access::DOCUMENT, data };
    let replace = request(Action::ReplaceSelectedText, Some(ActionData::Value("new".into())));
    assert_eq!(access::request(&doc, &replace), Some(Request::Replace("new".into())));
    for ignored in [request(Action::Focus, None), request(Action::SetTextSelection, None), request(Action::Click, None)]
    {
        assert_eq!(access::request(&doc, &ignored), None);
    }
}
