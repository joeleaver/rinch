//! The step maps of a change the history did not record, reconstructed by
//! diffing the range it replaced.

use crate::model::{Fragment, Node};
use crate::state::Transaction;
use crate::transform::step_map::StepMap;
use crate::transform::steps::ReplaceStep;

/// One position's worth of a document, for [`unrecorded_maps`]: what a step map
/// cares about, and nothing it does not (marks, a block's type or attributes).
#[derive(PartialEq)]
enum Tok<'a> {
    Open,
    Close,
    Char(char),
    Leaf(&'a str),
}

/// The tokens of positions `from..to` of `content` (which starts at `base`).
fn tokens<'a>(content: &'a Fragment, base: usize, from: usize, to: usize, out: &mut Vec<Tok<'a>>) {
    let mut pos = base;
    for child in content.children() {
        let end = pos + child.node_size();
        if end <= from {
            pos = end;
            continue;
        }
        if pos >= to {
            break;
        }
        if let Some(text) = child.text() {
            let lo = from.saturating_sub(pos);
            let hi = (to - pos).min(end - pos);
            out.extend(text.chars().skip(lo).take(hi - lo).map(Tok::Char));
        } else if child.is_leaf() {
            out.push(Tok::Leaf(child.type_name()));
        } else {
            if pos >= from {
                out.push(Tok::Open);
            }
            tokens(child.content(), pos + 1, from, to, out);
            if end - 1 < to {
                out.push(Tok::Close);
            }
        }
        pos = end;
    }
}

/// The maps the history keeps for a change it does not record: each step's map,
/// narrowed for a replace to the positions that actually changed, and none for a
/// step that moves nothing (a mark, a block's type or attributes).
///
/// A collaboration peer's edit arrives as a replace of every block it changed
/// (`rinch-editor-collab`'s remote integration), and a position inside a replaced
/// range has nowhere to map: an undo step there is dropped. Narrowed to what
/// changed — the char a peer typed — a step of mine elsewhere in that paragraph
/// maps past it and still undoes. The narrowing compares positions, not nodes: a
/// char is the same char whatever its marks, and a block's open and close tokens
/// are the same whatever its type, so a peer's bold, heading or alignment moves
/// no position and keeps my edit undoable (review of #1889). Everything outside
/// the narrowed range is the same in both documents, so the map is exact for it.
pub(super) fn unrecorded_maps(tr: &Transaction) -> Vec<StepMap> {
    let docs = tr.docs();
    let steps = tr.steps();
    // Several replaces in one change (the collab integration replaces each changed
    // block, back to front): diffed as one range of the document before and after
    // all of them, so a block one step deletes and another inserts elsewhere (a
    // peer's move) is found as the move it is, not as blocks paired by place.
    if steps.len() > 1
        && steps
            .iter()
            .all(|s| s.as_any().downcast_ref::<ReplaceStep>().is_some())
    {
        let maps = tr.mapping().maps();
        let back = |i: usize, pos: usize, assoc: i32| {
            maps[..i]
                .iter()
                .rev()
                .fold(pos, |p, m| m.invert().map(p, assoc))
        };
        let (mut lo, mut hi) = (usize::MAX, 0);
        for (i, step) in steps.iter().enumerate() {
            let r = step.as_any().downcast_ref::<ReplaceStep>().unwrap();
            lo = lo.min(back(i, r.from, -1));
            hi = hi.max(back(i, r.to, 1));
        }
        let (new_lo, new_hi) = (tr.mapping().map(lo, -1), tr.mapping().map(hi, 1));
        if let Some(found) = narrowed(&docs[0], tr.doc(), lo, hi, new_lo.min(new_hi), new_hi, true)
        {
            return found.into_iter().collect();
        }
    }
    let mut out = Vec::with_capacity(steps.len());
    for (i, step) in steps.iter().enumerate() {
        let Some(r) = step.as_any().downcast_ref::<ReplaceStep>() else {
            let map = step.get_map();
            if !map.is_empty() {
                out.push(map);
            }
            continue;
        };
        let after = docs.get(i + 1).unwrap_or(tr.doc());
        let to_new = r.from + r.slice.size();
        if let Some(Some(map)) = narrowed(&docs[i], after, r.from, r.to, r.from, to_new, false) {
            out.push(map);
        }
    }
    out
}

/// The map of a change that turned positions `from..to` of `before` into
/// `new_from..new_to` of `after` (everything outside them the same), narrowed to
/// the tokens that differ: `Some(None)` when none do. With `exact`, `None` when
/// the difference is too large to diff run by run (the caller then maps step by
/// step); otherwise one range covers it.
fn narrowed(
    before: &Node,
    after: &Node,
    from: usize,
    to: usize,
    new_from: usize,
    new_to: usize,
    exact: bool,
) -> Option<Option<StepMap>> {
    let (mut a, mut b) = (Vec::new(), Vec::new());
    tokens(before.content(), 0, from, to, &mut a);
    tokens(after.content(), 0, new_from, new_to, &mut b);
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let room = a.len().min(b.len()) - prefix;
    let suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(room)
        .take_while(|(x, y)| x == y)
        .count();
    let (old, new) = (a.len() - prefix - suffix, b.len() - prefix - suffix);
    if old == 0 && new == 0 {
        return Some(None);
    }
    let start = from + prefix;
    let ranges = if old == 0 || new == 0 {
        vec![start, old, new]
    } else {
        match token_diff(&a[prefix..prefix + old], &b[prefix..prefix + new]) {
            Some(edits) => edits
                .into_iter()
                .flat_map(|(at, o, n)| [start + at, o, n])
                .collect(),
            None if exact => return None,
            None => vec![start, old, new],
        }
    };
    Some(Some(StepMap::new(ranges)))
}

/// The changes that turn `a` into `b`, as `(position in a, deleted, inserted)`
/// runs in order: a shortest edit script (Myers). Separate changes in one replace
/// (a peer's keystrokes at both ends of a paragraph, a block wrapped in a quote:
/// an open token before it and a close token after) stay separate, so a position
/// between them maps exactly. `None` when the two differ too much for that to be
/// worth it, and the caller keeps one range.
fn token_diff(a: &[Tok<'_>], b: &[Tok<'_>]) -> Option<Vec<(usize, usize, usize)>> {
    const MAX_EDITS: usize = 64;
    const MAX_LEN: usize = 8192;
    let (n, m) = (a.len() as isize, b.len() as isize);
    if a.len() + b.len() > MAX_LEN {
        return None;
    }
    let max = (a.len() + b.len()).min(MAX_EDITS) as isize;
    let off = max + 1;
    let idx = |k: isize| (k + off) as usize;
    let mut v = vec![0isize; 2 * off as usize + 1];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut found = false;
    'outer: for d in 0..=max {
        trace.push(v.clone());
        let mut k = -d;
        while k <= d {
            let mut x = if k == -d || (k != d && v[idx(k - 1)] < v[idx(k + 1)]) {
                v[idx(k + 1)]
            } else {
                v[idx(k - 1)] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[idx(k)] = x;
            if x >= n && y >= m {
                found = true;
                break 'outer;
            }
            k += 2;
        }
    }
    if !found {
        return None;
    }
    // Walk back: `true` for an op that consumes `a` (a delete), `false` for an
    // insert; equal runs are skipped.
    let (mut x, mut y) = (n, m);
    let mut ops: Vec<(isize, bool)> = Vec::new();
    for (d, v) in trace.iter().enumerate().rev() {
        let d = d as isize;
        let k = x - y;
        let prev_k = if k == -d || (k != d && v[idx(k - 1)] < v[idx(k + 1)]) {
            k + 1
        } else {
            k - 1
        };
        let prev_x = v[idx(prev_k)];
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
        }
        if d > 0 {
            if x == prev_x {
                ops.push((x, false));
            } else {
                ops.push((prev_x, true));
            }
        }
        x = prev_x;
        y = prev_y;
    }
    ops.reverse();
    // Group the ops that sit at one place in `a` (a delete run and the inserts
    // beside it) into one range.
    let mut out: Vec<(usize, usize, usize)> = Vec::new();
    let mut del_end = 0usize;
    for (at, del) in ops {
        let at = at as usize;
        match out.last_mut() {
            Some(last) if at <= del_end => {
                if del {
                    last.1 += 1;
                    del_end += 1;
                } else {
                    last.2 += 1;
                }
            }
            _ => {
                out.push((at, usize::from(del), usize::from(!del)));
                del_end = at + usize::from(del);
            }
        }
    }
    Some(out)
}
