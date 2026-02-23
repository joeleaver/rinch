use rinch::prelude::*;

// === ATTEMPT 1: Writing this as a newcomer based purely on documentation ===
//
// FINDINGS LOG - I'll document every surprise, mistake, and friction point here.
//
// Each finding is tagged: [FINDING-N] description
//

// My todo item struct
#[derive(Clone, Debug, PartialEq)]
struct Todo {
    id: u32,
    text: String,
    completed: bool,
}

// [FINDING-1] The docs show `#[component]` on a PascalCase function to create
// a reusable component with props. Let me try making a TodoItem component.
// The docs say: children: &[NodeHandle] is special, Callback has a default (no-op),
// and props are owned types (String, bool, etc.)
#[component]
pub fn TodoItem(
    label: String,
    completed: bool,
    on_toggle: Callback,
    on_delete: Callback,
) -> NodeHandle {
    rsx! {
        Group { gap: "sm", align: "center",
            style: "padding: 8px 0; border-bottom: 1px solid var(--rinch-color-gray-3);",
            Checkbox {
                checked: completed,
                onchange: on_toggle,
            }
            Text {
                style: {
                    let completed = completed;
                    move || if completed {
                        "text-decoration: line-through; color: var(--rinch-color-dimmed);"
                    } else {
                        ""
                    }
                },
                {label.clone()}
            }
            ActionIcon {
                variant: "subtle",
                color: "red",
                onclick: on_delete,
                "×"
            }
        }
    }
}

// Filter enum for showing all/active/completed
#[derive(Clone, Copy, Debug, PartialEq)]
enum Filter {
    All,
    Active,
    Completed,
}

#[component]
fn app() -> NodeHandle {
    // State
    let todos = Signal::new(Vec::<Todo>::new());
    let input_text = Signal::new(String::new());
    let filter = Signal::new(Filter::All);
    let next_id = Signal::new(1u32);

    // Add todo handler - shared between TextInput onsubmit and Button onclick
    let add_todo = move || {
        let text = input_text.get();
        if !text.trim().is_empty() {
            let id = next_id.get();
            todos.update(|t| t.push(Todo {
                id,
                text: text.trim().to_string(),
                completed: false,
            }));
            next_id.set(id + 1);
            input_text.set(String::new());
        }
    };

    rsx! {
        Container { size: "sm",
            Stack { gap: "md", p: "lg",
                // Header
                Title { order: 1, "Todo App" }
                Text { color: "dimmed", size: "sm",
                    {|| format!("{} items, {} completed",
                        todos.get().len(),
                        todos.get().iter().filter(|t| t.completed).count()
                    )}
                }

                // Input row
                Group { gap: "sm",
                    TextInput {
                        placeholder: "What needs to be done?",
                        value_fn: move || input_text.get(),
                        oninput: move |value: String| input_text.set(value),
                        onsubmit: add_todo,
                        style: "flex: 1;",
                    }
                    Button {
                        variant: "filled",
                        onclick: add_todo,
                        "Add"
                    }
                }

                // Filter buttons
                Group { gap: "xs",
                    Button {
                        variant: {|| if filter.get() == Filter::All { "filled" } else { "light" }},
                        size: "xs",
                        onclick: move || filter.set(Filter::All),
                        "All"
                    }
                    Button {
                        variant: {|| if filter.get() == Filter::Active { "filled" } else { "light" }},
                        size: "xs",
                        onclick: move || filter.set(Filter::Active),
                        "Active"
                    }
                    Button {
                        variant: {|| if filter.get() == Filter::Completed { "filled" } else { "light" }},
                        size: "xs",
                        onclick: move || filter.set(Filter::Completed),
                        "Completed"
                    }
                }

                // Todo list
                // [FINDING-2] The docs say for loops work with Clone + PartialEq + 'static items.
                // My Todo struct derives those. Let me try filtering inline.
                // [FINDING-3] I'm not sure if I can do .filter() inside a for loop expression.
                // The docs show `for todo in todos.get()` directly. Let me try filtering first.
                Stack { gap: "0",
                    for todo in todos.get().into_iter().filter(|t| {
                        match filter.get() {
                            Filter::All => true,
                            Filter::Active => !t.completed,
                            Filter::Completed => t.completed,
                        }
                    }).collect::<Vec<_>>() {
                        // [FINDING-4] The docs show TodoItem component usage like:
                        // TodoItem { label: "text", completed: false, on_toggle: || ..., on_delete: || ... }
                        // But I need to capture `todo.id` for the callbacks.
                        let id = todo.id;
                        TodoItem {
                            key: id,
                            label: todo.text.clone(),
                            completed: todo.completed,
                            on_toggle: move || {
                                todos.update(|list| {
                                    if let Some(t) = list.iter_mut().find(|t| t.id == id) {
                                        t.completed = !t.completed;
                                    }
                                });
                            },
                            on_delete: move || {
                                todos.update(|list| list.retain(|t| t.id != id));
                            },
                        }
                    }

                    // Empty state
                    if todos.get().is_empty() {
                        Text { color: "dimmed", style: "text-align: center; padding: 40px;",
                            "No todos yet. Add one above!"
                        }
                    }
                }

                // Clear completed button
                if todos.get().iter().any(|t| t.completed) {
                    Button {
                        variant: "subtle",
                        color: "red",
                        size: "xs",
                        onclick: move || todos.update(|list| list.retain(|t| !t.completed)),
                        "Clear completed"
                    }
                }
            }
        }
    }
}

fn main() {
    // [FINDING-5] The docs show two ways to run:
    // - run("title", w, h, app)  -- simple
    // - run_with_theme("title", w, h, app, theme)  -- with theme
    // I want a nice look, so let me use run_with_theme.
    let theme = ThemeProviderProps {
        primary_color: Some("blue".into()),
        default_radius: Some("md".into()),
        ..Default::default()
    };

    run_with_theme("Todo App", 500, 700, app, theme);
}
