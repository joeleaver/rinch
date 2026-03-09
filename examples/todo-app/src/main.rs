use rinch::prelude::*;

#[derive(Clone, Debug, PartialEq)]
struct Todo {
    id: u32,
    text: String,
    completed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Filter {
    All,
    Active,
    Completed,
}

#[derive(Clone, Copy)]
struct TodoStore {
    todos: Signal<Vec<Todo>>,
    input_text: Signal<String>,
    filter: Signal<Filter>,
    next_id: Signal<u32>,
}

impl TodoStore {
    fn new() -> Self {
        Self {
            todos: Signal::new(Vec::new()),
            input_text: Signal::new(String::new()),
            filter: Signal::new(Filter::All),
            next_id: Signal::new(1),
        }
    }

    fn add_todo(self) {
        let text = self.input_text.get();
        if !text.trim().is_empty() {
            let id = self.next_id.get();
            self.todos.update(|t| {
                t.push(Todo {
                    id,
                    text: text.trim().to_string(),
                    completed: false,
                })
            });
            self.next_id.set(id + 1);
            self.input_text.set(String::new());
        }
    }

    fn toggle_todo(self, id: u32) {
        self.todos.update(|list| {
            if let Some(t) = list.iter_mut().find(|t| t.id == id) {
                t.completed = !t.completed;
            }
        });
    }

    fn delete_todo(self, id: u32) {
        self.todos.update(|list| list.retain(|t| t.id != id));
    }

    fn clear_completed(self) {
        self.todos.update(|list| list.retain(|t| !t.completed));
    }

    fn set_filter(self, filter: Filter) {
        self.filter.set(filter);
    }

    fn set_input(self, value: String) {
        self.input_text.set(value);
    }

    fn filtered_todos(self) -> Vec<Todo> {
        let filter = self.filter.get();
        self.todos
            .get()
            .into_iter()
            .filter(|t| match filter {
                Filter::All => true,
                Filter::Active => !t.completed,
                Filter::Completed => t.completed,
            })
            .collect()
    }
}

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
                "x"
            }
        }
    }
}

#[component]
pub fn todo_input() -> NodeHandle {
    let store = use_store::<TodoStore>();

    rsx! {
        Group { gap: "sm",
            TextInput {
                placeholder: "What needs to be done?",
                value_fn: move || store.input_text.get(),
                oninput: move |value: String| store.set_input(value),
                onsubmit: move || store.add_todo(),
                style: "flex: 1;",
            }
            Button {
                variant: "filled",
                onclick: move || store.add_todo(),
                "Add"
            }
        }
    }
}

#[component]
pub fn filter_buttons() -> NodeHandle {
    let store = use_store::<TodoStore>();

    rsx! {
        Group { gap: "xs",
            Button {
                variant: {move || if store.filter.get() == Filter::All { "filled" } else { "light" }},
                size: "xs",
                onclick: move || store.set_filter(Filter::All),
                "All"
            }
            Button {
                variant: {move || if store.filter.get() == Filter::Active { "filled" } else { "light" }},
                size: "xs",
                onclick: move || store.set_filter(Filter::Active),
                "Active"
            }
            Button {
                variant: {move || if store.filter.get() == Filter::Completed { "filled" } else { "light" }},
                size: "xs",
                onclick: move || store.set_filter(Filter::Completed),
                "Completed"
            }
        }
    }
}

#[component]
pub fn todo_list() -> NodeHandle {
    let store = use_store::<TodoStore>();

    rsx! {
        Stack { gap: "0",
            for todo in store.filtered_todos() {
                let id = todo.id;
                TodoItem {
                    key: id,
                    label: todo.text.clone(),
                    completed: todo.completed,
                    on_toggle: move || store.toggle_todo(id),
                    on_delete: move || store.delete_todo(id),
                }
            }

            if store.todos.get().is_empty() {
                Text { color: "dimmed", style: "text-align: center; padding: 40px;",
                    "No todos yet. Add one above!"
                }
            }
        }
    }
}

#[component]
fn app() -> NodeHandle {
    let store = create_store(TodoStore::new());

    rsx! {
        Container { size: "sm",
            Stack { gap: "md", p: "lg",
                Title { order: 1, "Todo App" }
                Text { color: "dimmed", size: "sm",
                    {move || format!("{} items, {} completed",
                        store.todos.get().len(),
                        store.todos.get().iter().filter(|t| t.completed).count()
                    )}
                }

                todo_input {}
                filter_buttons {}
                todo_list {}

                if store.todos.get().iter().any(|t| t.completed) {
                    Button {
                        variant: "subtle",
                        color: "red",
                        size: "xs",
                        onclick: move || store.clear_completed(),
                        "Clear completed"
                    }
                }
            }
        }
    }
}

fn main() {
    let theme = ThemeProviderProps {
        primary_color: Some("blue".into()),
        default_radius: Some("md".into()),
        ..Default::default()
    };

    run_with_theme("Todo App", 500, 700, app, theme);
}
