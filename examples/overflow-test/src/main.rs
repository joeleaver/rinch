use rinch::prelude::*;

// Issue #32 repro: closure parameter inside a for-loop iter expression.
// Previously failed with "cannot find value `b` in this scope" because the
// shadow-locals scanner emitted `let b = b.clone();` for the closure param.
#[component]
fn ruler() -> NodeHandle {
    let total_bars: u32 = 16;
    rsx! {
        div { style: "padding: 16px; display: flex; gap: 8px",
            // The closure param `b` is the only `b` in scope. Pre-fix: macro
            // emitted `let b = b.clone();` for the for-loop iter, which failed
            // to compile since no outer `b` exists.
            for bar in (0..total_bars).filter(|b| b % 4 == 0).collect::<Vec<u32>>() {
                span { key: bar.to_string(),
                    style: "padding: 4px 8px; background: #ddd; border-radius: 4px",
                    {bar.to_string()}
                }
            }
        }
    }
}

// Also exercise tuple-destructure closure params + a non-Copy capture in the
// same iter expr (the original #26 fix still works for real captures).
#[component]
fn pairs() -> NodeHandle {
    let prefix = String::from("pair");
    let data: Vec<(i32, i32)> = vec![(1, 2), (3, 4), (5, 6), (7, 4)];
    rsx! {
        div { style: "padding: 16px; display: flex; flex-direction: column; gap: 4px",
            for pair in data.iter().filter(|(a, b)| a < b).cloned().collect::<Vec<_>>() {
                div { key: format!("{:?}", pair),
                    {format!("{}: {:?}", prefix, pair)}
                }
            }
        }
    }
}

#[component]
fn app() -> NodeHandle {
    rsx! {
        div { style: "font-family: system-ui; padding: 16px",
            div { "Ruler (every 4th of 16 bars):" }
            {ruler(__scope)}
            div { "Pairs (a < b filter, tuple destructure, captured prefix):" }
            {pairs(__scope)}
        }
    }
}

fn main() {
    App::new(app).title("issue #32 repro").size(600, 300).run();
}
