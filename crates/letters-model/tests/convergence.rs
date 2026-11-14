//! Replicas that have seen the same edits show the same document, whatever the edits and their order.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::ops::Range;

use letters_model::{
    Block, BlockId, BlockKind, Body, BodyShape, Change, Document, ListKind, Mark, Model, Op, OpError, Pos, ReplicaId,
    Run, Tone,
};
use proptest::collection::vec;
use proptest::prelude::*;

const REPLICAS: usize = 3;

/// An edit by rough place. [`op`] fits it to whatever the document holds when its turn comes.
#[derive(Clone, Debug)]
enum Edit {
    Type { block: usize, at: usize, text: String },
    Delete { block: usize, at: usize, len: usize },
    Mark { block: usize, at: usize, len: usize, mark: Mark, on: bool },
    Link { block: usize, at: usize, len: usize, link: Option<String> },
    Split { block: usize, at: usize, id: usize },
    Join { block: usize },
    Kind { block: usize, kind: usize },
    Insert { parent: Option<usize>, index: usize, kind: usize, id: usize, text: String },
    Remove { block: usize },
}

#[derive(Clone, Debug)]
enum Step {
    Edit {
        replica: usize,
        edits: Vec<Edit>,
    },
    Undo {
        replica: usize,
    },
    Redo {
        replica: usize,
    },
    /// One replica takes in what another holds: its whole history, or only what is new.
    Sync {
        from: usize,
        to: usize,
        whole: bool,
    },
    /// A replica looks at an earlier revision and comes back.
    Visit {
        replica: usize,
        back: usize,
    },
}

/// Short text with characters of every width, and the one the history marks blocks with.
fn text() -> impl Strategy<Value = String> {
    let chars = prop_oneof![Just('a'), Just('b'), Just(' '), Just('é'), Just('字'), Just('🙂'), Just('\n')];
    vec(chars, 0..4).prop_map(String::from_iter)
}

fn edit() -> impl Strategy<Value = Edit> {
    let n = || 0..64usize;
    // Few enough ids that two replicas sometimes pick the same one.
    let id = || 0..24usize;
    let mark = prop_oneof![Just(Mark::Bold), Just(Mark::Italic), Just(Mark::Other("x:y%".into()))];
    let link = prop_oneof![Just(None), Just(Some("https://example.com/a".to_owned()))];
    prop_oneof![
        6 => (n(), n(), text()).prop_map(|(block, at, text)| Edit::Type { block, at, text }),
        3 => (n(), n(), 0..4usize).prop_map(|(block, at, len)| Edit::Delete { block, at, len }),
        2 => (n(), n(), 0..6usize, mark, any::<bool>())
            .prop_map(|(block, at, len, mark, on)| Edit::Mark { block, at, len, mark, on }),
        1 => (n(), n(), 0..6usize, link).prop_map(|(block, at, len, link)| Edit::Link { block, at, len, link }),
        3 => (n(), n(), id()).prop_map(|(block, at, id)| Edit::Split { block, at, id }),
        3 => n().prop_map(|block| Edit::Join { block }),
        1 => (n(), n()).prop_map(|(block, kind)| Edit::Kind { block, kind }),
        2 => (proptest::option::of(n()), 0..4usize, n(), id(), text())
            .prop_map(|(parent, index, kind, id, text)| Edit::Insert { parent, index, kind, id, text }),
        2 => n().prop_map(|block| Edit::Remove { block }),
    ]
}

fn step() -> impl Strategy<Value = Step> {
    let replica = || 0..REPLICAS;
    prop_oneof![
        8 => (replica(), vec(edit(), 1..3)).prop_map(|(replica, edits)| Step::Edit { replica, edits }),
        1 => replica().prop_map(|replica| Step::Undo { replica }),
        1 => replica().prop_map(|replica| Step::Redo { replica }),
        3 => (replica(), replica(), any::<bool>()).prop_map(|(from, to, whole)| Step::Sync { from, to, whole }),
        1 => (replica(), 0..64usize).prop_map(|(replica, back)| Step::Visit { replica, back }),
    ]
}

/// A document with every shape of block, formatted text and a nested container.
fn fixture() -> Document {
    let mut marked = Block::new("m", BlockKind::Paragraph);
    marked.body = Body::Text(vec![Run::plain("plain "), Run::marked("bold", [Mark::Bold]), Run::plain(" tail")]);
    let mut quote = Block::new("q", BlockKind::Quote);
    quote.body = Body::Blocks(vec![
        Block::text("q1", BlockKind::Paragraph, "Quoted"),
        Block::new("q2", BlockKind::Callout { tone: Tone::Tip }),
    ]);
    let blocks = vec![
        Block::text("h", BlockKind::Heading { level: 1 }, "Title"),
        marked,
        quote,
        Block::new("d", BlockKind::Divider),
        Block::text("l", BlockKind::ListItem { list: ListKind::Bullet, depth: 0 }, "Item é字🙂"),
    ];
    Document { blocks, ..Document::default() }
}

fn kinds(shape: BodyShape) -> Vec<BlockKind> {
    match shape {
        BodyShape::Text => vec![
            BlockKind::Paragraph,
            BlockKind::Heading { level: 2 },
            BlockKind::ListItem { list: ListKind::Task { checked: true }, depth: 1 },
            BlockKind::Code { language: Some("rust".into()) },
        ],
        BodyShape::Blocks => vec![BlockKind::Quote, BlockKind::Callout { tone: Tone::Warn }],
        BodyShape::None => vec![BlockKind::Divider],
    }
}

/// The byte range of `len` characters of a block's text from character `at`, wrapped to fit.
fn range(block: &Block, at: usize, len: usize) -> Range<usize> {
    let text = block.plain_text();
    let mut starts: Vec<usize> = text.char_indices().map(|(at, _)| at).collect();
    starts.push(text.len());
    let start = at % starts.len();
    starts[start]..starts[(start + len).min(starts.len() - 1)]
}

/// The operation an edit comes to in `doc`. `None` when the document has nothing for it to act on.
fn op(doc: &Document, edit: &Edit) -> Option<Op> {
    let all: Vec<&Block> = doc.walk().collect();
    let texts: Vec<&Block> = all.iter().copied().filter(|block| block.kind.shape() == BodyShape::Text).collect();
    let pick = |from: &[&Block], n: usize| (!from.is_empty()).then(|| from[n % from.len()].clone());
    let new = |id: usize| BlockId::new(format!("n{id}"));
    Some(match edit {
        Edit::Type { block, at, text } => {
            let block = pick(&texts, *block)?;
            Op::InsertText { at: Pos { offset: range(&block, *at, 0).start, block: block.id }, text: text.clone() }
        }
        Edit::Delete { block, at, len } => {
            let block = pick(&texts, *block)?;
            Op::DeleteText { range: range(&block, *at, *len), block: block.id }
        }
        Edit::Mark { block, at, len, mark, on } => {
            let block = pick(&texts, *block)?;
            Op::SetMark { range: range(&block, *at, *len), block: block.id, mark: mark.clone(), on: *on }
        }
        Edit::Link { block, at, len, link } => {
            let block = pick(&texts, *block)?;
            Op::SetLink { range: range(&block, *at, *len), block: block.id, link: link.clone() }
        }
        Edit::Split { block, at, id } => {
            let block = pick(&texts, *block)?;
            Op::Split { at: Pos { offset: range(&block, *at, 0).start, block: block.id }, new_id: new(*id) }
        }
        Edit::Join { block } => Op::Join { block: pick(&texts, *block)?.id },
        Edit::Kind { block, kind } => {
            let block = pick(&all, *block)?;
            let kinds = kinds(block.kind.shape());
            Op::SetKind { block: block.id, kind: kinds[kind % kinds.len()].clone() }
        }
        Edit::Insert { parent, index, kind, id, text } => {
            let kinds: Vec<BlockKind> = [BodyShape::Text, BodyShape::Blocks, BodyShape::None].map(kinds).concat();
            let mut block = Block::new(new(*id), kinds[kind % kinds.len()].clone());
            if let Body::Text(runs) = &mut block.body {
                runs.push(Run::marked(text.as_str(), [Mark::Italic]));
            }
            let containers: Vec<&Block> =
                all.iter().copied().filter(|block| block.kind.shape() == BodyShape::Blocks).collect();
            let parent = parent.and_then(|n| pick(&containers, n));
            let siblings = parent.as_ref().map_or(doc.blocks.len(), |parent| parent.body.blocks().len());
            Op::InsertBlock { parent: parent.map(|parent| parent.id), index: index % (siblings + 1), block }
        }
        Edit::Remove { block } => Op::RemoveBlock { block: pick(&all, *block)?.id },
    })
}

/// Every block's id and how deep it sits, in document order.
fn outline(doc: &Document) -> Vec<(BlockId, usize)> {
    fn walk(blocks: &[Block], depth: usize, out: &mut Vec<(BlockId, usize)>) {
        for block in blocks {
            out.push((block.id.clone(), depth));
            walk(block.body.blocks(), depth + 1, out);
        }
    }
    let mut out = Vec::new();
    walk(&doc.blocks, 0, &mut out);
    out
}

/// What layout would get wrong by trusting `change`: the blocks that differ without being named, and
/// `"structure"` when blocks moved without it saying so.
fn missed(before: &Document, after: &Document, change: &Change) -> Vec<String> {
    let own = |block: &Block| (block.kind.clone(), block.body.runs().to_vec(), block.extra.clone());
    let ids: BTreeSet<BlockId> = before.walk().chain(after.walk()).map(|block| block.id.clone()).collect();
    let differs = |id: &&BlockId| before.block(id).map(own) != after.block(id).map(own);
    let mut missed: Vec<String> =
        ids.iter().filter(differs).filter(|id| !change.blocks.contains(id)).map(BlockId::to_string).collect();
    if outline(before) != outline(after) && !change.structure {
        missed.push("structure".into());
    }
    missed
}

/// The document as a replica that only ever read this one's history sees it.
fn reread(model: &Model) -> Document {
    Model::load(&model.save(), ReplicaId::from(99)).expect("a saved history loads").doc().clone()
}

fn unique_ids(doc: &Document) -> bool {
    let mut ids = BTreeSet::new();
    doc.walk().all(|block| ids.insert(&block.id))
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

    #[test]
    fn replicas_converge_however_their_edits_interleave(steps in vec(step(), 1..48)) {
        let first = Model::new(fixture(), ReplicaId::from(1));
        // One copy starts from the same file, the other from the first copy's history.
        let third = first.fork(ReplicaId::from(3));
        let mut replicas = [first, Model::new(fixture(), ReplicaId::from(2)), third];
        let mut seen = vec![Vec::new(); REPLICAS];
        // Each replica's last edit, as the documents either side of it and whether it has been undone.
        let mut last: Vec<Option<(Document, Document, bool)>> = vec![None; REPLICAS];

        for step in steps {
            let touched = match step {
                Step::Edit { replica, .. } | Step::Undo { replica } | Step::Redo { replica } => replica,
                Step::Visit { replica, .. } => replica,
                Step::Sync { to, .. } => to,
            };
            let before = replicas[touched].doc().clone();
            let change = match step {
                Step::Edit { replica, edits } => {
                    let ops: Vec<Op> = edits.iter().filter_map(|edit| op(&before, edit)).collect();
                    let applied = replicas[replica].apply(ops);
                    let after = replicas[replica].doc().clone();
                    let reported = applied.as_ref().is_ok_and(|change| *change != Change::default());
                    prop_assert_eq!(reported, after != before, "an edit reports a change exactly when it made one");
                    seen[replica].push((replicas[replica].revision(), after.clone()));
                    if after != before {
                        last[replica] = Some((before.clone(), after, false));
                    }
                    applied.ok()
                }
                Step::Undo { replica } => {
                    let undone = replicas[replica].undo();
                    last[replica] = match last[replica].take() {
                        Some((before, after, false)) => {
                            prop_assert_eq!(replicas[replica].doc(), &before, "undo did not take the last edit back");
                            Some((before, after, true))
                        }
                        _ => None,
                    };
                    undone
                }
                Step::Redo { replica } => {
                    let redone = replicas[replica].redo();
                    last[replica] = match last[replica].take() {
                        Some((before, after, true)) => {
                            prop_assert_eq!(replicas[replica].doc(), &after, "redo did not make the edit again");
                            Some((before, after, false))
                        }
                        Some(edited) => {
                            prop_assert!(redone.is_none(), "an edit left something to redo");
                            Some(edited)
                        }
                        None => None,
                    };
                    redone
                }
                Step::Sync { from, to, whole } => {
                    last[to] = None;
                    let history = match whole {
                        true => replicas[from].save(),
                        false => replicas[from].updates_since(&replicas[to].revision()),
                    };
                    Some(replicas[to].merge(&history).expect("a replica reads another's history"))
                }
                Step::Visit { replica, back } => {
                    last[replica] = None;
                    let model = &mut replicas[replica];
                    if let Some((revision, shown)) = seen[replica].get(back % seen[replica].len().max(1)) {
                        model.checkout(revision).expect("a replica knows its own revisions");
                        prop_assert_eq!(model.doc(), shown, "an old revision reads differently than it did");
                        prop_assert!(model.is_latest() || model.apply([]) == Err(OpError::ReadOnly));
                    }
                    Some(model.checkout_latest())
                }
            };

            let model = &replicas[touched];
            if let Some(change) = change {
                prop_assert_eq!(missed(&before, model.doc(), &change), Vec::<String>::new());
            }
            prop_assert_eq!(model.doc(), &reread(model), "the document drifted from its history");
            prop_assert!(unique_ids(model.doc()), "two blocks share an id");
        }

        for to in 0..REPLICAS {
            for from in (0..REPLICAS).filter(|from| *from != to) {
                let history = replicas[from].save();
                replicas[to].merge(&history).expect("a replica reads another's history");
            }
        }
        let [first, second, third] = &replicas;
        prop_assert_eq!(first.doc(), second.doc());
        prop_assert_eq!(first.doc(), third.doc());
        prop_assert_eq!(first.compare(&third.save()), Ok(Some(Ordering::Equal)));
        let stored = Document::from_json(&first.doc().to_json());
        prop_assert_eq!(stored.as_ref(), Ok(first.doc()), "the merged document cannot be stored");
    }
}
