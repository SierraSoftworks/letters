//! Several writers on one document: whatever they do and however their edits reach each other, they end
//! on the same document, and each step tells layout the truth about what it changed.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use letters_model::{Block, BlockId, Body, Change, Conflict, Document, Model, OpError, Style};

use crate::edits::{Shown, op, ops, reread, sample, shown, unique_ids};
use crate::input::Script;

const REPLICAS: usize = 3;
const STEPS: usize = 48;

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

pub fn run(data: &[u8]) {
    let mut script = Script::new(data);
    let first = Model::new(sample(), 1.into());
    // One copy starts from the same file, the other from the first copy's history.
    let third = first.fork(3.into());
    let mut replicas = [first, Model::new(sample(), 2.into()), third];
    // Two of the writers give their names, and the third writes unnamed.
    replicas[0].set_author("Ana");
    replicas[1].set_author("Ben");
    let mut seen = vec![Vec::new(); REPLICAS];
    // Each replica's last edit, as the documents either side of it and whether it has been undone.
    let mut last: Vec<Option<(Shown, Shown, bool)>> = vec![None; REPLICAS];

    for nth in 0..STEPS {
        if script.done() {
            break;
        }
        let (step, touched) = (script.pick(16), script.pick(REPLICAS));
        let before = shown(&replicas[touched]);
        // Each writer's clock runs at its own pace, and one has none.
        replicas[touched].set_time(if touched == 2 { 0 } else { 1_800_000_000 + (nth * (40 + touched)) as u64 });
        let change = match step {
            0..=7 => {
                let ops = ops(&mut script, &before, 2);
                // Edits made together can take each other back, as a join and a split in the same place.
                let alone = ops.len() < 2;
                let applied = replicas[touched].apply(ops);
                let after = shown(&replicas[touched]);
                let reported = applied.as_ref().is_ok_and(|change| *change != Change::default());
                assert!(reported || after == before, "an edit that changed the document did not report it");
                assert!(!alone || reported == (after != before), "an edit that changed nothing reported a change");
                seen[touched].push((replicas[touched].revision(), after.clone()));
                if after != before {
                    last[touched] = Some((before.clone(), after, false));
                }
                applied.ok()
            }
            8 => {
                let undone = replicas[touched].undo();
                last[touched] = match last[touched].take() {
                    Some((before, after, false)) => {
                        assert_eq!(shown(&replicas[touched]), before, "undo did not take the last edit back");
                        Some((before, after, true))
                    }
                    _ => None,
                };
                undone
            }
            9 => {
                let redone = replicas[touched].redo();
                last[touched] = match last[touched].take() {
                    Some((before, after, true)) => {
                        assert_eq!(shown(&replicas[touched]), after, "redo did not make the edit again");
                        Some((before, after, false))
                    }
                    Some(edited) => {
                        assert!(redone.is_none(), "an edit left something to redo");
                        Some(edited)
                    }
                    None => None,
                };
                redone
            }
            // One replica takes in what another holds: its whole history, or only what is new.
            10..=12 => {
                let (to, from) = (touched, script.pick(REPLICAS));
                last[to] = None;
                let history = match script.flag() {
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
                assert_eq!(apart, mirror.iter().map(turned).collect::<Vec<_>>(), "the sides of a conflict tell apart");
                for conflict in &apart {
                    let versions = conflict.sides.each_ref().map(|side| side.block.as_ref());
                    let then = held.each_ref().map(|doc| doc.block(&conflict.block));
                    assert_eq!(versions, then, "a conflict shows a block as neither side left it");
                }
                let (settled, found) = replicas[to].settle(&ours, &theirs).expect("a replica holds what it merged");
                assert_eq!(found, apart, "settling found other conflicts than the search");
                for conflict in &apart {
                    let now = replicas[to].block(&conflict.block).map(Block::plain_text);
                    let later = conflict.sides[conflict.later].block.as_ref().map(Block::plain_text);
                    // A block that one side joined to it stays joined. Joined by the earlier side, its text
                    // follows. Joined by the later side, what the earlier made of it is in between.
                    let (first, last) = (conflict.later, 1 - conflict.later);
                    let mut joined = [first, last].map(|side| {
                        let mut after = held[side].walk().skip_while(|block| block.id != conflict.block);
                        after.nth(1).is_some_and(|next| held[1 - side].block(&next.id).is_none())
                    });
                    // The earlier side may not hold the block to tell by. Then any block of its own that the
                    // later side has made away with may be the one.
                    let away = |block: &Block| {
                        held[first].block(&block.id).is_none() && replicas[to].block(&block.id).is_none()
                    };
                    joined[1] |= held[last].block(&conflict.block).is_none() && held[last].walk().any(away);
                    let whole = match (&now, &later) {
                        (Some(now), Some(later)) if joined[0] => now.starts_with(later.as_str()),
                        (Some(now), Some(later)) if joined[1] => {
                            let mut rest = now.chars();
                            later.chars().all(|c| rest.any(|next| next == c))
                        }
                        _ => now == later,
                    };
                    assert!(whole, "a settled block reads {now:?}, and the later side left it {later:?}");
                }
                if !apart.is_empty() {
                    // The other device meets the same two revisions, and settles them with no word from this one.
                    let mut other = Model::load(&replicas[from].save(), 97.into()).expect("a saved history loads");
                    other.merge(&mine).expect("a replica reads another's history");
                    other.settle(&theirs, &ours).expect("a replica holds what it merged");
                    assert_eq!(shown(&other), shown(&replicas[to]), "two devices settled a conflict differently");
                    let same = other.compare(&replicas[to].save());
                    assert_eq!(same, Ok(Some(Ordering::Equal)), "two devices settled a conflict by different changes");
                }
                merged.blocks.extend(settled.blocks);
                merged.structure |= settled.structure;
                Some(merged)
            }
            // A replica looks at an earlier revision and comes back.
            13 => {
                last[touched] = None;
                let model = &mut replicas[touched];
                let back = script.pick(seen[touched].len());
                if let Some((revision, then)) = seen[touched].get(back) {
                    model.checkout(revision).expect("a replica knows its own revisions");
                    assert_eq!(&shown(model), then, "an old revision reads differently than it did");
                    assert!(model.is_latest() || model.apply([]) == Err(OpError::ReadOnly));
                }
                Some(model.checkout_latest())
            }
            // Another tool edits a replica's file, knowing nothing of its history.
            _ => {
                last[touched] = None;
                let mut outside = Model::new(before.0.clone(), 98.into()).with_style(before.1.clone());
                for _ in 0..=script.pick(5) {
                    let _ = outside.apply(op(&mut script, &shown(&outside)));
                }
                let model = &mut replicas[touched];
                let rewritten = model.rewrite(outside.doc().clone());
                let restyled = model.rewrite_style(outside.style().clone());
                assert_eq!(shown(model), shown(&outside), "a rewrite did not give what it was told");
                seen[touched].push((model.revision(), shown(model)));
                rewritten.ok().zip(restyled.ok()).map(|(change, restyled)| Change { style: restyled.style, ..change })
            }
        };

        let model = &replicas[touched];
        if let Some(change) = change {
            assert_eq!(missed(&before, &shown(model), &change), Vec::<String>::new(), "a change left this unsaid");
        }
        assert!(unique_ids(model.doc()), "two blocks share an id");
        let found = |block: &Block| model.block(&block.id).is_some_and(|found| std::ptr::eq(found, block));
        assert!(model.doc().walk().all(found), "a block is not found by its id");
    }

    for model in &replicas {
        assert_eq!(shown(model), reread(model), "the document drifted from its history");
    }
    for to in 0..REPLICAS {
        for from in (0..REPLICAS).filter(|from| *from != to) {
            let history = replicas[from].save();
            replicas[to].merge(&history).expect("a replica reads another's history");
        }
    }
    let [first, second, third] = &replicas;
    assert_eq!(shown(first), reread(first), "the merged document drifted from its history");
    assert_eq!(first.authors(), third.authors());
    assert_eq!(shown(first), shown(second), "two writers ended on different documents");
    assert_eq!(shown(first), shown(third), "two writers ended on different documents");
    assert_eq!(first.compare(&third.save()), Ok(Some(Ordering::Equal)));
    let stored = Document::from_json(&first.doc().to_json());
    assert_eq!(stored.as_ref(), Ok(first.doc()), "the merged document cannot be stored");
    let stored = Style::from_json(&first.style().to_json());
    assert_eq!(stored.as_ref(), Ok(first.style()), "the merged style cannot be stored");
}

/// Scripts of a few steps each, so that the fuzzer starts from every kind of step.
pub fn seeds() -> Vec<Vec<u8>> {
    let edit = [0, 0, 1, 0, 2, 1, 3];
    let sync = [10, 1, 0, 1, 11, 2, 1, 0];
    let mut seeds: Vec<Vec<u8>> =
        (0..16u8).map(|step| [&edit[..], &[step, 1, 2, 3, 4, 5, 6, 7], &sync].concat()).collect();
    seeds.push((0..=255).collect());
    seeds
}
