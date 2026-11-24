//! The skipping pass against the pass that derives every step (issue #748).
//!
//! Two documents get the same history — two steppers each, then a seeded
//! sequence of insertions, removals, reorders and moves between the two
//! steppers — one with [`DERIVE_EVERY_STEP`] set for every change. After
//! each change the two trees must be the same tree: every attribute this
//! file writes, every text node, every child, in order.

use super::*;
use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::traits::DomDocument;
use std::cell::RefCell;
use std::collections::HashMap;

/// Every attribute a stepper, a step or an icon box can carry.
const ATTRS: &[&str] = &[
    "class",
    "style",
    "tabindex",
    "role",
    "data-rid",
    "data-active",
    "data-icon-for",
    DISABLED_ATTR,
    POSITION_ATTR,
    OWN_CLICKABLE_ATTR,
    CLICK_HANDLER_ID_ATTR,
    ICON_HAS_ATTR,
    ICON_LIVE_ATTR,
    STATE_ATTR,
    STEP_ATTR,
    STEP_DERIVED_ATTR,
    SETTLED_BY_ATTR,
];

/// Attributes whose value is an id minted from a thread-wide counter, which
/// the two documents therefore draw different numbers from. Compared by
/// order of first appearance instead.
const MINTED: &[&str] = &["data-rid", CLICK_HANDLER_ID_ATTR, SETTLED_BY_ATTR];

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn one_in(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }
}

struct World {
    _doc: Rc<RefCell<MockDomDocument>>,
    scope: RenderScope,
    root: NodeHandle,
    containers: Vec<NodeHandle>,
    every_step: bool,
}

impl World {
    fn new(every_step: bool, rng: &mut Rng) -> Self {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone() as Rc<RefCell<dyn DomDocument>>, body);
        let root = scope.create_element("div");
        let mut world = Self {
            _doc: doc,
            scope,
            root,
            containers: Vec::new(),
            every_step,
        };
        world.with_mode(|world| {
            for stepper in [
                Stepper {
                    active: 2,
                    on_step_click: Some(ValueCallback::new(|_: u32| {})),
                    completed_icon: Some(TablerIcon::CircleCheck),
                    ..Default::default()
                },
                Stepper {
                    active: 4,
                    allow_next_steps_select: rng.one_in(2),
                    progress_icon: Some(TablerIcon::Bell),
                    ..Default::default()
                },
            ] {
                let mut children: Vec<NodeHandle> = (0..3).map(|_| world.step(rng)).collect();
                // A wrapper between the stepper and some of its steps, as a
                // `for` loop puts one, and a completed block mid-list.
                let wrapper = world.scope.create_element("div");
                for _ in 0..2 {
                    let step = world.step(rng);
                    wrapper.append_child(&step);
                }
                children.insert(1, wrapper);
                children.insert(3, StepperCompleted.render(&mut world.scope, &[]));
                let rendered = stepper.render(&mut world.scope, &children);
                let container = rendered
                    .children()
                    .into_iter()
                    .find(|c| has_class(c, "rinch-stepper__steps"))
                    .expect("steps row");
                world.root.append_child(&rendered);
                world.containers.push(container);
            }
        });
        world
    }

    fn with_mode<R>(&mut self, run: impl FnOnce(&mut Self) -> R) -> R {
        DERIVE_EVERY_STEP.with(|flag| flag.set(self.every_step));
        let out = run(self);
        DERIVE_EVERY_STEP.with(|flag| flag.set(false));
        out
    }

    fn step(&mut self, rng: &mut Rng) -> NodeHandle {
        let icon = |rng: &mut Rng, icon| rng.one_in(3).then_some(icon);
        StepperStep {
            icon: icon(rng, TablerIcon::Home),
            completed_icon: icon(rng, TablerIcon::Check),
            progress_icon: icon(rng, TablerIcon::Clock),
            allow_step_click: rng.one_in(5),
            loading: rng.one_in(9),
            disabled: rng.one_in(7),
            state: match rng.below(8) {
                0 => "completed".into(),
                1 => "progress".into(),
                2 => "inactive".into(),
                _ => String::new(),
            },
            step: rng.one_in(6).then(|| rng.below(20) as u32),
            ..Default::default()
        }
        .render(&mut self.scope, &[])
    }

    /// Every node a step can be put into: the two steps rows and the
    /// wrappers inside them.
    fn parents(&self) -> Vec<NodeHandle> {
        let mut out = Vec::new();
        for container in &self.containers {
            out.push(container.clone());
            for child in container.children() {
                if !has_class(&child, STEP_CLASS) && !has_class(&child, COMPLETED_CLASS) {
                    out.push(child);
                }
            }
        }
        out
    }

    fn steps(&self) -> Vec<NodeHandle> {
        self.containers
            .iter()
            .flat_map(|c| collect_steps(c).0)
            .collect()
    }

    /// Put `step` into parent `p` in front of that parent's child `at`, or
    /// at its end.
    fn place(&self, step: &NodeHandle, p: usize, at: usize) {
        let parents = self.parents();
        let parent = &parents[p % parents.len()];
        let siblings: Vec<NodeHandle> = parent
            .children()
            .into_iter()
            .filter(|c| c.node_id() != step.node_id())
            .collect();
        match siblings.get(at % (siblings.len() + 1)) {
            Some(reference) => parent.insert_before(step, reference),
            None => parent.append_child(step),
        }
    }

    fn apply(&mut self, op: Op, rng: &mut Rng) {
        self.with_mode(|world| match op {
            Op::Insert { p, at } => {
                let step = world.step(rng);
                world.place(&step, p, at);
            }
            Op::Remove { which } => {
                let steps = world.steps();
                if !steps.is_empty() {
                    steps[which % steps.len()].remove();
                }
            }
            Op::Move { which, p, at } => {
                let steps = world.steps();
                if !steps.is_empty() {
                    world.place(&steps[which % steps.len()], p, at);
                }
            }
            Op::Replace { which } => {
                // What a `for` reconcile does to a changed row.
                let steps = world.steps();
                if !steps.is_empty() {
                    let old = &steps[which % steps.len()];
                    let new = world.step(rng);
                    old.insert_after(&new);
                    old.discard();
                }
            }
        });
    }

    fn dump(&self) -> String {
        let mut out = String::new();
        let mut minted = HashMap::new();
        dump(&self.root, 0, &mut minted, &mut out);
        out
    }
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Insert { p: usize, at: usize },
    Remove { which: usize },
    Move { which: usize, p: usize, at: usize },
    Replace { which: usize },
}

fn dump(
    node: &NodeHandle,
    depth: usize,
    minted: &mut HashMap<(&'static str, String), usize>,
    out: &mut String,
) {
    out.push_str(&"  ".repeat(depth));
    match node.tag_name() {
        Some(tag) => {
            out.push_str(&tag);
            for name in ATTRS {
                let Some(value) = node.get_attribute(name) else {
                    continue;
                };
                let value = if MINTED.contains(name) {
                    let next = minted.len();
                    format!("#{}", minted.entry((name, value)).or_insert(next))
                } else {
                    value
                };
                out.push_str(&format!(" {name}={value:?}"));
            }
            out.push('\n');
            for child in node.children() {
                dump(&child, depth + 1, minted, out);
            }
        }
        None => {
            out.push_str(&format!("{:?}\n", node.text_content()));
        }
    }
}

fn run(seed: u64, ops: usize) {
    // One generator per world, seeded alike, so the steps the two build are
    // the same steps; a third draws the history.
    let mut rng_a = Rng(seed);
    let mut rng_b = Rng(seed);
    let mut history = Rng(seed ^ 0x9e3779b97f4a7c15);
    let mut skipping = World::new(false, &mut rng_a);
    let mut reference = World::new(true, &mut rng_b);
    assert_eq!(skipping.dump(), reference.dump(), "seed {seed}: at render");

    for n in 0..ops {
        let op = match history.below(10) {
            0..=3 => Op::Insert {
                p: history.below(64),
                at: history.below(64),
            },
            4..=5 => Op::Remove {
                which: history.below(64),
            },
            6..=8 => Op::Move {
                which: history.below(64),
                p: history.below(64),
                at: history.below(64),
            },
            _ => Op::Replace {
                which: history.below(64),
            },
        };
        skipping.apply(op, &mut rng_a);
        reference.apply(op, &mut rng_b);
        let (got, want) = (skipping.dump(), reference.dump());
        assert!(
            got == want,
            "seed {seed}, change {n} ({op:?}): the skipping pass left a \
             different tree from the pass that derives every step.\n\
             --- skipping ---\n{got}\n--- every step ---\n{want}"
        );
    }
}

#[test]
fn the_skipping_pass_leaves_the_tree_the_whole_pass_leaves() {
    let seeds: u64 = std::env::var("RINCH_TWIN_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    for seed in 1..=seeds {
        run(seed, 60);
    }
}

#[test]
fn the_reference_pass_really_derives_every_step() {
    // The oracle's own control: with the flag set, an append to a settled
    // stepper reads far more than three attributes per step it leaves alone.
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone() as Rc<RefCell<dyn DomDocument>>, body);
    let steps: Vec<NodeHandle> = (0..20)
        .map(|_| StepperStep::default().render(&mut scope, &[]))
        .collect();
    let rendered = Stepper {
        active: 9,
        ..Default::default()
    }
    .render(&mut scope, &steps);
    let container = rendered.children().into_iter().next().expect("steps row");

    let mut reads = Vec::new();
    for every_step in [false, true] {
        let step = StepperStep::default().render(&mut scope, &[]);
        DERIVE_EVERY_STEP.with(|flag| flag.set(every_step));
        let before = doc.borrow().__op_counts();
        container.append_child(&step);
        reads.push(doc.borrow().__op_counts().since(before).attribute_reads);
        DERIVE_EVERY_STEP.with(|flag| flag.set(false));
    }
    assert!(
        reads[1] > reads[0] + 20 * 10,
        "skipping {} reads, every step {} reads",
        reads[0],
        reads[1]
    );
}
