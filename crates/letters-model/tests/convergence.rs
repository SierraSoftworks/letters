//! Replicas that have seen the same edits show the same document and style, whatever the edits and their
//! order.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::ops::Range;

use letters_model::{
    Align, Block, BlockId, BlockKind, Body, BodyShape, Change, Conflict, Document, ListKind, Mark, Model, Op, OpError,
    Page, Pos, Properties, PropertiesFormat, ReplicaId, Run, Style, Tone,
};
use proptest::collection::vec;
use proptest::prelude::*;
use proptest::test_runner::RngSeed;

const REPLICAS: usize = 3;

/// The cases a run draws. A plain run draws the same ones every time, so that a fault in a rare
/// interleaving fails the change that made it, and no change after. `PROPTEST_CASES=1500` draws that many
/// afresh, which is how new faults are looked for. One that fails is kept beside this file and run first.
fn cases(cases: u32) -> ProptestConfig {
    match std::env::var_os("PROPTEST_CASES") {
        Some(_) => ProptestConfig::default(),
        None => ProptestConfig { cases, rng_seed: RngSeed::Fixed(1_800_000_000), ..ProptestConfig::default() },
    }
}

/// An edit by rough place. [`op`] fits it to whatever the document holds when its turn comes.
#[derive(Clone, Debug)]
enum Edit {
    Type { block: usize, at: usize, text: String },
    Delete { block: usize, at: usize, len: usize },
    Mark { block: usize, at: usize, len: usize, mark: Mark, on: bool },
    Link { block: usize, at: usize, len: usize, link: Option<String> },
    Lang { block: usize, at: usize, len: usize, lang: usize },
    Split { block: usize, at: usize, id: usize },
    Join { block: usize },
    Kind { block: usize, kind: usize },
    Props { block: usize, page_break: bool, lang: usize },
    Insert { parent: Option<usize>, index: usize, kind: usize, id: usize, text: String },
    Remove { block: usize },
    Style { element: usize, field: usize, value: usize },
    Page { field: usize, value: usize },
    Language { language: usize },
    Word { word: usize, add: bool },
    Properties { properties: usize },
}

/// No language, or one of two.
fn lang(n: usize) -> Option<String> {
    [None, Some("fr"), Some("de-CH")][n % 3].map(str::to_owned)
}

/// Everything a replica shows.
type Shown = (Document, Style);

fn shown(model: &Model) -> Shown {
    (model.doc().clone(), model.style().clone())
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
    /// Another tool edits a replica's file, knowing nothing of its history.
    Rewrite {
        replica: usize,
        edits: Vec<Edit>,
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
    let mark = prop_oneof![
        Just(Mark::Bold),
        Just(Mark::Italic),
        Just(Mark::Math),
        Just(Mark::Other("x:y%".into())),
        Just(Mark::Ref("q1".into())),
        Just(Mark::Ref("gone".into())),
    ];
    let link = prop_oneof![Just(None), Just(Some("https://example.com/a".to_owned()))];
    prop_oneof![
        6 => (n(), n(), text()).prop_map(|(block, at, text)| Edit::Type { block, at, text }),
        3 => (n(), n(), 0..4usize).prop_map(|(block, at, len)| Edit::Delete { block, at, len }),
        2 => (n(), n(), 0..6usize, mark, any::<bool>())
            .prop_map(|(block, at, len, mark, on)| Edit::Mark { block, at, len, mark, on }),
        1 => (n(), n(), 0..6usize, link).prop_map(|(block, at, len, link)| Edit::Link { block, at, len, link }),
        1 => (n(), n(), 0..6usize, n()).prop_map(|(block, at, len, lang)| Edit::Lang { block, at, len, lang }),
        1 => (n(), any::<bool>(), n()).prop_map(|(block, page_break, lang)| Edit::Props { block, page_break, lang }),
        1 => n().prop_map(|properties| Edit::Properties { properties }),
        3 => (n(), n(), id()).prop_map(|(block, at, id)| Edit::Split { block, at, id }),
        3 => n().prop_map(|block| Edit::Join { block }),
        1 => (n(), n()).prop_map(|(block, kind)| Edit::Kind { block, kind }),
        2 => (proptest::option::of(n()), 0..4usize, n(), id(), text())
            .prop_map(|(parent, index, kind, id, text)| Edit::Insert { parent, index, kind, id, text }),
        2 => n().prop_map(|block| Edit::Remove { block }),
        1 => (n(), n(), n()).prop_map(|(element, field, value)| Edit::Style { element, field, value }),
        1 => (n(), n()).prop_map(|(field, value)| Edit::Page { field, value }),
        1 => n().prop_map(|language| Edit::Language { language }),
        1 => (n(), any::<bool>()).prop_map(|(word, add)| Edit::Word { word, add }),
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
        1 => (replica(), vec(edit(), 1..6)).prop_map(|(replica, edits)| Step::Rewrite { replica, edits }),
    ]
}

fn picture(asset: Option<&str>, src: Option<&str>, width: f32) -> BlockKind {
    let (numbered, asset, src) = (src.is_some(), asset.map(str::to_owned), src.map(str::to_owned));
    BlockKind::Figure { asset, src, alt: "é".into(), width, height: 90.5, numbered }
}

fn holding(id: &str, kind: BlockKind, parts: Vec<Block>) -> Block {
    Block { body: Body::Blocks(parts), ..Block::new(id, kind) }
}

/// A document with every shape of block, formatted text, nested containers and blocks made of parts.
fn fixture() -> Document {
    let mut marked = Block::new("m", BlockKind::Paragraph);
    marked.body = Body::Text(vec![
        Run::plain("plain "),
        Run::marked("bold", [Mark::Bold]),
        Run { lang: lang(1), ..Run::plain(" tail") },
    ]);
    let mut quote = Block::new("q", BlockKind::Quote);
    quote.body = Body::Blocks(vec![
        Block::text("q1", BlockKind::Paragraph, "Quoted"),
        Block::new("q2", BlockKind::Callout { tone: Tone::Tip }),
    ]);
    let blocks = vec![
        Block { page_break: true, lang: lang(2), ..Block::text("h", BlockKind::Heading { level: 1 }, "Title") },
        marked,
        quote,
        Block::new("d", BlockKind::Divider),
        Block::text("l", BlockKind::ListItem { list: ListKind::Bullet, depth: 0 }, "Item é字🙂"),
        holding(
            "g",
            BlockKind::Gallery { numbered: false },
            vec![
                Block::text("g1", picture(Some("assets/a.png"), None, 120.0), "One"),
                Block::text("g2", picture(None, Some("https://example.com/b.png"), 64.0), ""),
                Block::text("g3", BlockKind::Caption, "Both"),
            ],
        ),
        holding(
            "t",
            BlockKind::Table { columns: vec![Align::Start, Align::End], header: true },
            vec![
                holding(
                    "r1",
                    BlockKind::TableRow,
                    ["c1", "c2"].map(|id| Block::text(id, BlockKind::TableCell, id)).into(),
                ),
                holding(
                    "r2",
                    BlockKind::TableRow,
                    ["c3", "c4"].map(|id| Block::text(id, BlockKind::TableCell, id)).into(),
                ),
            ],
        ),
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
            BlockKind::Formula,
            BlockKind::Caption,
            BlockKind::TableCell,
            picture(Some("assets/c.jpg"), None, 200.0),
            picture(None, Some("https://example.com/d.png"), 33.25),
            // No file would hold this one, so no edit may make it.
            picture(None, None, 10.0),
        ],
        BodyShape::Blocks => vec![
            BlockKind::Quote,
            BlockKind::Callout { tone: Tone::Warn },
            BlockKind::Gallery { numbered: true },
            BlockKind::Table { columns: vec![Align::Center], header: false },
            BlockKind::TableRow,
        ],
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

/// The operation an edit comes to in what a replica shows. `None` when there is nothing for it to act on.
fn op((doc, style): &Shown, edit: &Edit) -> Option<Op> {
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
        Edit::Lang { block, at, len, lang: tag } => {
            let block = pick(&texts, *block)?;
            Op::SetLang { range: range(&block, *at, *len), block: block.id, lang: lang(*tag) }
        }
        Edit::Props { block, page_break, lang: tag } => {
            Op::SetBlockProps { block: pick(&all, *block)?.id, page_break: *page_break, lang: lang(*tag) }
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
            let containers: Vec<&Block> =
                all.iter().copied().filter(|block| block.kind.shape() == BodyShape::Blocks).collect();
            let parent = parent.and_then(|n| pick(&containers, n));
            // Mostly a kind its parent takes, so that tables and galleries fill up. Now and then any kind.
            let mut kinds: Vec<BlockKind> = [BodyShape::Text, BodyShape::Blocks, BodyShape::None].map(kinds).concat();
            if kind % 8 > 0 {
                kinds.retain(|kind| parent.as_ref().is_none_or(|parent| parent.kind.admits(kind)));
            }
            let mut block = Block::new(new(*id), kinds[kind % kinds.len()].clone());
            if let Body::Text(runs) = &mut block.body {
                runs.push(Run::marked(text.as_str(), [Mark::Italic]));
            }
            let siblings = parent.as_ref().map_or(doc.blocks.len(), |parent| parent.body.blocks().len());
            Op::InsertBlock { parent: parent.map(|parent| parent.id), index: index % (siblings + 1), block }
        }
        Edit::Remove { block } => Op::RemoveBlock { block: pick(&all, *block)?.id },
        Edit::Style { element, field, value } => {
            let element = ["body", "heading1", "aside"][element % 3];
            let mut text = style.types.get(element).cloned().unwrap_or_default();
            match field % 3 {
                // The last size is one no style may hold.
                0 => text.size = [9.0, 12.5, 30.0, 0.0][value % 4],
                1 => text.weight = [400, 700][value % 2],
                _ => text.color = ["#000000", "#ff0000"][value % 2].into(),
            }
            Op::SetStyle { element: element.into(), style: text }
        }
        // Either is a page on its own. A narrow page with a wide margin, as a merge can give, is none.
        Edit::Page { field, value } => Op::SetPage {
            page: match field % 2 {
                0 => Page { width: [300.0, 595.0][value % 2], ..style.page.clone() },
                _ => Page { margin_left: [71.0, 250.0][value % 2], ..style.page.clone() },
            },
        },
        Edit::Language { language } => Op::SetLanguage { language: ["en", "fr", "de"][language % 3].into() },
        Edit::Properties { properties: n } => {
            let source = ["title: a\n", "title: b\n"][n % 2].to_owned();
            let properties = Properties { format: PropertiesFormat::Toml, source, extra: Default::default() };
            Op::SetProperties { properties: (n % 3 > 0).then_some(properties) }
        }
        Edit::Word { word, add } => {
            let word = ["mot", "word", "字\u{1e}x"][word % 3].to_owned();
            if *add { Op::AddWord { word } } else { Op::RemoveWord { word } }
        }
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
/// whatever else differs without it saying so.
fn missed((before, style): &Shown, (after, restyled): &Shown, change: &Change) -> Vec<String> {
    let own = |block: &Block| Block { body: Body::Text(block.body.runs().to_vec()), ..block.clone() };
    let ids: BTreeSet<BlockId> = before.walk().chain(after.walk()).map(|block| block.id.clone()).collect();
    let differs = |id: &&BlockId| before.block(id).map(own) != after.block(id).map(own);
    let mut missed: Vec<String> =
        ids.iter().filter(differs).filter(|id| !change.blocks.contains(id)).map(BlockId::to_string).collect();
    let fields = |doc: &Document| (doc.language.clone(), doc.dictionary.clone(), doc.properties.clone());
    let unsaid = [
        ("structure", outline(before) != outline(after) && !change.structure),
        ("style", style != restyled && !change.style),
        ("document", fields(before) != fields(after) && !change.document),
    ];
    missed.extend(unsaid.iter().filter(|(_, unsaid)| *unsaid).map(|(what, _)| (*what).to_owned()));
    missed
}

/// What a replica that only ever read this one's history shows.
fn reread(model: &Model) -> Shown {
    shown(&Model::load(&model.save(), ReplicaId::from(99)).expect("a saved history loads"))
}

fn unique_ids(doc: &Document) -> bool {
    let mut ids = BTreeSet::new();
    doc.walk().all(|block| ids.insert(&block.id))
}

/// A sentence whose words are parted by spaces, with marks of every kind standing against them.
const SENTENCE: &str = "One two, three (four) five. Six!";
const WORDS: usize = 6;

/// What a writer does to one word of [`SENTENCE`], by place in the word: its ends are places too.
#[derive(Clone, Debug)]
enum Touch {
    Type {
        at: usize,
        text: String,
    },
    Cut {
        at: usize,
        len: usize,
    },
    /// Cuts, and types where the cut began.
    Swap {
        at: usize,
        len: usize,
        text: String,
    },
    /// Types at the start of the word or at its end, with a space or a mark between that and the word.
    Beside {
        end: bool,
        text: String,
    },
}

fn touch() -> impl Strategy<Value = Touch> {
    prop_oneof![
        (0..8usize, text()).prop_map(|(at, text)| Touch::Type { at, text }),
        (0..8usize, 1..4usize).prop_map(|(at, len)| Touch::Cut { at, len }),
        (0..8usize, 1..4usize, text()).prop_map(|(at, len, text)| Touch::Swap { at, len, text }),
    ]
}

fn beside() -> impl Strategy<Value = Touch> {
    let parting = || prop_oneof![Just(' '), Just('.'), Just(','), Just('!')];
    (any::<bool>(), parting(), text(), parting())
        .prop_map(|(end, before, text, after)| Touch::Beside { end, text: format!("{before}{text}{after}") })
}

/// The edit to the sentence in the paragraph `s` that a touch of its `word` is. With it, where the edit
/// starts in the sentence, and where it types.
fn touched(word: usize, touch: &Touch) -> (Vec<Op>, usize, Option<usize>) {
    let start: usize = SENTENCE.split(' ').take(word).map(|word| word.len() + 1).sum();
    let word = SENTENCE.split(' ').nth(word).unwrap_or_default();
    let offset =
        |at: usize| start + word.char_indices().map(|(at, _)| at).chain([word.len()]).nth(at).unwrap_or(word.len());
    let typed = |at: usize, text: &str| Op::InsertText { at: Pos::new("s", at), text: text.to_owned() };
    let cut = |at: usize, len: usize| {
        let from = at.min(word.chars().count() - 1);
        (Op::DeleteText { block: BlockId::new("s"), range: offset(from)..offset(from + len) }, offset(from))
    };
    match touch {
        Touch::Type { at, text } => (vec![typed(offset(*at), text)], offset(*at), Some(offset(*at))),
        Touch::Cut { at, len } => (vec![cut(*at, *len).0], cut(*at, *len).1, None),
        Touch::Swap { at, len, text } => {
            let (cut, from) = cut(*at, *len);
            (vec![cut, typed(from, text)], from, Some(from))
        }
        Touch::Beside { end, text } => {
            let at = if *end { start + word.len() } else { start };
            (vec![typed(at, text)], at, Some(at))
        }
    }
}

proptest! {
    #![proptest_config(cases(512))]

    /// Settling only ever replaces text where two writers changed the same word. Text typed beside a word
    /// is no change to the word, and neither is anything done to another word: all of it stays.
    #[test]
    fn what_one_writer_alone_touched_survives_a_settling(
        (word, hers) in (0..WORDS, touch()),
        (away, his) in prop_oneof![(0..WORDS, beside()), (1..WORDS, touch())],
        her_clock_is_ahead in any::<bool>(),
    ) {
        let doc = Document { blocks: vec![Block::text("s", BlockKind::Paragraph, SENTENCE)], ..Document::default() };
        let made = [touched(word, &hers), touched((word + away) % WORDS, &his)];
        // One writer makes both edits, the one further on first, so that neither moves the other's place.
        let mut alone = Model::new(doc.clone(), ReplicaId::from(9));
        for nth in if made[0].1 >= made[1].1 { [0, 1] } else { [1, 0] } {
            alone.apply(made[nth].0.clone()).expect("an edit to the sentence fits it");
        }
        // Text that both typed at one place stands there in either order.
        let one_place = made[0].2.is_some() && made[0].2 == made[1].2;
        let read = |model: &Model| {
            let mut text: Vec<char> = model.doc().blocks[0].plain_text().chars().collect();
            if one_place {
                text.sort_unstable();
            }
            text
        };

        let mut writers = [Model::new(doc.clone(), ReplicaId::from(1)), Model::new(doc, ReplicaId::from(2))];
        for (nth, writer) in writers.iter_mut().enumerate() {
            writer.set_time(1_800_000_000 + if (nth == 0) == her_clock_is_ahead { 3_600 } else { 0 });
            writer.apply(made[nth].0.clone()).expect("an edit to the sentence fits it");
        }
        let stood = writers.each_ref().map(Model::revision);
        let held = writers.each_ref().map(Model::save);
        for (nth, writer) in writers.iter_mut().enumerate() {
            writer.merge(&held[1 - nth]).expect("a replica reads another's history");
            let (_, found) = writer.settle(&stood[nth], &stood[1 - nth]).expect("a replica holds what it merged");
            prop_assert_eq!(found, Vec::new(), "edits that share no word conflict");
            prop_assert_eq!(read(writer), read(&alone), "what one writer alone touched was lost or misplaced");
        }
        prop_assert_eq!(writers[0].doc(), writers[1].doc());
    }
}

proptest! {
    #![proptest_config(cases(128))]

    #[test]
    fn replicas_converge_however_their_edits_interleave(steps in vec(step(), 1..48)) {
        let first = Model::new(fixture(), ReplicaId::from(1));
        // One copy starts from the same file, the other from the first copy's history.
        let third = first.fork(ReplicaId::from(3));
        let mut replicas = [first, Model::new(fixture(), ReplicaId::from(2)), third];
        // Two of the writers give their names, and the third writes unnamed.
        replicas[0].set_author("Ana");
        replicas[1].set_author("Ben");
        let mut seen = vec![Vec::new(); REPLICAS];
        // Each replica's last edit, as the documents either side of it and whether it has been undone.
        let mut last: Vec<Option<(Shown, Shown, bool)>> = vec![None; REPLICAS];

        for (nth, step) in steps.into_iter().enumerate() {
            let touched = match step {
                Step::Edit { replica, .. } | Step::Undo { replica } | Step::Redo { replica } => replica,
                Step::Visit { replica, .. } | Step::Rewrite { replica, .. } => replica,
                Step::Sync { to, .. } => to,
            };
            let before = shown(&replicas[touched]);
            // Each writer's clock runs at its own pace, and one has none.
            replicas[touched].set_time(if touched == 2 { 0 } else { 1_800_000_000 + (nth * (40 + touched)) as u64 });
            let change = match step {
                Step::Edit { replica, edits } => {
                    let ops: Vec<Op> = edits.iter().filter_map(|edit| op(&before, edit)).collect();
                    // Edits made together can take each other back, as a join and a split in the same place.
                    let alone = ops.len() < 2;
                    let applied = replicas[replica].apply(ops);
                    let after = shown(&replicas[replica]);
                    let reported = applied.as_ref().is_ok_and(|change| *change != Change::default());
                    prop_assert!(reported || after == before, "an edit that changed the document did not report it");
                    prop_assert!(!alone || reported == (after != before), "an edit that changed nothing reported a change");
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
                            prop_assert_eq!(&shown(&replicas[replica]), &before, "undo did not take the last edit back");
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
                            prop_assert_eq!(&shown(&replicas[replica]), &after, "redo did not make the edit again");
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
                    let (ours, theirs) = (replicas[to].revision(), replicas[from].revision());
                    let held = [before.0.clone(), replicas[from].doc().clone()];
                    let mine = replicas[to].save();
                    let mut merged = replicas[to].merge(&history).expect("a replica reads another's history");
                    let apart = replicas[to].conflicts(&ours, &theirs).expect("a replica holds what it merged");
                    let mirror = replicas[to].conflicts(&theirs, &ours).expect("a replica holds what it merged");
                    let turned = |conflict: &Conflict| {
                        let [first, second] = conflict.sides.clone();
                        Conflict { sides: [second, first], later: 1 - conflict.later, ..conflict.clone() }
                    };
                    prop_assert_eq!(&apart, &mirror.iter().map(turned).collect::<Vec<_>>(), "the sides tell apart");
                    for conflict in &apart {
                        let versions = conflict.sides.each_ref().map(|side| side.block.as_ref());
                        let then = held.each_ref().map(|doc| doc.block(&conflict.block));
                        prop_assert_eq!(versions, then, "a conflict shows a block as neither side left it");
                    }
                    let (settled, found) = replicas[to].settle(&ours, &theirs).expect("a replica holds what it merged");
                    prop_assert_eq!(&found, &apart, "settling found other conflicts than the search");
                    for conflict in &apart {
                        let now = replicas[to].block(&conflict.block).map(Block::plain_text);
                        let later = conflict.sides[conflict.later].block.as_ref().map(Block::plain_text);
                        // A block that one side joined to it stays joined. Joined by the earlier side, its
                        // text follows. Joined by the later side, what the earlier made of it is in between.
                        let (first, last) = (conflict.later, 1 - conflict.later);
                        let mut joined = [first, last].map(|side| {
                            let mut after = held[side].walk().skip_while(|block| block.id != conflict.block);
                            after.nth(1).is_some_and(|next| held[1 - side].block(&next.id).is_none())
                        });
                        // The earlier side may not hold the block to tell by. Then any block of its own
                        // that the later side has made away with may be the one.
                        let away = |block: &Block| held[first].block(&block.id).is_none() && replicas[to].block(&block.id).is_none();
                        joined[1] |= held[last].block(&conflict.block).is_none() && held[last].walk().any(away);
                        // A later side that both joined a block to it and cut it in two holds some of the
                        // joined block's text, and what the earlier side made of that is in between as well.
                        let whole = match (&now, &later) {
                            (Some(now), Some(later)) if joined[1] => {
                                let mut rest = now.chars();
                                later.chars().all(|c| rest.any(|next| next == c))
                            }
                            (Some(now), Some(later)) if joined[0] => now.starts_with(later.as_str()),
                            _ => now == later,
                        };
                        prop_assert!(whole, "a settled block reads {:?}, and the later side left it {:?}", now, later);
                    }
                    // The other device meets the same two revisions, and settles them with no word from this one.
                    let mut other = Model::load(&replicas[from].save(), ReplicaId::from(97)).expect("a saved history loads");
                    other.merge(&mine).expect("a replica reads another's history");
                    other.settle(&theirs, &ours).expect("a replica holds what it merged");
                    prop_assert_eq!(shown(&other), shown(&replicas[to]), "two devices settled a conflict differently");
                    let same = other.compare(&replicas[to].save());
                    prop_assert_eq!(same, Ok(Some(Ordering::Equal)), "two devices settled a conflict by different changes");
                    merged.blocks.extend(settled.blocks);
                    merged.structure |= settled.structure;
                    Some(merged)
                }
                Step::Visit { replica, back } => {
                    last[replica] = None;
                    let model = &mut replicas[replica];
                    if let Some((revision, then)) = seen[replica].get(back % seen[replica].len().max(1)) {
                        model.checkout(revision).expect("a replica knows its own revisions");
                        prop_assert_eq!(&shown(model), then, "an old revision reads differently than it did");
                        prop_assert!(model.is_latest() || model.apply([]) == Err(OpError::ReadOnly));
                    }
                    Some(model.checkout_latest())
                }
                Step::Rewrite { replica, edits } => {
                    last[replica] = None;
                    let mut outside = Model::new(before.0.clone(), ReplicaId::from(98)).with_style(before.1.clone());
                    for edit in &edits {
                        let _ = outside.apply(op(&shown(&outside), edit));
                    }
                    let model = &mut replicas[replica];
                    let rewritten = model.rewrite(outside.doc().clone());
                    let restyled = model.rewrite_style(outside.style().clone());
                    prop_assert_eq!(shown(model), shown(&outside), "a rewrite did not give what it was told");
                    seen[replica].push((model.revision(), shown(model)));
                    rewritten.ok().zip(restyled.ok()).map(|(change, restyled)| Change { style: restyled.style, ..change })
                }
            };

            let model = &replicas[touched];
            if let Some(change) = change {
                prop_assert_eq!(missed(&before, &shown(model), &change), Vec::<String>::new());
            }
            prop_assert_eq!(shown(model), reread(model), "the document drifted from its history");
            prop_assert!(unique_ids(model.doc()), "two blocks share an id");
            let found = |block: &Block| model.block(&block.id).is_some_and(|found| std::ptr::eq(found, block));
            prop_assert!(model.doc().walk().all(found), "a block is not found by its id");
        }

        for to in 0..REPLICAS {
            for from in (0..REPLICAS).filter(|from| *from != to) {
                let history = replicas[from].save();
                replicas[to].merge(&history).expect("a replica reads another's history");
            }
        }
        let [first, second, third] = &replicas;
        prop_assert_eq!(first.authors(), third.authors());
        prop_assert_eq!(shown(first), shown(second));
        prop_assert_eq!(shown(first), shown(third));
        prop_assert_eq!(first.compare(&third.save()), Ok(Some(Ordering::Equal)));
        let stored = Document::from_json(&first.doc().to_json());
        prop_assert_eq!(stored.as_ref(), Ok(first.doc()), "the merged document cannot be stored");
        let stored = Style::from_json(&first.style().to_json());
        prop_assert_eq!(stored.as_ref(), Ok(first.style()), "the merged style cannot be stored");
    }
}
