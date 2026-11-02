//! The canvas: [`graph_canvas`] builds the DOM for a [`GraphView`] and wires
//! up every pointer and key interaction.
//!
//! ```text
//! div.rinch-graph            (focusable; pan, marquee, edge picking, keys, wheel)
//! ├── style
//! ├── div.rinch-graph-world  (one transform: the viewport)
//! │   ├── svg.rinch-graph-edges   (pointer-events: none)
//! │   │   ├── path × edges
//! │   │   └── path (the wire being drawn)
//! │   └── div.rinch-graph-node × nodes  (app body inside)
//! ├── div.rinch-graph-marquee
//! └── div.rinch-graph-controls
//! ```

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{
    Drag, KeyEventData, WheelEvent, batch, for_each_dom_typed, get_click_context, untracked,
};
use rinch_graph_core::{EdgeId, NodeId, Point, PortDir, PortId, Rect, Selection, Size};

use crate::geometry::{Viewport, edge_path};
use crate::style::CANVAS_CSS;
use crate::view::{GraphView, PendingWire, PortHint};

/// Called when a wire is dropped on empty canvas: the port it came from and
/// the drop point in graph space. The usual answer is a menu of nodes to add
/// there, connected.
pub type ConnectToEmpty = Rc<dyn Fn(PortId, Point)>;

/// Options for [`graph_canvas`].
#[derive(Clone, Default)]
pub struct CanvasOptions {
    /// Extra inline style for the canvas element; give it a size here
    /// (it fills its container by default).
    pub style: String,
    /// Show the zoom buttons in the corner.
    pub controls: bool,
    /// Draw a dot grid behind the graph.
    pub grid: bool,
    pub on_connect_to_empty: Option<ConnectToEmpty>,
    /// Fit the view to the graph once the canvas has a size.
    pub fit_on_mount: bool,
}

/// Build the canvas for `view`. `render_node` draws the inside of a node; it
/// runs once per node, when the node appears, inside the node's own scope.
/// Place each port's dot with [`GraphView::port_dot`] where the body wants it;
/// a port without a dot attaches its wires to the middle of the node's side.
pub fn graph_canvas<N, P, E>(
    scope: &mut RenderScope,
    view: &GraphView<N, P, E>,
    render_node: impl Fn(&mut RenderScope, NodeId) -> NodeHandle + 'static,
    options: CanvasOptions,
) -> NodeHandle
where
    N: Clone + 'static,
    P: Clone + 'static,
    E: Clone + 'static,
{
    let render_node = Rc::new(render_node);
    let root = scope.create_element("div");
    root.set_attribute("class", "rinch-graph");
    root.set_attribute("tabindex", "0");
    if !options.style.is_empty() {
        root.set_attribute("style", &options.style);
    }

    let css = scope.create_element("style");
    css.append_child(&scope.create_text(CANVAS_CSS));
    root.append_child(&css);

    // The canvas's own size and origin.
    let root_bounds = root.bounds_signal();
    {
        let v = *view;
        let fit = Cell::new(options.fit_on_mount);
        scope.create_effect(move || {
            let b = root_bounds.get();
            v.i()
                .canvas_size
                .set_if_changed(Size::new(b.width, b.height));
            if fit.get() && b.width > 0.0 && b.height > 0.0 {
                fit.set(false);
                untracked(|| v.fit_view());
            }
        });
    }

    // Grid: a dot pattern that follows the pan and zoom.
    if options.grid {
        let v = *view;
        let el = root.clone();
        scope.create_effect(move || {
            let vp = v.viewport();
            let step = 20.0 * vp.zoom;
            el.set_styles(&[
                (
                    "background-image",
                    "radial-gradient(circle, var(--rinch-graph-grid, #cdd2d9) 1px, transparent 1.5px)",
                ),
                ("background-size", &format!("{step}px {step}px")),
                (
                    "background-position",
                    &format!("{}px {}px", vp.x.rem_euclid(step), vp.y.rem_euclid(step)),
                ),
            ]);
        });
    }

    // ---- the world layer ------------------------------------------------
    let world = scope.create_element("div");
    world.set_attribute("class", "rinch-graph-world");
    {
        let v = *view;
        let world = world.clone();
        scope.create_effect(move || {
            let vp = v.viewport();
            world.set_style(
                "transform",
                &format!("translate({}px, {}px) scale({})", vp.x, vp.y, vp.zoom),
            );
        });
    }
    root.append_child(&world);

    build_edge_layer(scope, view, &world);

    let nodes_parent = world.clone();
    {
        let v = *view;
        let render = render_node.clone();
        let ids = v.i().node_ids;
        for_each_dom_typed(
            scope,
            &nodes_parent,
            move || ids.get(),
            |id: &NodeId| id.0.to_string(),
            move |id, item| build_node(item, &v, id, &*render),
        );
    }

    // ---- overlays -------------------------------------------------------
    let marquee = scope.create_element("div");
    marquee.set_attribute("class", "rinch-graph-marquee");
    {
        let v = *view;
        let m = marquee.clone();
        scope.create_effect(move || match v.i().marquee.get() {
            Some(r) => {
                let vp = v.viewport();
                let a = vp.to_canvas(Point::new(r.x, r.y));
                m.set_styles(&[
                    ("display", "block"),
                    ("left", &format!("{}px", a.x)),
                    ("top", &format!("{}px", a.y)),
                    ("width", &format!("{}px", r.width * vp.zoom)),
                    ("height", &format!("{}px", r.height * vp.zoom)),
                ]);
            }
            None => m.set_style("display", "none"),
        });
    }
    root.append_child(&marquee);

    if options.controls {
        root.append_child(&build_controls(scope, view));
    }

    // ---- canvas pointer: edge pick, marquee, pan --------------------------
    {
        let v = *view;
        let handler = scope.register_handler(move || canvas_press(&v));
        root.set_attribute("data-rid", &handler.0.to_string());
    }

    // ---- keys -------------------------------------------------------------
    {
        let v = *view;
        let handler = scope.register_key_handler(move |k: &KeyEventData| canvas_key(&v, k));
        root.set_attribute("data-onkeydown", &handler.0.to_string());
    }

    // ---- wheel: zoom on ctrl/cmd+wheel and pinch, pan otherwise ----------
    {
        let v = *view;
        let handler = scope.register_wheel_handler(move |w: &WheelEvent| canvas_wheel(&v, w));
        root.set_attribute("data-onwheel", &handler.0.to_string());
    }

    // Port dots look up the drop callback through the view.
    *view.i().connect_to_empty.borrow_mut() = options.on_connect_to_empty.clone();
    *view.i().canvas_origin.borrow_mut() = Some(root_bounds);

    root
}

/// The pointer position in canvas pixels.
fn canvas_point(view: &GraphView<impl Sized, impl Sized, impl Sized>, x: f32, y: f32) -> Point {
    let origin = view
        .i()
        .canvas_origin
        .borrow()
        .map(|b| untracked(|| b.get()))
        .map_or((0.0, 0.0), |b| (b.x, b.y));
    Point::new(x - origin.0, y - origin.1)
}

fn canvas_press<N: Clone, P: Clone, E: Clone>(view: &GraphView<N, P, E>) {
    let ctx = get_click_context();
    let additive = ctx.modifiers.shift || ctx.modifiers.ctrl || ctx.modifiers.meta;
    let vp = untracked(|| view.viewport());
    let start = canvas_point(view, ctx.mouse_x, ctx.mouse_y);
    let g = vp.to_graph(start);

    // An edge under the pointer.
    if let Some(e) = view.edge_at(g, 8.0 / vp.zoom) {
        let mut sel = untracked(|| view.selection());
        if additive {
            if !sel.edges.remove(&e) {
                sel.edges.insert(e);
            }
        } else {
            sel = Selection::edges([e]);
        }
        view.set_selection(sel);
        return;
    }

    if additive {
        // Marquee: add the nodes it touches to the selection.
        let v = *view;
        let v_end = *view;
        let v_cancel = *view;
        let base = untracked(|| view.selection());
        Drag::absolute()
            .on_move(move |x, y| {
                let p = vp.to_graph(canvas_point(&v, x, y));
                v.i().marquee.set(Some(Rect::from_corners(g, p)));
            })
            .on_end(move |_, _| {
                if let Some(r) = untracked(|| v_end.i().marquee.get()) {
                    let mut sel = base.clone();
                    sel.nodes.extend(v_end.nodes_in(r));
                    v_end.i().marquee.set(None);
                    v_end.set_selection(sel);
                }
            })
            .on_cancel(move |_, _| v_cancel.i().marquee.set(None))
            .start();
        return;
    }

    // Pan; a press that does not move clears the selection.
    let v = *view;
    let v_end = *view;
    let moved = Rc::new(Cell::new(false));
    let moved_end = moved.clone();
    let (sx, sy) = (ctx.mouse_x, ctx.mouse_y);
    Drag::absolute()
        .on_move(move |x, y| {
            if !moved.get() && (x - sx).abs() + (y - sy).abs() < 3.0 {
                return;
            }
            moved.set(true);
            v.set_viewport(Viewport {
                x: vp.x + (x - sx),
                y: vp.y + (y - sy),
                zoom: vp.zoom,
            });
        })
        .on_end(move |_, _| {
            if !moved_end.get() {
                v_end.set_selection(Selection::default());
            }
        })
        .start();
}

/// A wheel over the canvas. Ctrl/Cmd+wheel and a trackpad pinch (which
/// arrives as a Ctrl wheel) zoom around the pointer; any other wheel or
/// two-finger scroll pans, Shift turning a vertical wheel sideways. Every
/// wheel is consumed, so the page around the canvas neither scrolls nor
/// zooms.
fn canvas_wheel<N: Clone, P: Clone, E: Clone>(view: &GraphView<N, P, E>, w: &WheelEvent) -> bool {
    if w.modifiers.ctrl || w.modifiers.meta {
        view.zoom_around(w.zoom_factor(), canvas_point(view, w.x, w.y));
        return true;
    }
    let (dx, dy) = if w.modifiers.shift && w.delta_x == 0.0 {
        (w.delta_y, 0.0)
    } else {
        (w.delta_x, w.delta_y)
    };
    let vp = untracked(|| view.viewport());
    // A positive delta scrolls down / right: the content moves up / left.
    view.set_viewport(Viewport {
        x: vp.x - dx,
        y: vp.y - dy,
        zoom: vp.zoom,
    });
    true
}

fn canvas_key<N: Clone, P: Clone, E: Clone>(view: &GraphView<N, P, E>, k: &KeyEventData) -> bool {
    let cmd = k.ctrl || k.meta;
    match k.key.as_str() {
        "Delete" | "Backspace" => {
            view.delete_selected();
            true
        }
        "z" | "Z" if cmd && k.shift => view.redo() || true,
        "z" if cmd => view.undo() || true,
        "y" if cmd => view.redo() || true,
        "a" if cmd => {
            view.select_all();
            true
        }
        "Escape" => {
            if Drag::is_active() {
                Drag::cancel();
            } else {
                view.set_selection(Selection::default());
            }
            true
        }
        "+" | "=" => {
            view.zoom_by(1.2);
            true
        }
        "-" => {
            view.zoom_by(1.0 / 1.2);
            true
        }
        "0" if !cmd => {
            view.fit_view();
            true
        }
        "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" => {
            let step = if k.shift { 10.0 } else { 1.0 };
            let (dx, dy) = match k.key.as_str() {
                "ArrowLeft" => (-step, 0.0),
                "ArrowRight" => (step, 0.0),
                "ArrowUp" => (0.0, -step),
                _ => (0.0, step),
            };
            let nodes = untracked(|| view.selection()).nodes;
            if nodes.is_empty() {
                return false;
            }
            let _ = view.edit(|tx| tx.move_by(nodes, dx, dy));
            true
        }
        _ => false,
    }
}

// ---- nodes -----------------------------------------------------------------

fn build_node<N: Clone, P: Clone, E: Clone>(
    scope: &mut RenderScope,
    view: &GraphView<N, P, E>,
    id: NodeId,
    render: &dyn Fn(&mut RenderScope, NodeId) -> NodeHandle,
) -> NodeHandle {
    let frame = scope.create_element("div");
    frame.set_attribute("class", "rinch-graph-node");
    frame.set_attribute("data-node", &id.0.to_string());
    let frame_bounds = frame.bounds_signal();
    let Some(slot) = view.mount_node(id, frame_bounds) else {
        return frame;
    };

    {
        let f = frame.clone();
        scope.create_effect(move || {
            let Some(p) = slot.pos.try_get() else { return };
            f.set_styles(&[
                ("left", &format!("{}px", p.x)),
                ("top", &format!("{}px", p.y)),
            ]);
        });
    }
    {
        let f = frame.clone();
        scope.create_effect(move || {
            if slot.selected.try_get().unwrap_or(false) {
                f.add_class("selected");
            } else {
                f.remove_class("selected");
            }
        });
    }
    {
        // The frame's size in graph units, for fitting, marquee and wires.
        let v = *view;
        scope.create_effect(move || {
            let b = frame_bounds.get();
            let z = untracked(|| v.viewport().zoom);
            if b.width > 0.0 || b.height > 0.0 {
                slot.size
                    .set_if_changed(Size::new(b.width / z, b.height / z));
            }
        });
    }

    // Press: select, then drag the selection.
    {
        let v = *view;
        let handler = scope.register_handler(move || node_press(&v, id));
        frame.set_attribute("data-rid", &handler.0.to_string());
    }

    let body = render(scope, id);
    frame.append_child(&body);
    frame
}

fn node_press<N: Clone, P: Clone, E: Clone>(view: &GraphView<N, P, E>, id: NodeId) {
    let ctx = get_click_context();
    let additive = ctx.modifiers.shift || ctx.modifiers.ctrl || ctx.modifiers.meta;
    let mut sel = untracked(|| view.selection());
    if additive {
        if !sel.nodes.remove(&id) {
            sel.nodes.insert(id);
        }
        view.set_selection(sel);
        return;
    }
    let was_selected = sel.nodes.contains(&id);
    if !was_selected {
        sel = Selection::nodes([id]);
        view.set_selection(sel.clone());
    }
    let nodes: Vec<NodeId> = sel.nodes.iter().copied().collect();
    let starts: Vec<(NodeId, Point)> = nodes
        .iter()
        .filter_map(|n| Some((*n, untracked(|| view.node_slot(*n)?.pos.try_get())?)))
        .collect();
    let (sx, sy) = (ctx.mouse_x, ctx.mouse_y);
    let moved = Rc::new(Cell::new(false));
    let (moved_end, v, v_end, v_cancel) = (moved.clone(), *view, *view, *view);
    let (nodes_end, nodes_cancel) = (nodes.clone(), nodes);
    view.i().dragging.set(true);
    Drag::absolute()
        .on_move(move |x, y| {
            if !moved.get() && (x - sx).abs() + (y - sy).abs() < 3.0 {
                return;
            }
            moved.set(true);
            let zoom = untracked(|| v.viewport().zoom);
            let snap = v.i().config.borrow().snap;
            let (dx, dy) = ((x - sx) / zoom, (y - sy) / zoom);
            batch(|| {
                for (n, start) in &starts {
                    if let Some(slot) = v.node_slot(*n) {
                        slot.pos.set_if_changed(start.offset(dx, dy).snapped(snap));
                    }
                }
            });
        })
        .on_end(move |_, _| {
            if moved_end.get() {
                v_end.commit_drag(&nodes_end);
            } else {
                v_end.i().dragging.set(false);
                if was_selected {
                    // A click on one of several selected nodes picks it alone.
                    v_end.set_selection(Selection::nodes([id]));
                }
            }
        })
        .on_cancel(move |_, _| v_cancel.revert_drag(&nodes_cancel))
        .start();
}

// ---- ports -----------------------------------------------------------------

impl<N: Clone + 'static, P: Clone + 'static, E: Clone + 'static> GraphView<N, P, E> {
    /// The dot for `port`, to place inside a node body. Pressing it draws a
    /// wire; its position is measured, and wires attach to its centre. Style
    /// it through the `rinch-graph-port` class (plus `in`/`out`, and
    /// `source`/`candidate`/`target` while a wire is drawn).
    pub fn port_dot(&self, scope: &mut RenderScope, port: PortId) -> NodeHandle {
        let dot = scope.create_element("span");
        let dir = untracked(|| {
            self.i()
                .state
                .borrow()
                .graph()
                .port(port)
                .map(|p| (p.dir, p.node))
        });
        let Some((dir, node)) = dir else {
            return dot;
        };
        let side = if dir == PortDir::In { "in" } else { "out" };
        dot.set_attribute("data-port", &port.0.to_string());
        let slot = self.mount_port(port);

        {
            let v = *self;
            let d = dot.clone();
            scope.create_effect(move || {
                let hint = match v.port_hint(port) {
                    PortHint::Idle => "",
                    PortHint::Source => " source",
                    PortHint::Candidate => " candidate",
                    PortHint::Target => " target",
                };
                d.set_attribute("class", &format!("rinch-graph-port {side}{hint}"));
            });
        }
        {
            // Measure the dot's centre relative to its node's frame. The
            // frame is created before its ports, so its bounds are already
            // current when the dot's change arrives; it is read untracked.
            let v = *self;
            let dot_bounds = dot.bounds_signal();
            scope.create_effect(move || {
                let b = dot_bounds.get();
                if b.width <= 0.0 && b.height <= 0.0 {
                    return;
                }
                let frame = untracked(|| {
                    v.i()
                        .frames
                        .borrow()
                        .get(&node)
                        .copied()
                        .and_then(|f| f.try_get())
                });
                let Some(f) = frame else { return };
                let z = untracked(|| v.viewport().zoom);
                let off = Point::new(
                    (b.x + b.width / 2.0 - f.x) / z,
                    (b.y + b.height / 2.0 - f.y) / z,
                );
                slot.offset.set_if_changed(Some(off));
            });
        }
        {
            let v = *self;
            let handler = scope.register_handler(move || port_press(&v, port));
            dot.set_attribute("data-rid", &handler.0.to_string());
        }
        dot
    }
}

fn port_press<N: Clone + 'static, P: Clone + 'static, E: Clone + 'static>(
    view: &GraphView<N, P, E>,
    port: PortId,
) {
    let ctx = get_click_context();
    // Pressing an input that already has a wire picks that wire up: it can be
    // moved to another input, or dropped on empty canvas to delete it.
    let picked = untracked(|| {
        let inner = view.i();
        let state = inner.state.borrow();
        let g = state.graph();
        let p = g.port(port)?;
        if p.dir != PortDir::In {
            return None;
        }
        let e = *g.port_edges(port).last()?;
        Some((e, g.edge(e)?.from))
    });
    let (from, lifted) = match picked {
        Some((edge, from)) => (from, Some(edge)),
        None => (port, None),
    };

    let mut candidates = view.candidates_for(from);
    if let Some(edge) = lifted {
        // With the lifted wire gone, its old input is a candidate too.
        candidates.extend(candidates_without(view, edge, from));
    }
    let candidates = Rc::new(candidates);
    let start = untracked(|| view.port_point(from)).unwrap_or_default();
    batch(|| {
        view.i().wire_candidates.set(Some(candidates.clone()));
        view.i().wire_target.set(None);
        view.i().wire.set(Some(PendingWire {
            from,
            cursor: start,
        }));
    });

    let (sx, sy) = (ctx.mouse_x, ctx.mouse_y);
    let moved = Rc::new(Cell::new(false));
    let (moved_end, v, v_end, v_cancel) = (moved.clone(), *view, *view, *view);
    Drag::absolute()
        .on_move(move |x, y| {
            if !moved.get() && (x - sx).abs() + (y - sy).abs() < 3.0 {
                return;
            }
            moved.set(true);
            let vp = untracked(|| v.viewport());
            let g = vp.to_graph(canvas_point(&v, x, y));
            let radius = v.i().config.borrow().port_snap_radius / vp.zoom;
            let target = v.port_near(g, radius, &candidates);
            batch(|| {
                v.i().wire.set(Some(PendingWire { from, cursor: g }));
                v.i().wire_target.set_if_changed(target);
            });
        })
        .on_end(move |x, y| {
            let target = untracked(|| v_end.i().wire_target.get());
            let vp = untracked(|| v_end.viewport());
            let drop = vp.to_graph(canvas_point(&v_end, x, y));
            clear_wire(&v_end);
            if !moved_end.get() {
                return;
            }
            match (target, lifted) {
                (Some(t), Some(edge)) => {
                    let data = untracked(|| {
                        v_end
                            .i()
                            .state
                            .borrow()
                            .graph()
                            .edge(edge)
                            .map(|e| e.data.clone())
                    });
                    let inner = v_end.i();
                    let _ = v_end.edit(move |tx| {
                        tx.disconnect(edge)?;
                        match data {
                            Some(d) => tx.connect(&*inner.rules, from, t, d).map(|_| ()),
                            None => Ok(()),
                        }
                    });
                }
                (Some(t), None) => {
                    let _ = v_end.connect(from, t);
                }
                (None, Some(edge)) => {
                    let _ = v_end.edit(|tx| tx.disconnect(edge));
                }
                (None, None) => {
                    let cb = v_end.i().connect_to_empty.borrow().clone();
                    if let Some(cb) = cb {
                        cb(from, drop);
                    }
                }
            }
        })
        .on_cancel(move |_, _| clear_wire(&v_cancel))
        .start();
}

/// The ports `from` could connect to if `edge` were gone.
fn candidates_without<N: Clone, P: Clone, E: Clone>(
    view: &GraphView<N, P, E>,
    edge: EdgeId,
    from: PortId,
) -> HashSet<PortId> {
    let graph = view.i().state.borrow().graph().clone();
    let mut probe = rinch_graph_core::GraphState::from_graph(graph);
    if probe.edit(|tx| tx.disconnect(edge)).is_err() {
        return HashSet::new();
    }
    let g = probe.graph();
    g.ports()
        .filter(|p| rinch_graph_core::check_connect(g, &*view.i().rules, from, p.id).is_ok())
        .map(|p| p.id)
        .collect()
}

fn clear_wire<N, P, E>(view: &GraphView<N, P, E>) {
    batch(|| {
        view.i().wire.set(None);
        view.i().wire_target.set(None);
        view.i().wire_candidates.set(None);
    });
}

// ---- edges -----------------------------------------------------------------

fn build_edge_layer<N: Clone, P: Clone, E: Clone>(
    scope: &mut RenderScope,
    view: &GraphView<N, P, E>,
    world: &NodeHandle,
) {
    let svg = scope.create_element("svg");
    svg.set_attribute("class", "rinch-graph-edges");
    svg.set_attribute("preserveAspectRatio", "none");
    svg.set_attribute("fill", "none");
    {
        // Size the SVG to hold every node with room to spare, so the shapes
        // stay inside its box (damage and culling go by the box).
        let v = *view;
        let s = svg.clone();
        scope.create_effect(move || {
            let mut r = v.content_bounds().unwrap_or(Rect::new(0.0, 0.0, 1.0, 1.0));
            if let Some(w) = v.i().wire.get() {
                r = r.union(&Rect::new(w.cursor.x, w.cursor.y, 0.0, 0.0));
            }
            let m = 400.0;
            let (x, y, w, h) = (
                (r.x - m).floor(),
                (r.y - m).floor(),
                (r.width + 2.0 * m).ceil(),
                (r.height + 2.0 * m).ceil(),
            );
            s.set_attribute("viewBox", &format!("{x} {y} {w} {h}"));
            s.set_styles(&[
                ("left", &format!("{x}px")),
                ("top", &format!("{y}px")),
                ("width", &format!("{w}px")),
                ("height", &format!("{h}px")),
            ]);
        });
    }
    world.append_child(&svg);

    {
        let v = *view;
        let ids = v.i().edge_ids;
        for_each_dom_typed(
            scope,
            &svg,
            move || ids.get(),
            |id: &EdgeId| id.0.to_string(),
            move |id, item| build_edge(item, &v, id),
        );
    }

    // The wire being drawn.
    let wire = scope.create_element("path");
    wire.set_attribute("class", "rinch-graph-wire");
    {
        let v = *view;
        let w = wire.clone();
        scope.create_effect(move || {
            let Some(pending) = v.i().wire.get() else {
                w.set_attribute("d", "");
                return;
            };
            let cfg = v.config();
            let target = v.i().wire_target.get();
            let Some(from_pt) = v.port_point(pending.from) else {
                return;
            };
            let end = target
                .and_then(|t| v.port_point(t))
                .unwrap_or(pending.cursor);
            let from_is_out = untracked(|| {
                v.i()
                    .state
                    .borrow()
                    .graph()
                    .port(pending.from)
                    .is_some_and(|p| p.dir == PortDir::Out)
            });
            let (a, b) = if from_is_out {
                (from_pt, end)
            } else {
                (end, from_pt)
            };
            w.set_attribute("d", &edge_path(a, b, cfg.edge_shape, cfg.flow));
            w.set_attribute(
                "stroke",
                if target.is_some() {
                    "#40c057"
                } else {
                    "#868e96"
                },
            );
        });
    }
    wire.set_attribute("stroke-width", "2");
    wire.set_attribute("stroke-dasharray", "6 4");
    svg.append_child(&wire);
}

fn build_edge<N: Clone, P: Clone, E: Clone>(
    scope: &mut RenderScope,
    view: &GraphView<N, P, E>,
    id: EdgeId,
) -> NodeHandle {
    let path = scope.create_element("path");
    path.set_attribute("class", "rinch-graph-edge");
    path.set_attribute("data-edge", &id.0.to_string());
    path.set_attribute("stroke-linecap", "round");
    let slot = view.mount_edge(id);
    let v = *view;
    let p = path.clone();
    scope.create_effect(move || {
        let cfg = v.config();
        match v.edge_points(id) {
            Some((a, b)) => p.set_attribute("d", &edge_path(a, b, cfg.edge_shape, cfg.flow)),
            None => p.set_attribute("d", ""),
        }
    });
    let p = path.clone();
    scope.create_effect(move || {
        let selected = slot.selected.try_get().unwrap_or(false);
        p.set_attribute("stroke", if selected { "#228be6" } else { "#9aa3ad" });
        p.set_attribute("stroke-width", if selected { "3" } else { "2" });
    });
    path
}

// ---- controls --------------------------------------------------------------

fn build_controls<N: Clone, P: Clone, E: Clone>(
    scope: &mut RenderScope,
    view: &GraphView<N, P, E>,
) -> NodeHandle {
    let bar = scope.create_element("div");
    bar.set_attribute("class", "rinch-graph-controls");
    type Action<N, P, E> = Rc<dyn Fn(&GraphView<N, P, E>)>;
    let buttons: [(&str, &str, Action<N, P, E>); 3] = [
        ("+", "Zoom in", Rc::new(|v| v.zoom_by(1.2))),
        ("\u{2212}", "Zoom out", Rc::new(|v| v.zoom_by(1.0 / 1.2))),
        ("Fit", "Fit to view", Rc::new(|v| v.fit_view())),
    ];
    for (label, title, action) in buttons {
        let b = scope.create_element("button");
        b.set_attribute("class", "rinch-graph-control");
        b.set_attribute("title", title);
        b.append_child(&scope.create_text(label));
        let v = *view;
        let handler = scope.register_handler(move || action(&v));
        b.set_attribute("data-rid", &handler.0.to_string());
        bar.append_child(&b);
    }
    bar
}

#[cfg(test)]
mod wheel_tests {
    //! The canvas's wheel on an unmounted view: the canvas origin falls back to
    //! (0, 0), so the pointer is in canvas pixels.

    use super::*;
    use rinch_core::ModifierState;
    use rinch_graph_core::{AnyConnection, GraphState};

    type View = GraphView<&'static str, (), ()>;

    fn view() -> View {
        GraphView::new(GraphState::new(), AnyConnection)
    }

    fn mods(ctrl: bool, meta: bool, shift: bool) -> ModifierState {
        ModifierState {
            ctrl,
            meta,
            shift,
            ..ModifierState::default()
        }
    }

    #[test]
    fn a_plain_wheel_pans_and_is_consumed() {
        let v = view();
        assert!(canvas_wheel(&v, &WheelEvent::new(10.0, 40.0, 50.0, 50.0)));
        let vp = untracked(|| v.viewport());
        assert_eq!((vp.x, vp.y, vp.zoom), (-10.0, -40.0, 1.0));
    }

    #[test]
    fn shift_turns_a_vertical_wheel_sideways() {
        let v = view();
        let w = WheelEvent::new(0.0, 40.0, 0.0, 0.0).with_modifiers(mods(false, false, true));
        canvas_wheel(&v, &w);
        let vp = untracked(|| v.viewport());
        assert_eq!((vp.x, vp.y), (-40.0, 0.0));
    }

    #[test]
    fn ctrl_or_cmd_wheel_and_pinch_zoom_around_the_pointer() {
        for (ctrl, meta) in [(true, false), (false, true)] {
            let v = view();
            let anchor = Point::new(120.0, 80.0);
            let before = untracked(|| v.viewport()).to_graph(anchor);
            // Wheel turned away from the user (negative delta_y): zoom in.
            let w = WheelEvent::new(0.0, -50.0, anchor.x, anchor.y)
                .with_modifiers(mods(ctrl, meta, false));
            assert!(canvas_wheel(&v, &w));
            let vp = untracked(|| v.viewport());
            assert!(
                (vp.zoom - w.zoom_factor()).abs() < 1e-5 && vp.zoom > 1.0,
                "{}",
                vp.zoom
            );
            // The viewport is pixel-aligned, so the anchor holds to half a
            // screen pixel.
            let after = vp.to_graph(anchor);
            let tol = 0.5 / vp.zoom;
            assert!(
                (after.x - before.x).abs() <= tol && (after.y - before.y).abs() <= tol,
                "the point under the pointer stays put: {before:?} -> {after:?}"
            );
        }
    }
}
