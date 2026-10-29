//! The canvas's built-in stylesheet. Colours read `--rinch-graph-*` custom
//! properties, so an app re-themes the canvas without replacing the sheet.

pub(crate) const CANVAS_CSS: &str = r#"
.rinch-graph {
  position: relative;
  overflow: hidden;
  width: 100%;
  height: 100%;
  background-color: var(--rinch-graph-bg, #f5f6f8);
  outline: none;
}
.rinch-graph-world {
  position: absolute;
  left: 0;
  top: 0;
  width: 0;
  height: 0;
  transform-origin: 0 0;
}
.rinch-graph-edges {
  position: absolute;
  overflow: visible;
  pointer-events: none;
}
.rinch-graph-node {
  position: absolute;
  box-sizing: border-box;
  background-color: var(--rinch-graph-node-bg, #ffffff);
  border: 1px solid var(--rinch-graph-node-border, #ced4da);
  border-radius: 8px;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.08);
  white-space: nowrap;
}
.rinch-graph-node.selected {
  border-color: var(--rinch-graph-accent, #228be6);
  box-shadow: 0 0 0 2px var(--rinch-graph-accent-soft, rgba(34, 139, 230, 0.3));
}
.rinch-graph-port {
  display: inline-block;
  width: 12px;
  height: 12px;
  box-sizing: border-box;
  border-radius: 6px;
  background-color: var(--rinch-graph-port-bg, #ffffff);
  border: 2px solid var(--rinch-graph-port, #868e96);
  vertical-align: middle;
}
.rinch-graph-port.source {
  background-color: var(--rinch-graph-accent, #228be6);
  border-color: var(--rinch-graph-accent, #228be6);
}
.rinch-graph-port.candidate {
  border-color: var(--rinch-graph-ok, #40c057);
}
.rinch-graph-port.target {
  background-color: var(--rinch-graph-ok, #40c057);
  border-color: var(--rinch-graph-ok, #40c057);
}
.rinch-graph-marquee {
  position: absolute;
  display: none;
  box-sizing: border-box;
  border: 1px dashed var(--rinch-graph-accent, #228be6);
  background-color: var(--rinch-graph-accent-faint, rgba(34, 139, 230, 0.08));
  pointer-events: none;
}
.rinch-graph-controls {
  position: absolute;
  left: 12px;
  bottom: 12px;
  display: flex;
  gap: 4px;
}
.rinch-graph-control {
  min-width: 32px;
  height: 28px;
  padding: 0 8px;
  border: 1px solid var(--rinch-graph-node-border, #ced4da);
  border-radius: 6px;
  background-color: var(--rinch-graph-node-bg, #ffffff);
  color: #343a40;
  font-size: 14px;
}
"#;
