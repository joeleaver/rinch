# Getting Started

This guide will walk you through creating your first rinch application.

## Prerequisites

- Rust 1.75 or later
- A C++ compiler (for native dependencies)

## Create a New Project

```bash
cargo new my-app
cd my-app
```

## Add Dependencies

Add rinch to your `Cargo.toml`:

```toml
[dependencies]
rinch = { path = "../path/to/rinch", features = ["desktop"] }
```

The `"desktop"` feature is required for windowing and rendering. Add `"widgets"` and `"theme"` for the full widget library and theme system:

```toml
[dependencies]
rinch = { path = "../path/to/rinch", features = ["desktop", "widgets", "theme"] }
```

## Write Your First App

Replace the contents of `src/main.rs`:

```rust
use rinch::prelude::*;

#[component]
fn app() -> NodeHandle {
    rsx! {
        div {
            h1 { "Hello, Rinch!" }
            p { "Welcome to your first rinch application." }
        }
    }
}

fn main() {
    run("My First App", 800, 600, app);
}
```

## Run Your App

```bash
cargo run
```

You should see a window appear with your content rendered inside.

## What's Next?

- Learn about [RSX Syntax](./rsx-syntax.md) for building UI
- Explore [Windows](./windows.md) for multi-window support
- Add [Menus](./menus.md) to your application
- Understand [Reactivity](./reactivity.md) for dynamic state
