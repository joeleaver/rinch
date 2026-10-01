//! `BatchStep::map` (#1200, from the second review of #1224): a batch mapped
//! over other changes makes the document its edits make as separate mapped
//! steps, except where the step's docs say otherwise — pinned here as such.
use rinch_editor_core::*;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn para(s: &Schema, t: &str) -> Node {
    let c = if t.is_empty() {
        Fragment::empty()
    } else {
        Fragment::from_node(s.text(t).unwrap())
    };
    s.branch("paragraph", c).unwrap()
}

fn doc(s: &Schema, rng: &mut Rng) -> Node {
    let mut blocks = Vec::new();
    for _ in 0..2 + rng.below(4) {
        match rng.below(4) {
            0 | 3 => blocks.push(para(s, &"ab".repeat(rng.below(4)))),
            1 => blocks.push(
                s.create_node(
                    "heading",
                    Attrs::from_iter([("level", AttrValue::Int(1))]),
                    Fragment::from_node(s.text("hh").unwrap()),
                )
                .unwrap(),
            ),
            _ => {
                let item = s
                    .create_node(
                        "list_item",
                        Attrs::new(),
                        Fragment::from_node(para(s, "li")),
                    )
                    .unwrap();
                blocks.push(
                    s.create_node("bullet_list", Attrs::new(), Fragment::from_node(item))
                        .unwrap(),
                )
            }
        }
    }
    s.branch("doc", Fragment::from_children(blocks)).unwrap()
}

fn content(s: &Schema, rng: &mut Rng, doc: &Node, open_ok: bool) -> Slice {
    match rng.below(6) {
        0 => Slice::from_fragment(Fragment::from_node(s.text("zz").unwrap())),
        1 => Slice::from_fragment(Fragment::from_node(para(s, "p"))),
        2 if open_ok => {
            let size = doc.content_size();
            let x = rng.below(size + 1);
            let y = (x + rng.below(8)).min(size);
            doc.slice(x, y).unwrap_or_else(|_| Slice::empty())
        }
        _ => Slice::empty(),
    }
}

/// Random disjoint edits; slices may be open (`doc.slice`) when `open_ok`.
fn edits(s: &Schema, rng: &mut Rng, doc: &Node, open_ok: bool) -> Vec<BatchEdit> {
    let size = doc.content_size();
    let mut pts: Vec<usize> = (0..2 * (1 + rng.below(4)))
        .map(|_| rng.below(size + 1))
        .collect();
    pts.sort_unstable();
    let mut out = Vec::new();
    for p in pts.chunks(2) {
        let (from, mut to) = (p[0], p[1]);
        if rng.below(3) == 0 {
            to = from;
        }
        if rng.below(4) == 0 {
            out.push(BatchEdit::SetAttr {
                pos: from,
                attr: "level".into(),
                value: Some(AttrValue::Int(2 + rng.below(3) as i64)),
            });
        } else {
            out.push(BatchEdit::Replace {
                from,
                to,
                slice: content(s, rng, doc, open_ok),
            });
        }
    }
    out
}

/// A step with its sort key: position, attribute-or-not, edit index.
type Keyed = ((usize, bool, usize), Box<dyn Step>);

fn as_steps(step_edits: &[BatchEdit]) -> Vec<Keyed> {
    let mut v: Vec<Keyed> = Vec::new();
    for (i, e) in step_edits.iter().enumerate() {
        match e {
            BatchEdit::Replace { from, to, slice } => v.push((
                (*from, false, i),
                Box::new(ReplaceStep::new(*from, *to, slice.clone())),
            )),
            BatchEdit::SetAttr { pos, attr, value } => v.push((
                (*pos, true, 0),
                Box::new(SetNodeAttrStep {
                    pos: *pos,
                    attr: attr.clone(),
                    value: value.clone(),
                }),
            )),
        }
    }
    v
}

fn merged_at_new(es: &[BatchEdit], b: &BatchStep) -> bool {
    let reps = es
        .iter()
        .filter(|e| matches!(e, BatchEdit::Replace { .. }))
        .count();
    let mut ap: Vec<usize> = es
        .iter()
        .filter_map(|e| {
            if let BatchEdit::SetAttr { pos, .. } = e {
                Some(*pos)
            } else {
                None
            }
        })
        .collect();
    ap.sort();
    ap.dedup();
    b.len() < reps + ap.len()
}

#[derive(Default, Debug)]
struct Tally {
    merged_class: usize,
    cases: usize,
    equal: usize,
    differ: usize,
    batch_fail_seq_ok: usize,
    none_seq_changes: usize,
    inv_bad: usize,
    seq_fail: usize,
    both_fail: usize,
}

fn run(open_ok: bool, seed: u64, n: usize) -> Tally {
    let s = Schema::starter_kit();
    let mut rng = Rng(seed);
    let mut t = Tally::default();
    let mut shown = 0;
    for case in 0..n {
        let d0 = doc(&s, &mut rng);
        let es = edits(&s, &mut rng, &d0, open_ok);
        let Ok(batch) = BatchStep::new(es.clone()) else {
            continue;
        };
        // Apply only when the batch applies on its own and equals the sequence there.
        if batch.apply(&d0).is_err() {
            continue;
        }
        // The "other" change: 1..3 real replace steps.
        let mut d1 = d0.clone();
        let mut over = Mapping::new();
        for _ in 0..1 + rng.below(3) {
            let size = d1.content_size();
            let x = rng.below(size + 1);
            let y = (x + rng.below(7)).min(size);
            let sl = match rng.below(4) {
                0 => Slice::from_fragment(Fragment::from_node(s.text("q").unwrap())),
                _ => Slice::empty(),
            };
            let st = ReplaceStep::new(x, y, sl);
            if let Ok(n) = st.apply(&d1) {
                d1 = n;
                over.append_map(st.get_map());
            }
        }
        t.cases += 1;
        // Sequence: the batch's normalised edits? Use the given edits (before merge) as separate steps.
        let mut keyed = as_steps(&es);
        keyed.sort_by_key(|k| std::cmp::Reverse(k.0));
        let mut seq = Ok(d1.clone());
        for (_, st) in keyed {
            if let Some(m) = st.map(&over) {
                seq = seq.and_then(|d| m.apply(&d));
            }
        }
        match (batch.map(&over), seq) {
            (Some(m), Ok(w)) => match m.apply(&d1) {
                Ok(got) if got == w => {
                    t.equal += 1;
                    let back = m.invert(&d1).apply(&got);
                    if back.as_ref().ok() != Some(&d1) {
                        t.inv_bad += 1;
                    }
                }
                Ok(got) if merged_at_new(&es, &batch) => {
                    let _ = got;
                    t.merged_class += 1;
                }
                Ok(got) => {
                    t.differ += 1;
                    if shown < 4 {
                        shown += 1;
                        eprintln!(
                            "DIFFER case {case}: {es:?}\n over {over:?}\n mapped {m:?}\n d1 {d1:?}\n got {got:?}\n want {w:?}"
                        );
                    }
                }
                Err(e) => {
                    t.batch_fail_seq_ok += 1;
                    if shown < 4 {
                        shown += 1;
                        eprintln!(
                            "BATCHFAIL case {case}: {e:?} {es:?}\n over {over:?}\n mapped {m:?}\n d1 {d1:?}\n want {w:?}"
                        );
                    }
                }
            },
            (None, Ok(w)) => {
                if w != d1 {
                    t.none_seq_changes += 1;
                    eprintln!("NONE-CHANGES case {case}: {es:?}\n over {over:?}");
                    if shown < 4 {
                        shown += 1;
                        eprintln!("NONE case {case}: {es:?}\n over {over:?}");
                    }
                }
            }
            (Some(m), Err(_)) => {
                if m.apply(&d1).is_ok() {
                    t.seq_fail += 1
                } else {
                    t.both_fail += 1
                }
            }
            (None, Err(_)) => t.both_fail += 1,
        }
    }
    t
}

#[test]
fn closed_slices_map_like_a_sequence() {
    let seed = std::env::var("R_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0xabcdef12345u64);
    let n = std::env::var("R_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30000);
    let t = run(false, seed, n);
    eprintln!("closed: {t:?}");
    assert!(t.equal > 1000);
    assert_eq!(
        (t.differ, t.batch_fail_seq_ok, t.none_seq_changes, t.inv_bad),
        (0, 0, 0, 0),
        "{t:?}"
    );
}

fn text_para(s: &Schema, t: &str) -> Node {
    para(s, t)
}

/// Three kept edits meeting at one point after mapping: two inserts (mergeable,
/// left for `new`) then a replace that folds onto the second → `new` refuses.
#[test]
fn three_edits_meeting_at_one_point_keep_all() {
    let s = Schema::starter_kit();
    let d0 = s
        .branch("doc", Fragment::from_node(text_para(&s, "abcdef")))
        .unwrap();
    let t = |x: &str| Slice::from_fragment(Fragment::from_node(s.text(x).unwrap()));
    let es = vec![
        BatchEdit::Replace {
            from: 2,
            to: 2,
            slice: t("X"),
        },
        BatchEdit::Replace {
            from: 4,
            to: 4,
            slice: t("Y"),
        },
        BatchEdit::Replace {
            from: 6,
            to: 7,
            slice: t("Z"),
        },
    ];
    let batch = BatchStep::new(es.clone()).unwrap();
    let o1 = ReplaceStep::new(2, 4, Slice::empty());
    let d_mid = o1.apply(&d0).unwrap();
    let o2 = ReplaceStep::new(2, 4, Slice::empty());
    let d1 = o2.apply(&d_mid).unwrap();
    let mut over = Mapping::new();
    over.append_map(o1.get_map());
    over.append_map(o2.get_map());
    let mut keyed = as_steps(&es);
    keyed.sort_by_key(|k| std::cmp::Reverse(k.0));
    let mut want = d1.clone();
    for (_, st) in keyed {
        if let Some(m) = st.map(&over) {
            want = m.apply(&want).unwrap();
        }
    }
    let m = batch
        .map(&over)
        .expect("the batch mapped to nothing; separately all three survive");
    assert_eq!(m.apply(&d1).unwrap(), want);
}

/// An attribute change that a mapping puts inside a kept replace (a StepMap
/// with two adjacent ranges): the guard drops the change, not the step.
#[test]
fn attr_guard_is_reachable() {
    let s = Schema::starter_kit();
    let es = vec![
        BatchEdit::Replace {
            from: 2,
            to: 5,
            slice: Slice::from_fragment(Fragment::from_node(s.text("x").unwrap())),
        },
        BatchEdit::delete(8, 11),
        BatchEdit::SetAttr {
            pos: 6,
            attr: "a".into(),
            value: None,
        },
    ];
    let batch = BatchStep::new(es).unwrap();
    let mut over = Mapping::new();
    over.append_map(StepMap::new(vec![2, 4, 2, 6, 4, 0]));
    let del = ReplaceStep::new(8, 11, Slice::empty()).map(&over);
    assert!(del.is_some(), "positive control");
    assert!(batch.map(&over).is_some(), "the batch mapped to nothing");
}

/// Two deletions that meet are one range from construction, and map as one
/// `ReplaceStep` of that range would: over a concurrent replace that
/// straddles their shared boundary, the range deletes the concurrent insertion
/// that two separate deletions would keep. Documented, pinned as such.
#[test]
fn merged_deletions_map_as_one_range() {
    let s = Schema::starter_kit();
    let d0 = s
        .branch("doc", Fragment::from_node(text_para(&s, "abcdefgh")))
        .unwrap();
    let es = vec![BatchEdit::delete(3, 5), BatchEdit::delete(5, 7)];
    let batch = BatchStep::new(es.clone()).unwrap();
    let o = ReplaceStep::new(
        4,
        6,
        Slice::from_fragment(Fragment::from_node(s.text("q").unwrap())),
    );
    let d1 = o.apply(&d0).unwrap();
    let mut over = Mapping::new();
    over.append_map(o.get_map());
    let mut keyed = as_steps(&es);
    keyed.sort_by_key(|k| std::cmp::Reverse(k.0));
    let mut want = d1.clone();
    for (_, st) in keyed {
        if let Some(m) = st.map(&over) {
            want = m.apply(&want).unwrap();
        }
    }
    let got = batch.map(&over).unwrap().apply(&d1).unwrap();
    assert_eq!(
        want,
        s.branch("doc", Fragment::from_node(text_para(&s, "abqgh")))
            .unwrap()
    );
    assert_eq!(
        got,
        s.branch("doc", Fragment::from_node(text_para(&s, "abgh")))
            .unwrap(),
        "d1 {d1:?}"
    );
    let one = ReplaceStep::new(3, 7, Slice::empty()).map(&over).unwrap();
    assert_eq!(
        got,
        one.apply(&d1).unwrap(),
        "as one ReplaceStep of the range"
    );
}

/// An open slice and a closed insert that a mapping brings end to end: the
/// later edit is dropped and its content is LOST, where separate steps keep
/// it. Documented (no table command builds an open slice), pinned as such.
#[test]
fn an_open_pair_loses_the_later_edit() {
    let s = Schema::starter_kit();
    let d0 = s
        .branch(
            "doc",
            Fragment::from_children(vec![text_para(&s, "abcd"), text_para(&s, "efgh")]),
        )
        .unwrap();
    // open slice: "b</p><p>e" style — the slice from 2..9 of d0
    let open = d0.slice(2, 9).unwrap();
    eprintln!("open slice {open:?}");
    let es = vec![
        BatchEdit::Replace {
            from: 2,
            to: 3,
            slice: open,
        },
        BatchEdit::Replace {
            from: 5,
            to: 5,
            slice: Slice::from_fragment(Fragment::from_node(s.text("Z").unwrap())),
        },
    ];
    let batch = BatchStep::new(es.clone()).unwrap();
    let o = ReplaceStep::new(3, 5, Slice::empty());
    let d1 = o.apply(&d0).unwrap();
    let mut over = Mapping::new();
    over.append_map(o.get_map());
    let mut keyed = as_steps(&es);
    keyed.sort_by_key(|k| std::cmp::Reverse(k.0));
    let mut want = d1.clone();
    for (_, st) in keyed {
        if let Some(m) = st.map(&over) {
            want = m.apply(&want).unwrap();
        }
    }
    let m = batch.map(&over).unwrap();
    let got = m.apply(&d1).unwrap();
    assert_ne!(got, want, "positive control: separate steps keep Z");
    // Exactly the first edit, mapped on its own.
    let first = ReplaceStep::new(2, 3, d0.slice(2, 9).unwrap())
        .map(&over)
        .unwrap();
    assert_eq!(got, first.apply(&d1).unwrap(), "d1 {d1:?}\n mapped {m:?}");
}

/// The last of two changes of one attribute wins (sequence order).
#[test]
fn the_last_duplicate_attr_change_wins() {
    let s = Schema::starter_kit();
    let h = s
        .create_node(
            "heading",
            Attrs::from_iter([("level", AttrValue::Int(1))]),
            Fragment::from_node(s.text("h").unwrap()),
        )
        .unwrap();
    let d = s.branch("doc", Fragment::from_node(h)).unwrap();
    let b = BatchStep::new(vec![
        BatchEdit::set_attr(0, "level", AttrValue::Int(2)),
        BatchEdit::set_attr(0, "level", AttrValue::Int(3)),
    ])
    .unwrap();
    assert_eq!(
        b.apply(&d).unwrap().child(0).attrs().get_int("level"),
        Some(3)
    );
}

/// A fold keeps the earlier slice first.
#[test]
fn a_fold_keeps_order() {
    let s = Schema::starter_kit();
    let d0 = s
        .branch("doc", Fragment::from_node(para(&s, "abcdef")))
        .unwrap();
    let t = |x: &str| Slice::from_fragment(Fragment::from_node(s.text(x).unwrap()));
    let b = BatchStep::new(vec![
        BatchEdit::Replace {
            from: 2,
            to: 3,
            slice: t("X"),
        },
        BatchEdit::Replace {
            from: 5,
            to: 6,
            slice: t("Y"),
        },
    ])
    .unwrap();
    let o = ReplaceStep::new(3, 5, Slice::empty());
    let d1 = o.apply(&d0).unwrap();
    let mut over = Mapping::new();
    over.append_map(o.get_map());
    let got = b.map(&over).unwrap().apply(&d1).unwrap();
    assert_eq!(
        got,
        s.branch("doc", Fragment::from_node(para(&s, "aXYf")))
            .unwrap()
    );
}
