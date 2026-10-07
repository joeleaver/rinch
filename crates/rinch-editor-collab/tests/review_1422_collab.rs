//! Review of #1422: a fitted delete (a `ReplaceAroundStep`, or a `ReplaceStep`
//! across depths) through the collab projection: the CRDT reads back as the
//! model, and a peer that integrates the delta holds the same document.

use std::rc::Rc;

use rinch_editor_collab::CollabSession;
use rinch_editor_core::serialize::slice_from_html;
use rinch_editor_core::{EditorState, Node, Pos, Schema, Selection, default_plugins};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn inline(rng: &mut Rng, out: &mut String, n: &mut usize) {
    for _ in 0..1 + rng.below(2) {
        *n += 1;
        let text = format!("{}{}", ["a", "bc", "d", "xy"][rng.below(4)], *n % 10);
        match rng.below(8) {
            0 => out.push_str(&format!("<strong>{text}</strong>")),
            1 => out.push_str(&format!("<a href=\"https://e.x/\">{text}</a>")),
            2 => out.push_str("<br>"),
            3 => out.push_str("<img src=\"https://e.x/i.png\" alt=\"i\">"),
            _ => out.push_str(&text),
        }
    }
}

fn blocks(rng: &mut Rng, depth: usize, out: &mut String, n: &mut usize) {
    for _ in 0..1 + rng.below(2) {
        let pick = if depth >= 3 {
            rng.below(4)
        } else {
            rng.below(10)
        };
        match pick {
            0 | 1 => {
                out.push_str("<p>");
                inline(rng, out, n);
                out.push_str("</p>");
            }
            2 => {
                out.push_str("<h2>");
                inline(rng, out, n);
                out.push_str("</h2>");
            }
            3 => out.push_str(if rng.below(2) == 0 {
                "<hr>"
            } else {
                "<pre>let x;\ny</pre>"
            }),
            4..=6 => {
                let tag = if rng.below(2) == 0 { "ul" } else { "ol" };
                out.push_str(&format!("<{tag}>"));
                for _ in 0..1 + rng.below(2) {
                    out.push_str("<li>");
                    blocks(rng, depth + 1, out, n);
                    out.push_str("</li>");
                }
                out.push_str(&format!("</{tag}>"));
            }
            _ => {
                out.push_str("<blockquote>");
                blocks(rng, depth + 1, out, n);
                out.push_str("</blockquote>");
            }
        }
    }
}

fn random_doc(schema: &Schema, rng: &mut Rng) -> Node {
    let mut html = String::new();
    let mut n = 0;
    blocks(rng, 0, &mut html, &mut n);
    let slice = slice_from_html(schema, &html).expect("generated html parses");
    schema.branch("doc", slice.content).expect("a valid doc")
}

#[test]
fn a_fitted_delete_projects_and_reaches_a_peer() {
    let seeds: u64 = std::env::var("RINCH_1422_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    let schema = Rc::new(Schema::starter_kit());
    let (mut fitted, mut around) = (0usize, 0usize);
    for seed in 1..=seeds {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let doc = random_doc(&schema, &mut rng);
        let size = doc.content_size();
        if size > 120 {
            continue;
        }
        let text: Vec<(usize, usize)> = (0..=size)
            .filter_map(|p| {
                let r = doc.resolve(Pos(p)).ok()?;
                r.parent().is_textblock().then_some((p, r.depth()))
            })
            .collect();
        if text.is_empty() {
            continue;
        }
        for _ in 0..40 {
            let (a, da) = text[rng.below(text.len())];
            let (b, db) = text[rng.below(text.len())];
            if a == b || da == db {
                continue;
            }
            let (from, to) = (a.min(b), a.max(b));
            let what = format!("seed {seed}: {from}..{to} of {doc:?}");
            let mut state = EditorState::create(schema.clone(), doc.clone(), default_plugins());
            let mut host = CollabSession::new(&state).unwrap_or_else(|e| panic!("{what}: {e}"));
            let mut guest = CollabSession::from_bytes(&host.snapshot()).unwrap();
            let guest_state = EditorState::create(
                schema.clone(),
                guest.projected_doc(&schema).unwrap(),
                default_plugins(),
            );
            assert_eq!(guest_state.doc, doc, "{what}: join");
            state.selection = Selection::text(Pos(from), Pos(to));
            let Some(after) = state.run("deleteSelection") else {
                panic!("{what}: deleteSelection did nothing");
            };
            fitted += 1;
            host.record_local(&schema, &doc, &after.doc)
                .unwrap_or_else(|e| panic!("{what}: record_local: {e}"));
            assert_eq!(
                host.projected_doc(&schema).unwrap(),
                after.doc,
                "{what}: the CRDT is not the model"
            );
            let delta = host.save_incremental().unwrap();
            let next = guest
                .integrate_incremental(&guest_state, &delta)
                .unwrap_or_else(|e| panic!("{what}: integrate: {e}"));
            let guest_doc = next.map_or(guest_state.doc.clone(), |s| s.doc);
            if after.doc != doc {
                around += 1;
            }
            assert_eq!(guest_doc, after.doc, "{what}: the peer differs");
        }
    }
    eprintln!("{fitted} deletes across depths, {around} changed the document");
    assert!(fitted > 300 && around > 300);
}
