//! The connected outputs (monitors), live (issue #2414).
//!
//! `cargo run -p outputs` lists each output rinch sees: its name (what
//! `WindowProps::monitor` and, on Wayland, `WindowProps::output` take), its
//! logical position and size, and its scale. Plug a monitor in or out,
//! rotate one, or change its scale, and the table follows with no restart;
//! below it, every change since start is logged as `output_changes` reports
//! it. The header says whether this platform follows changes at all
//! (`output_tracking`).
//!
//! Each published list and each change is also printed on stdout
//! (`OUTPUTS …`, `CHANGE …`), which is what the live test of #2414 reads; see
//! `docs/internals/app-and-windows.md` "Outputs" for driving it in a nested
//! KWin.

use rinch::prelude::*;
use rinch_core::Effect;

fn describe(o: &Output) -> String {
    format!(
        "{} {}x{} at {},{} scale {}",
        o.name.as_deref().unwrap_or("(unnamed)"),
        o.size.0,
        o.size.1,
        o.position.0,
        o.position.1,
        o.scale
    )
}

fn describe_change(change: &OutputChange) -> String {
    match change {
        OutputChange::Added(o) => format!("added {}", describe(o)),
        OutputChange::Removed(o) => format!("removed {}", describe(o)),
        OutputChange::Changed { before, after } => {
            format!("changed {} -> {}", describe(before), describe(after))
        }
        other => format!("{other:?}"),
    }
}

#[component]
fn app() -> NodeHandle {
    let log: Signal<Vec<(usize, String)>> = Signal::new(Vec::new());

    // Keep the last list and log what changed: the effect re-runs whenever
    // the shell publishes a new one.
    let mut last: Vec<Output> = Vec::new();
    let mut next = 0;
    Effect::new(move || {
        let now = outputs();
        let tracking = output_tracking();
        println!(
            "OUTPUTS tracking={tracking:?} [{}]",
            now.iter().map(describe).collect::<Vec<_>>().join("; ")
        );
        let changes = output_changes(&last, &now);
        if !changes.is_empty() {
            let lines: Vec<(usize, String)> = changes
                .iter()
                .map(|c| {
                    let line = describe_change(c);
                    println!("CHANGE {line}");
                    next += 1;
                    (next, line)
                })
                .collect();
            untracked(|| log.update(|l| l.extend(lines)));
        }
        last = now;
    });

    rsx! {
        div {
            style: "font-family: sans-serif; padding: 16px; display: flex; flex-direction: column; gap: 12px;",
            h2 { style: "margin: 0;", "Outputs" }
            p {
                id: "tracking",
                {|| match output_tracking() {
                    OutputTracking::Live => "Changes are followed live.".to_string(),
                    OutputTracking::Snapshot => {
                        "Listed at start-up; this platform does not report changes.".to_string()
                    }
                    other => format!("{other:?}"),
                }}
            }
            table {
                id: "outputs",
                style: "border-collapse: collapse;",
                tr {
                    th { style: "text-align: left; padding: 4px 12px 4px 0;", "Name" }
                    th { style: "text-align: left; padding: 4px 12px 4px 0;", "Position" }
                    th { style: "text-align: left; padding: 4px 12px 4px 0;", "Size" }
                    th { style: "text-align: left; padding: 4px 12px 4px 0;", "Scale" }
                }
                for output in outputs() {
                    // Keyed by every field: a row whose output changed is
                        // rebuilt (a keyed row with an equal key is kept as is).
                    tr {
                        key: format!("{output:?}"),
                        class: "output",
                        td { style: "padding: 4px 12px 4px 0;", {output.name.clone().unwrap_or_else(|| "(unnamed)".into())} }
                        td { style: "padding: 4px 12px 4px 0;", {format!("{}, {}", output.position.0, output.position.1)} }
                        td { style: "padding: 4px 12px 4px 0;", {format!("{} x {}", output.size.0, output.size.1)} }
                        td { style: "padding: 4px 12px 4px 0;", {output.scale.to_string()} }
                    }
                }
            }
            h3 { style: "margin: 0;", "Changes" }
            ul {
                id: "changes",
                style: "margin: 0;",
                for (n, line) in log.get() {
                    li { key: n, {line.clone()} }
                }
            }
        }
    }
}

fn main() {
    App::new(app).title("Outputs").size(560, 360).run();
}

/// An app's own test of code that takes outputs: they are built with
/// `OutputId::from_raw` (#2658), which compiles in an app's test build.
#[cfg(test)]
mod tests {
    use super::*;

    fn output(id: u64, name: Option<&str>, width: f64) -> Output {
        Output {
            id: OutputId::from_raw(id),
            name: name.map(str::to_owned),
            position: (0.0, 0.0),
            size: (width, 1080.0),
            scale: 1.0,
        }
    }

    #[test]
    fn an_unnamed_output_rescaled_is_described_as_changed() {
        let before = [output(7, None, 1920.0), output(8, Some("DP-1"), 2560.0)];
        let after = [output(7, None, 1280.0), output(8, Some("DP-1"), 2560.0)];
        let lines: Vec<String> = output_changes(&before, &after)
            .iter()
            .map(describe_change)
            .collect();
        assert_eq!(
            lines,
            ["changed (unnamed) 1920x1080 at 0,0 scale 1 -> (unnamed) 1280x1080 at 0,0 scale 1"]
        );
    }
}
