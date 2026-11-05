//! [`GraphView`]: the handle that owns a [`GraphState`] and mirrors it into
//! signals, so each node, port and edge on screen updates on its own.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;

use rinch_core::{ElementBounds, Signal, batch, untracked};
use rinch_graph_core::{
    ConnectPlan, EdgeId, Graph, GraphError, GraphState, NodeId, Point, PortDir, PortId, Rect,
    Rules, Selection, Size, Step, Tx, check_connect,
};

use crate::geometry::{EdgeShape, Flow, Viewport, distance_to_edge};

/// Canvas settings. Change them with [`GraphView::configure`].
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub flow: Flow,
    pub edge_shape: EdgeShape,
    /// Snap dragged nodes to a grid of this many graph units (0: off).
    pub snap: f32,
    pub min_zoom: f32,
    pub max_zoom: f32,
    /// How close (in screen pixels) a dropped wire must be to a port to
    /// connect to it.
    pub port_snap_radius: f32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            flow: Flow::LeftToRight,
            edge_shape: EdgeShape::Bezier,
            snap: 0.0,
            min_zoom: 0.2,
            max_zoom: 3.0,
            port_snap_radius: 24.0,
        }
    }
}

/// A wire being drawn from a port.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingWire {
    /// The port the drag started on.
    pub from: PortId,
    /// The pointer, in graph space.
    pub cursor: Point,
}

/// A port's role while a wire is being drawn, for styling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortHint {
    /// No wire is being drawn, or this port is not a candidate.
    Idle,
    /// The port the wire starts from.
    Source,
    /// The rules allow connecting here.
    Candidate,
    /// The wire would connect here if dropped now.
    Target,
}

/// One mounted node's signals. Created when its frame renders, owned by it.
#[derive(Clone, Copy)]
pub(crate) struct NodeSlot {
    pub pos: Signal<Point>,
    pub selected: Signal<bool>,
    /// Bumped when the node's data or port list changes.
    pub rev: Signal<u64>,
    /// The frame's size in graph units, as last measured.
    pub size: Signal<Size>,
}

#[derive(Clone, Copy)]
pub(crate) struct PortSlot {
    /// The port's centre relative to its node's origin, in graph units, once
    /// measured.
    pub offset: Signal<Option<Point>>,
    pub rev: Signal<u64>,
}

#[derive(Clone, Copy)]
pub(crate) struct EdgeSlot {
    pub selected: Signal<bool>,
}

/// Makes the data of a wire the user draws.
pub(crate) type NewEdge<N, P, E> = Box<dyn Fn(&Graph<N, P, E>, PortId, PortId) -> E>;

pub(crate) struct Inner<N: 'static, P: 'static, E: 'static> {
    pub state: RefCell<GraphState<N, P, E>>,
    pub rules: Box<dyn Rules<N, P, E>>,
    pub new_edge: NewEdge<N, P, E>,
    pub config: RefCell<Config>,
    pub config_sig: Signal<u64>,

    pub node_ids: Signal<Vec<NodeId>>,
    pub edge_ids: Signal<Vec<EdgeId>>,
    pub revision: Signal<u64>,
    /// Bumped whenever a node frame or port dot mounts, so geometry computed
    /// before it existed (an edge drawn before its nodes) is computed again.
    pub mounts: Signal<u64>,
    pub selection: Signal<Selection>,
    pub history: Signal<(bool, bool)>,
    pub viewport: Signal<Viewport>,
    /// The canvas element's size in screen pixels, set by the canvas.
    pub canvas_size: Signal<Size>,

    pub wire: Signal<Option<PendingWire>>,
    /// Ports the current wire may connect to (fixed for the whole drag).
    pub wire_candidates: Signal<Option<Rc<HashSet<PortId>>>>,
    pub wire_target: Signal<Option<PortId>>,
    pub marquee: Signal<Option<Rect>>,

    pub nodes: RefCell<HashMap<NodeId, NodeSlot>>,
    pub ports: RefCell<HashMap<PortId, PortSlot>>,
    pub edges: RefCell<HashMap<EdgeId, EdgeSlot>>,
    /// Each mounted node frame's on-screen box, for measuring its ports.
    pub frames: RefCell<HashMap<NodeId, Signal<ElementBounds>>>,
    /// The canvas element's on-screen box, set by the canvas.
    pub canvas_origin: RefCell<Option<Signal<ElementBounds>>>,
    pub connect_to_empty: RefCell<Option<crate::canvas::ConnectToEmpty>>,
    /// Which node each port is on, kept so a removed port's node can be found.
    pub port_owner: RefCell<HashMap<PortId, NodeId>>,
    /// Nodes being dragged right now: their positions are live in their slots
    /// and not yet in the model.
    pub dragging: Cell<bool>,
}

/// The handle behind a graph canvas: owns the [`GraphState`] and the app's
/// [`Rules`], applies edits, and keeps the canvas in step. Cheap to clone.
///
/// Create it inside a component (its signals belong to that component), hand
/// it to [`graph_canvas`](crate::graph_canvas), and use it from node bodies
/// and toolbars.
///
/// `Copy`, like [`Signal`]: the state lives in the reactive store and belongs
/// to the component that created the view, so closures capture it freely.
pub struct GraphView<N: 'static, P: 'static, E: 'static>(Signal<Rc<Inner<N, P, E>>>);

impl<N, P, E> Clone for GraphView<N, P, E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<N, P, E> Copy for GraphView<N, P, E> {}

impl<N, P, E> GraphView<N, P, E> {
    /// The shared state. Untracked: holding the view subscribes to nothing.
    pub(crate) fn i(&self) -> Rc<Inner<N, P, E>> {
        untracked(|| self.0.with(Rc::clone))
    }
}

impl<N, P, E> std::fmt::Debug for GraphView<N, P, E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GraphView")
    }
}

impl<N: Clone + 'static, P: Clone + 'static, E: Clone + Default + 'static> GraphView<N, P, E> {
    /// A view over `state` under `rules`. Wires the user draws get
    /// `E::default()` as their data.
    pub fn new(state: GraphState<N, P, E>, rules: impl Rules<N, P, E> + 'static) -> Self {
        Self::with_edge_data(state, rules, |_, _, _| E::default())
    }
}

impl<N: Clone + 'static, P: Clone + 'static, E: Clone + 'static> GraphView<N, P, E> {
    /// A view whose drawn wires get their data from `new_edge(graph, from, to)`.
    pub fn with_edge_data(
        state: GraphState<N, P, E>,
        rules: impl Rules<N, P, E> + 'static,
        new_edge: impl Fn(&Graph<N, P, E>, PortId, PortId) -> E + 'static,
    ) -> Self {
        let node_ids = Signal::new(state.graph().node_ids().collect());
        let edge_ids = Signal::new(state.graph().edge_ids().collect());
        let history = Signal::new((state.can_undo(), state.can_redo()));
        let selection = Signal::new(state.selection().clone());
        let port_owner = state.graph().ports().map(|p| (p.id, p.node)).collect();
        GraphView(Signal::new(Rc::new(Inner {
            state: RefCell::new(state),
            rules: Box::new(rules),
            new_edge: Box::new(new_edge),
            config: RefCell::new(Config::default()),
            config_sig: Signal::new(0),
            node_ids,
            edge_ids,
            revision: Signal::new(0),
            mounts: Signal::new(0),
            selection,
            history,
            viewport: Signal::new(Viewport::default()),
            canvas_size: Signal::new(Size::default()),
            wire: Signal::new(None),
            wire_candidates: Signal::new(None),
            wire_target: Signal::new(None),
            marquee: Signal::new(None),
            nodes: RefCell::new(HashMap::new()),
            ports: RefCell::new(HashMap::new()),
            edges: RefCell::new(HashMap::new()),
            port_owner: RefCell::new(port_owner),
            frames: RefCell::new(HashMap::new()),
            canvas_origin: RefCell::new(None),
            connect_to_empty: RefCell::new(None),
            dragging: Cell::new(false),
        })))
    }

    // ---- reading -------------------------------------------------------

    /// Read the graph. Tracked: a closure reading this re-runs on any change.
    pub fn graph<R>(&self, f: impl FnOnce(&Graph<N, P, E>) -> R) -> R {
        self.i().revision.get();
        f(self.i().state.borrow().graph())
    }

    /// Read the whole state (graph, selection, history) without tracking.
    pub fn with_state<R>(&self, f: impl FnOnce(&GraphState<N, P, E>) -> R) -> R {
        f(&self.i().state.borrow())
    }

    /// A node's data. Tracked per node: re-runs only when this node's data or
    /// port list changes.
    pub fn node_data(&self, node: NodeId) -> Option<N> {
        self.track_node(node);
        untracked(|| {
            self.i()
                .state
                .borrow()
                .graph()
                .node(node)
                .map(|n| n.data.clone())
        })
    }

    /// A node's ports facing `dir`, in display order. Tracked per node.
    pub fn node_ports(&self, node: NodeId, dir: PortDir) -> Vec<PortId> {
        self.track_node(node);
        self.i().state.borrow().graph().node_ports(node, dir)
    }

    /// A port's data. Tracked per port.
    pub fn port_data(&self, port: PortId) -> Option<P> {
        match self.i().ports.borrow().get(&port) {
            Some(slot) => {
                slot.rev.get();
            }
            None => {
                self.i().revision.get();
            }
        }
        self.i()
            .state
            .borrow()
            .graph()
            .port(port)
            .map(|p| p.data.clone())
    }

    fn track_node(&self, node: NodeId) {
        let slot = self.i().nodes.borrow().get(&node).copied();
        match slot {
            Some(slot) => {
                slot.rev.get();
            }
            None => {
                self.i().revision.get();
            }
        }
    }

    /// The selection. Tracked.
    pub fn selection(&self) -> Selection {
        self.i().selection.get()
    }

    pub fn can_undo(&self) -> bool {
        self.i().history.get().0
    }

    pub fn can_redo(&self) -> bool {
        self.i().history.get().1
    }

    /// The pan and zoom. Tracked.
    pub fn viewport(&self) -> Viewport {
        self.i().viewport.get()
    }

    /// The wire being drawn, if any. Tracked.
    pub fn pending_wire(&self) -> Option<PendingWire> {
        self.i().wire.get()
    }

    /// A port's role in the wire being drawn. Tracked; changes only when a
    /// drag starts or ends or the target port changes.
    pub fn port_hint(&self, port: PortId) -> PortHint {
        let Some(wire) = self.i().wire.with(|w| w.as_ref().map(|w| w.from)) else {
            return PortHint::Idle;
        };
        if wire == port {
            return PortHint::Source;
        }
        if self.i().wire_target.get() == Some(port) {
            return PortHint::Target;
        }
        let candidate = self
            .i()
            .wire_candidates
            .with(|c| c.as_ref().is_some_and(|c| c.contains(&port)));
        if candidate {
            PortHint::Candidate
        } else {
            PortHint::Idle
        }
    }

    pub fn config(&self) -> Config {
        self.i().config_sig.get();
        self.i().config.borrow().clone()
    }

    pub fn configure(&self, f: impl FnOnce(&mut Config)) {
        f(&mut self.i().config.borrow_mut());
        self.i().config_sig.update(|v| *v += 1);
    }

    // ---- editing -------------------------------------------------------

    /// Make an undoable change (see [`GraphState::edit`]) and update the
    /// canvas.
    pub fn edit<R>(
        &self,
        f: impl FnOnce(&mut Tx<'_, N, P, E>) -> Result<R, GraphError>,
    ) -> Result<R, GraphError> {
        let r = self.i().state.borrow_mut().edit(f);
        self.sync();
        r
    }

    /// Connect two ports (in either order) under the view's rules.
    pub fn connect(&self, a: PortId, b: PortId) -> Result<EdgeId, GraphError> {
        let data = {
            let inner = self.i();
            let state = inner.state.borrow();
            let plan = check_connect(state.graph(), &*self.i().rules, a, b)?;
            (self.i().new_edge)(state.graph(), plan.from, plan.to)
        };
        let inner = self.i();
        self.edit(move |tx| tx.connect(&*inner.rules, a, b, data))
    }

    /// Whether `a` and `b` may be connected, and how. Untracked.
    pub fn check_connect(&self, a: PortId, b: PortId) -> Result<ConnectPlan, GraphError> {
        check_connect(self.i().state.borrow().graph(), &*self.i().rules, a, b)
    }

    pub fn undo(&self) -> bool {
        let done = self.i().state.borrow_mut().undo();
        self.sync();
        done
    }

    pub fn redo(&self) -> bool {
        let done = self.i().state.borrow_mut().redo();
        self.sync();
        done
    }

    pub fn set_selection(&self, selection: Selection) {
        self.i().state.borrow_mut().set_selection(selection);
        self.sync();
    }

    pub fn select_all(&self) {
        let all = {
            let inner = self.i();
            let state = inner.state.borrow();
            Selection {
                nodes: state.graph().node_ids().collect(),
                edges: state.graph().edge_ids().collect(),
            }
        };
        self.set_selection(all);
    }

    /// Remove the selected nodes and edges, as one undo step.
    pub fn delete_selected(&self) {
        let _ = self.edit(|tx| tx.remove_selected());
    }

    /// Replace the whole state (a loaded file) and redraw.
    pub fn replace_state(&self, state: GraphState<N, P, E>) {
        *self.i().port_owner.borrow_mut() = state.graph().ports().map(|p| (p.id, p.node)).collect();
        *self.i().state.borrow_mut() = state;
        let (nodes, edges, sel, hist, positions) = {
            let inner = self.i();
            let s = inner.state.borrow();
            let g = s.graph();
            let positions: Vec<(NodeId, Point)> = self
                .i()
                .nodes
                .borrow()
                .keys()
                .filter_map(|n| Some((*n, g.node(*n)?.pos)))
                .collect();
            (
                g.node_ids().collect::<Vec<_>>(),
                g.edge_ids().collect::<Vec<_>>(),
                s.selection().clone(),
                (s.can_undo(), s.can_redo()),
                positions,
            )
        };
        batch(|| {
            // Nodes that stay mounted (same id) take the new position and data.
            for (n, pos) in positions {
                if let Some(slot) = self.node_slot(n) {
                    slot.pos.set_if_changed(pos);
                    slot.rev.update(|r| *r += 1);
                }
            }
            let ports: Vec<PortSlot> = self.i().ports.borrow().values().copied().collect();
            for slot in ports {
                slot.rev.update(|r| *r += 1);
            }
            self.i().node_ids.set_if_changed(nodes);
            self.i().edge_ids.set_if_changed(edges);
            self.apply_selection(sel);
            self.i().history.set_if_changed(hist);
            self.i().revision.update(|r| *r += 1);
        });
    }

    // ---- viewport ------------------------------------------------------

    pub fn set_viewport(&self, viewport: Viewport) {
        self.i().viewport.set_if_changed(viewport);
    }

    /// Multiply the zoom by `factor` around the canvas centre.
    pub fn zoom_by(&self, factor: f32) {
        let size = untracked(|| self.i().canvas_size.get());
        let center = Point::new(size.width / 2.0, size.height / 2.0);
        self.zoom_around(factor, center);
    }

    /// Multiply the zoom by `factor`, keeping the graph point under `anchor`
    /// (canvas pixels) in place.
    pub fn zoom_around(&self, factor: f32, anchor: Point) {
        let cfg = self.i().config.borrow().clone();
        let v = untracked(|| self.i().viewport.get());
        self.set_viewport(v.zoomed_around(v.zoom * factor, anchor, cfg.min_zoom, cfg.max_zoom));
    }

    /// Pan and zoom so every node is in view.
    pub fn fit_view(&self) {
        let Some(bounds) = untracked(|| self.content_bounds()) else {
            self.set_viewport(Viewport::default());
            return;
        };
        let cfg = self.i().config.borrow().clone();
        let size = untracked(|| self.i().canvas_size.get());
        if size.width <= 0.0 || size.height <= 0.0 {
            return;
        }
        self.set_viewport(Viewport::fitting(
            bounds,
            size,
            40.0,
            cfg.min_zoom,
            cfg.max_zoom.min(1.0),
        ));
    }

    /// The rectangle holding every node, in graph space (tracked per node).
    pub fn content_bounds(&self) -> Option<Rect> {
        let ids = self.i().node_ids.get();
        ids.iter()
            .filter_map(|id| self.node_rect(*id))
            .reduce(|a, b| a.union(&b))
    }

    /// A node's rectangle in graph space. Tracked.
    pub fn node_rect(&self, node: NodeId) -> Option<Rect> {
        let slot = self.i().nodes.borrow().get(&node).copied();
        match slot {
            Some(s) => Some(Rect::from_origin_size(s.pos.try_get()?, s.size.try_get()?)),
            None => {
                self.i().revision.get();
                self.i().mounts.get();
                let pos = self.i().state.borrow().graph().node(node)?.pos;
                Some(Rect::from_origin_size(pos, Size::default()))
            }
        }
    }

    // ---- geometry used by the canvas -----------------------------------

    /// Where a port's wire attaches, in graph space. Tracked: the node's
    /// position and the port's measured offset.
    pub fn port_point(&self, port: PortId) -> Option<Point> {
        let (node, dir) = {
            let inner = self.i();
            let state = inner.state.borrow();
            let p = state.graph().port(port)?;
            (p.node, p.dir)
        };
        let rect = self.node_rect(node)?;
        let slot = self.i().ports.borrow().get(&port).copied();
        let measured = match slot {
            Some(s) => s.offset.try_get().flatten(),
            None => {
                // Not mounted yet, or never placed: hear when it is.
                self.i().mounts.get();
                None
            }
        };
        if let Some(off) = measured {
            return Some(Point::new(rect.x + off.x, rect.y + off.y));
        }
        // Not placed by the node body: the middle of the side the flow says.
        let flow = self.i().config.borrow().flow;
        Some(match (flow, dir) {
            (Flow::LeftToRight, PortDir::In) => Point::new(rect.x, rect.y + rect.height / 2.0),
            (Flow::LeftToRight, PortDir::Out) => {
                Point::new(rect.right(), rect.y + rect.height / 2.0)
            }
            (Flow::TopToBottom, PortDir::In) => Point::new(rect.x + rect.width / 2.0, rect.y),
            (Flow::TopToBottom, PortDir::Out) => {
                Point::new(rect.x + rect.width / 2.0, rect.bottom())
            }
        })
    }

    /// An edge's two ends, in graph space. Tracked.
    pub fn edge_points(&self, edge: EdgeId) -> Option<(Point, Point)> {
        let (from, to) = {
            let inner = self.i();
            let state = inner.state.borrow();
            let e = state.graph().edge(edge)?;
            (e.from, e.to)
        };
        Some((self.port_point(from)?, self.port_point(to)?))
    }

    /// The edge nearest `p` (graph space) within `tolerance` graph units.
    pub fn edge_at(&self, p: Point, tolerance: f32) -> Option<EdgeId> {
        untracked(|| {
            let cfg = self.i().config.borrow().clone();
            let ids = self.i().edge_ids.get();
            ids.into_iter()
                .filter_map(|e| {
                    let (a, b) = self.edge_points(e)?;
                    let d = distance_to_edge(p, a, b, cfg.edge_shape, cfg.flow);
                    (d <= tolerance).then_some((e, d))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(e, _)| e)
        })
    }

    /// The port nearest `p` (graph space) within `radius` graph units among
    /// `candidates`.
    pub(crate) fn port_near(
        &self,
        p: Point,
        radius: f32,
        candidates: &HashSet<PortId>,
    ) -> Option<PortId> {
        untracked(|| {
            candidates
                .iter()
                .filter_map(|port| {
                    let q = self.port_point(*port)?;
                    let d = ((q.x - p.x).powi(2) + (q.y - p.y).powi(2)).sqrt();
                    (d <= radius).then_some((*port, d))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
                .map(|(port, _)| port)
        })
    }

    /// Every port the rules would let `from` connect to.
    pub(crate) fn candidates_for(&self, from: PortId) -> HashSet<PortId> {
        let inner = self.i();
        let state = inner.state.borrow();
        let g = state.graph();
        g.ports()
            .filter(|p| check_connect(g, &*self.i().rules, from, p.id).is_ok())
            .map(|p| p.id)
            .collect()
    }

    /// Nodes whose rectangles meet `rect` (graph space).
    pub(crate) fn nodes_in(&self, rect: Rect) -> BTreeSet<NodeId> {
        untracked(|| {
            self.i()
                .node_ids
                .get()
                .into_iter()
                .filter(|n| self.node_rect(*n).is_some_and(|r| r.intersects(&rect)))
                .collect()
        })
    }

    // ---- mounting (called by the canvas inside item renders) -----------

    /// Create a node's signals under the current owner (the node frame) and
    /// register them. Removed again when the frame unmounts.
    pub(crate) fn mount_node(
        &self,
        node: NodeId,
        frame: Signal<ElementBounds>,
    ) -> Option<NodeSlot> {
        let (pos, selected) = {
            let inner = self.i();
            let state = inner.state.borrow();
            let pos = state.graph().node(node)?.pos;
            (pos, state.selection().nodes.contains(&node))
        };
        let slot = NodeSlot {
            pos: Signal::new(pos),
            selected: Signal::new(selected),
            rev: Signal::new(0),
            size: Signal::new(Size::default()),
        };
        self.i().nodes.borrow_mut().insert(node, slot);
        self.i().mounts.update(|m| *m += 1);
        self.i().frames.borrow_mut().insert(node, frame);
        let weak = Rc::downgrade(&self.i());
        rinch_core::reactive::on_cleanup(move || {
            if let Some(inner) = weak.upgrade() {
                let mut nodes = inner.nodes.borrow_mut();
                if nodes
                    .get(&node)
                    .is_some_and(|s| s.pos.identity() == slot.pos.identity())
                {
                    nodes.remove(&node);
                    inner.frames.borrow_mut().remove(&node);
                }
            }
        });
        Some(slot)
    }

    pub(crate) fn mount_port(&self, port: PortId) -> PortSlot {
        let slot = PortSlot {
            offset: Signal::new(None),
            rev: Signal::new(0),
        };
        self.i().ports.borrow_mut().insert(port, slot);
        self.i().mounts.update(|m| *m += 1);
        let weak = Rc::downgrade(&self.i());
        rinch_core::reactive::on_cleanup(move || {
            if let Some(inner) = weak.upgrade() {
                let mut ports = inner.ports.borrow_mut();
                if ports
                    .get(&port)
                    .is_some_and(|s| s.offset.identity() == slot.offset.identity())
                {
                    ports.remove(&port);
                }
            }
        });
        slot
    }

    pub(crate) fn mount_edge(&self, edge: EdgeId) -> EdgeSlot {
        let selected = untracked(|| self.i().state.borrow().selection().edges.contains(&edge));
        let slot = EdgeSlot {
            selected: Signal::new(selected),
        };
        self.i().edges.borrow_mut().insert(edge, slot);
        let weak = Rc::downgrade(&self.i());
        rinch_core::reactive::on_cleanup(move || {
            if let Some(inner) = weak.upgrade() {
                let mut edges = inner.edges.borrow_mut();
                if edges
                    .get(&edge)
                    .is_some_and(|s| s.selected.identity() == slot.selected.identity())
                {
                    edges.remove(&edge);
                }
            }
        });
        slot
    }

    pub(crate) fn node_slot(&self, node: NodeId) -> Option<NodeSlot> {
        self.i().nodes.borrow().get(&node).copied()
    }

    /// Commit a finished node drag: one undo step from the model positions to
    /// the live ones in the slots.
    pub(crate) fn commit_drag(&self, nodes: &[NodeId]) {
        self.i().dragging.set(false);
        let moves: Vec<(NodeId, Point)> = nodes
            .iter()
            .filter_map(|n| {
                let pos = untracked(|| self.node_slot(*n)?.pos.try_get())?;
                let model = self.i().state.borrow().graph().node(*n)?.pos;
                (pos != model).then_some((*n, pos))
            })
            .collect();
        if !moves.is_empty() {
            let _ = self.edit(|tx| tx.move_nodes(moves));
        }
    }

    /// Put dragged nodes back where the model has them (a cancelled drag).
    pub(crate) fn revert_drag(&self, nodes: &[NodeId]) {
        self.i().dragging.set(false);
        batch(|| {
            for n in nodes {
                let model = self.i().state.borrow().graph().node(*n).map(|n| n.pos);
                if let (Some(slot), Some(pos)) = (self.node_slot(*n), model) {
                    slot.pos.set_if_changed(pos);
                }
            }
        });
    }

    // ---- syncing the signals from the model ----------------------------

    /// Mirror whatever the state applied since the last sync into the signals.
    pub(crate) fn sync(&self) {
        let mut nodes_changed = false;
        let mut edges_changed = false;
        let mut moved: Vec<NodeId> = Vec::new();
        let mut node_revs: Vec<NodeId> = Vec::new();
        let mut port_revs: Vec<PortId> = Vec::new();
        let (selection, history, node_ids, edge_ids, positions) = {
            let inner = self.i();
            let mut state = inner.state.borrow_mut();
            let steps = state.take_applied();
            let selection_changed = state.take_selection_changed();
            if steps.is_empty() && !selection_changed {
                return;
            }
            let mut owners = inner.port_owner.borrow_mut();
            for step in &steps {
                match step {
                    Step::InsertNode { node, .. } => {
                        nodes_changed = true;
                        for p in &node.ports {
                            owners.insert(*p, node.id);
                        }
                    }
                    Step::RemoveNode(_) => nodes_changed = true,
                    Step::InsertPort { port, .. } => {
                        owners.insert(port.id, port.node);
                        node_revs.push(port.node);
                    }
                    Step::RemovePort(port) => {
                        if let Some(n) = owners.get(port) {
                            node_revs.push(*n);
                        }
                    }
                    Step::MoveNodes(m) => moved.extend(m.iter().map(|(n, _)| *n)),
                    Step::SetNodeData(n, _) => node_revs.push(*n),
                    Step::SetPortData(p, _) => port_revs.push(*p),
                    Step::InsertEdge(_) | Step::RemoveEdge(_) => edges_changed = true,
                    Step::SetEdgeData(..) => {}
                }
            }
            let g = state.graph();
            let positions: Vec<(NodeId, Point)> = moved
                .iter()
                .filter_map(|n| Some((*n, g.node(*n)?.pos)))
                .collect();
            (
                state.selection().clone(),
                (state.can_undo(), state.can_redo()),
                nodes_changed.then(|| g.node_ids().collect::<Vec<_>>()),
                (edges_changed || nodes_changed).then(|| g.edge_ids().collect::<Vec<_>>()),
                positions,
            )
        };

        batch(|| {
            if let Some(ids) = node_ids {
                self.i().node_ids.set_if_changed(ids);
            }
            if let Some(ids) = edge_ids {
                self.i().edge_ids.set_if_changed(ids);
            }
            for (n, pos) in positions {
                if let Some(slot) = self.node_slot(n) {
                    slot.pos.set_if_changed(pos);
                }
            }
            for n in node_revs {
                if let Some(slot) = self.node_slot(n) {
                    slot.rev.update(|r| *r += 1);
                }
            }
            for p in port_revs {
                let slot = self.i().ports.borrow().get(&p).copied();
                if let Some(slot) = slot {
                    slot.rev.update(|r| *r += 1);
                }
            }
            self.apply_selection(selection);
            self.i().history.set_if_changed(history);
            self.i().revision.update(|r| *r += 1);
        });
    }

    fn apply_selection(&self, selection: Selection) {
        let old = untracked(|| self.i().selection.get());
        if old == selection {
            return;
        }
        for n in old.nodes.symmetric_difference(&selection.nodes) {
            if let Some(slot) = self.node_slot(*n) {
                slot.selected.set_if_changed(selection.nodes.contains(n));
            }
        }
        for e in old.edges.symmetric_difference(&selection.edges) {
            if let Some(slot) = self.i().edges.borrow().get(e).copied() {
                slot.selected.set_if_changed(selection.edges.contains(e));
            }
        }
        self.i().selection.set(selection);
    }
}
