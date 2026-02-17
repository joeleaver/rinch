use rinch::prelude::*;

// --- Data model ---

#[derive(Clone, Debug, Default, PartialEq)]
struct Todo {
    id: u32,
    text: String,
    completed: bool,
}

// --- Filter enum ---

#[derive(Clone, Copy, Debug, PartialEq)]
enum Filter {
    All,
    Active,
    Completed,
}

// --- Todo Item Component ---
// The docs show PascalCase #[component] generates a struct + Component impl.
// Parameters become struct fields (must be owned types).
// `children` is special, but we don't need it here.

#[component]
pub fn TodoItem(
    todo: Todo,
    on_toggle: Callback,
    on_delete: Callback,
) -> NodeHandle {
    rsx! {
        Group { gap: "sm", align: "center", style: "padding: 8px 0; border-bottom: 1px solid #eee;",
            Checkbox {
                checked: todo.completed,
                onchange: on_toggle,
            }
            Text {
                style: {
                    let completed = todo.completed;
                    move || if completed {
                        "flex: 1; text-decoration: line-through; color: #888;"
                    } else {
                        "flex: 1;"
                    }
                },
                {todo.text.clone()}
            }
            ActionIcon {
                variant: "subtle",
                color: "red",
                onclick: on_delete,
                "X"
            }
        }
    }
}

// --- Main App ---

#[component]
fn app() -> NodeHandle {
    let next_id = use_signal(|| 1u32);
    let todos = use_signal(|| Vec::<Todo>::new());
    let input_text = use_signal(|| String::new());
    let filter = use_signal(|| Filter::All);

    // Derived state
    let active_count = use_derived(move || {
        todos.get().iter().filter(|t| !t.completed).count()
    });

    let filtered_todos = use_derived(move || {
        let all = todos.get();
        match filter.get() {
            Filter::All => all,
            Filter::Active => all.into_iter().filter(|t| !t.completed).collect(),
            Filter::Completed => all.into_iter().filter(|t| t.completed).collect(),
        }
    });

    // Add todo handler
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

                // Status
                Text { size: "sm", color: "dimmed",
                    {|| format!("{} items remaining", active_count.get())}
                }

                // Todo list
                Paper { shadow: "xs", p: "md",
                    Stack { gap: "0",
                        for todo in filtered_todos.get() {
                            TodoItem {
                                key: todo.id,
                                todo: todo.clone(),
                                on_toggle: {
                                    let id = todo.id;
                                    move || todos.update(|t| {
                                        if let Some(item) = t.iter_mut().find(|i| i.id == id) {
                                            item.completed = !item.completed;
                                        }
                                    })
                                },
                                on_delete: {
                                    let id = todo.id;
                                    move || todos.update(|t| t.retain(|i| i.id != id))
                                },
                            }
                        }

                        // Empty state
                        if filtered_todos.get().is_empty() {
                            Center { p: "xl",
                                Text { color: "dimmed", "No todos yet. Add one above!" }
                            }
                        }
                    }
                }

                // Clear completed button
                if todos.get().iter().any(|t| t.completed) {
                    Button {
                        variant: "subtle",
                        color: "red",
                        size: "xs",
                        onclick: move || todos.update(|t| t.retain(|i| !i.completed)),
                        "Clear completed"
                    }
                }
            }
        }
    }
}

fn main() {
    run("Todo App", 500, 700, app);
}
