//! The skipping pass against the pass that derives every step, over the
//! shapes [`differential_tests`] does not sample (from the review of PR
//! #1423): a stepper with every prop set at once, a nested stepper whose
//! steps move to and from the outer ones, a step removed and put back
//! later, a wrapper or completed block moved with its steps, and the same
//! step nodes handed to a new render with different props.
use super::*;
use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::traits::DomDocument;
use std::cell::RefCell;

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
    ICON_ALT_ATTR,
    STATE_ATTR,
    STEP_ATTR,
    STEP_DERIVED_ATTR,
];
const MINTED: &[&str] = &["data-rid", CLICK_HANDLER_ID_ATTR];

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

struct Props {
    active: u32,
    next: bool,
    click: bool,
    ci: bool,
    pi: bool,
}
impl Props {
    fn stepper(&self) -> Stepper {
        Stepper {
            active: self.active,
            allow_next_steps_select: self.next,
            on_step_click: self.click.then(|| ValueCallback::new(|_: u32| {})),
            completed_icon: self.ci.then_some(TablerIcon::CircleCheck),
            progress_icon: self.pi.then_some(TablerIcon::Bell),
            ..Default::default()
        }
    }
}

struct World {
    _doc: Rc<RefCell<MockDomDocument>>,
    scope: RenderScope,
    root: NodeHandle,
    steppers: Vec<(NodeHandle, NodeHandle, Props)>, // rendered root, container, props
    stash: Vec<NodeHandle>,
    every_step: bool,
}

fn container_of(rendered: &NodeHandle) -> NodeHandle {
    rendered
        .children()
        .into_iter()
        .find(|c| has_class(c, "rinch-stepper__steps"))
        .expect("steps row")
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
            steppers: Vec::new(),
            stash: Vec::new(),
            every_step,
        };
        world.with_mode(|world| {
            // 0: everything on. 1: nothing. 2: nested in a step of 0. 3: bare under 1's row (double claim).
            let nested_props = Props {
                active: 1,
                next: rng.one_in(2),
                click: true,
                ci: false,
                pi: true,
            };
            let nested_steps: Vec<NodeHandle> = (0..3).map(|_| world.step(rng, &[])).collect();
            let nested = nested_props
                .stepper()
                .render(&mut world.scope, &nested_steps);
            let nested_c = container_of(&nested);
            for (i, props) in [
                Props {
                    active: 2,
                    next: true,
                    click: true,
                    ci: true,
                    pi: true,
                },
                Props {
                    active: 3,
                    next: rng.one_in(2),
                    click: rng.one_in(2),
                    ci: rng.one_in(2),
                    pi: rng.one_in(2),
                },
            ]
            .into_iter()
            .enumerate()
            {
                let mut children: Vec<NodeHandle> = (0..3).map(|_| world.step(rng, &[])).collect();
                if i == 0 {
                    let host = world.step(rng, std::slice::from_ref(&nested));
                    children.push(host);
                }
                let wrapper = world.scope.create_element("div");
                for _ in 0..2 {
                    let st = world.step(rng, &[]);
                    wrapper.append_child(&st);
                }
                children.insert(1, wrapper);
                children.insert(3, StepperCompleted.render(&mut world.scope, &[]));
                let rendered = props.stepper().render(&mut world.scope, &children);
                let c = container_of(&rendered);
                world.root.append_child(&rendered);
                world.steppers.push((rendered, c, props));
            }
            world.steppers.push((nested, nested_c, nested_props));
        });
        world
    }

    fn with_mode<R>(&mut self, run: impl FnOnce(&mut Self) -> R) -> R {
        DERIVE_EVERY_STEP.with(|flag| flag.set(self.every_step));
        let out = run(self);
        DERIVE_EVERY_STEP.with(|flag| flag.set(false));
        out
    }

    fn step(&mut self, rng: &mut Rng, children: &[NodeHandle]) -> NodeHandle {
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
        .render(&mut self.scope, children)
    }

    /// Rows, wrappers and completed blocks.
    fn parents(&self) -> Vec<NodeHandle> {
        let mut out = Vec::new();
        for (_, container, _) in &self.steppers {
            out.push(container.clone());
            for child in container.children() {
                if !has_class(&child, STEP_CLASS) {
                    out.push(child);
                }
            }
        }
        out
    }

    /// Every step node anywhere under the root, completed blocks included.
    fn steps(&self) -> Vec<NodeHandle> {
        fn walk(n: &NodeHandle, out: &mut Vec<NodeHandle>) {
            if has_class(n, STEP_CLASS) {
                out.push(n.clone());
            }
            for c in n.children() {
                walk(&c, out);
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut out);
        out
    }

    fn is_under(node: &NodeHandle, anc: &NodeHandle) -> bool {
        let mut cur = Some(node.clone());
        while let Some(n) = cur {
            if n.node_id() == anc.node_id() {
                return true;
            }
            cur = n.parent_node();
        }
        false
    }

    fn place(&self, node: &NodeHandle, p: usize, at: usize) -> bool {
        let parents = self.parents();
        let parent = &parents[p % parents.len()];
        if Self::is_under(parent, node) {
            return false;
        }
        let siblings: Vec<NodeHandle> = parent
            .children()
            .into_iter()
            .filter(|c| c.node_id() != node.node_id())
            .collect();
        match siblings.get(at % (siblings.len() + 1)) {
            Some(reference) => parent.insert_before(node, reference),
            None => parent.append_child(node),
        }
        true
    }

    fn apply(&mut self, op: Op, rng: &mut Rng) {
        self.with_mode(|world| match op {
            Op::Insert { p, at } => {
                let step = world.step(rng, &[]);
                world.place(&step, p, at);
            }
            Op::Remove { which } => {
                let steps = world.steps();
                if steps.len() > 1 {
                    steps[which % steps.len()].remove();
                }
            }
            Op::Move { which, p, at } => {
                let steps = world.steps();
                if !steps.is_empty() {
                    world.place(&steps[which % steps.len()], p, at);
                }
            }
            Op::Stash { which } => {
                let steps = world.steps();
                if steps.len() > 1 {
                    let s = steps[which % steps.len()].clone();
                    s.remove();
                    world.stash.push(s);
                }
            }
            Op::Unstash { p, at } => {
                if let Some(s) = world.stash.pop() {
                    world.place(&s, p, at);
                }
            }
            Op::MoveGroup { which, p, at } => {
                // a wrapper or a completed block, with whatever it holds
                let groups: Vec<NodeHandle> = world
                    .parents()
                    .into_iter()
                    .filter(|n| !has_class(n, "rinch-stepper__steps"))
                    .collect();
                if !groups.is_empty() {
                    let g = groups[which % groups.len()].clone();
                    world.place(&g, p, at);
                }
            }
            Op::Rerender { which, active } => {
                // The same step nodes handed to a new render with other props.
                let idx = which % 2;
                let (old_root, old_c, _) = &world.steppers[idx];
                let (old_root, old_c) = (old_root.clone(), old_c.clone());
                let children = old_c.children();
                world.steppers[idx].2.active = active as u32;
                if active % 2 == 0 {
                    world.steppers[idx].2.next = !world.steppers[idx].2.next;
                }
                let stepper = world.steppers[idx].2.stepper();
                let rendered = stepper.render(&mut world.scope, &children);
                let c = container_of(&rendered);
                old_root.insert_after(&rendered);
                old_root.discard();
                world.steppers[idx].0 = rendered;
                world.steppers[idx].1 = c;
            }
        });
    }

    fn dump(&self) -> String {
        let mut out = String::new();
        let mut minted = HashMap::new();
        dump(&self.root, 0, &mut minted, &mut out);
        for s in &self.stash {
            dump(s, 1, &mut minted, &mut out);
        }
        out
    }
}
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
enum Op {
    Insert { p: usize, at: usize },
    Remove { which: usize },
    Move { which: usize, p: usize, at: usize },
    Stash { which: usize },
    Unstash { p: usize, at: usize },
    MoveGroup { which: usize, p: usize, at: usize },
    Rerender { which: usize, active: usize },
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
        None => out.push_str(&format!("{:?}\n", node.text_content())),
    }
}

fn run(seed: u64, ops: usize) -> usize {
    let mut rng_a = Rng(seed);
    let mut rng_b = Rng(seed);
    let mut history = Rng(seed ^ 0x9e3779b97f4a7c15);
    let mut skipping = World::new(false, &mut rng_a);
    let mut reference = World::new(true, &mut rng_b);
    assert_eq!(skipping.dump(), reference.dump(), "seed {seed}: at render");
    let mut applied = 0;
    for n in 0..ops {
        let op = match history.below(16) {
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
            9..=10 => Op::Stash {
                which: history.below(64),
            },
            11..=12 => Op::Unstash {
                p: history.below(64),
                at: history.below(64),
            },
            13..=14 => Op::MoveGroup {
                which: history.below(64),
                p: history.below(64),
                at: history.below(64),
            },
            _ => Op::Rerender {
                which: history.below(64),
                active: history.below(7),
            },
        };
        skipping.apply(op, &mut rng_a);
        reference.apply(op, &mut rng_b);
        applied += 1;
        let (got, want) = (skipping.dump(), reference.dump());
        assert!(
            got == want,
            "seed {seed}, change {n} ({op:?}):\n--- skipping ---\n{got}\n--- every step ---\n{want}"
        );
    }
    applied
}

#[test]
fn the_skipping_pass_matches_the_whole_pass_over_the_shapes_the_review_added() {
    let seeds: u64 = std::env::var("RINCH_REVIEW_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(40);
    let mut total = 0;
    for seed in 1..=seeds {
        total += run(seed, 80);
    }
    assert!(total > 0);
    eprintln!("extended differential: {seeds} seeds, {total} changes");
}
