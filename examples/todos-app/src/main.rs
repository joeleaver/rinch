use rinch::prelude::*;

#[derive(Clone, Debug, PartialEq)]
struct Todo {
    id: u32,
    text: String,
    completed: bool,
}

#[component]
fn app() -> NodeHandle {
    let todos = use_signal(|| Vec::<Todo>::new());
    let next_id = use_signal(|| 1u32);
    let input_text = use_signal(|| String::new());

    let add_todo = move || {
        let binding = input_text.get();
        let text = binding.trim();
        if !text.is_empty() {
            let id = next_id.get();
            next_id.update(|n| *n += 1);
            todos.update(|t| {
                t.push(Todo {
                    id,
                    text: text.to_string(),
                    completed: false,
                });
            });
            input_text.set(String::new());
        }
    };

    rsx! {
        div { style: "display: flex; flex-direction: column; padding: 20px; gap: 16px; width: 500px; margin: 0 auto;",
            Title { order: 1, "Todos" }

            div { style: "display: flex; gap: 8px;",
                TextInput {
                    value_fn: move || input_text.get(),
                    oninput: move |v| input_text.set(v),
                    placeholder: "What needs to be done?",
                    style: "flex: 1; padding: 8px; font-size: 16px",
                }
                Button {
                    onclick: move || add_todo(),
                    style: "padding: 8px 16px; font-size: 16px; background-color: #4CAF50; color: white",
                    "Add"
                }
            }

            for todo in todos.get() {
                let id = todo.id;
                let completed = todo.completed;
                let text = todo.text.clone();
                div {
                    key: todo.id,
                    style: "display: flex; align-items: center; gap: 12px; padding: 12px; background-color: #f5f5f5; border-radius: 4px",
                    Checkbox {
                        checked: completed,
                        onchange: move || {
                            todos.update(|t| {
                                if let Some(todo) = t.iter_mut().find(|t| t.id == id) {
                                    todo.completed = !todo.completed;
                                }
                            });
                        }
                    }
                    div {
                        style: {format!("flex: 1; font-size: 16px; {}",
                            if completed { "text-decoration: line-through; color: #999;" } else { "" }
                        )},
                        { text }
                    }
                    Button {
                        onclick: move || {
                            todos.update(|t| {
                                t.retain(|t| t.id != id);
                            });
                        },
                        style: "padding: 4px 8px; font-size: 14px; background-color: #f44336; color: white",
                        "Delete"
                    }
                }
            }

            if !todos.get().is_empty() {
                div { style: "font-size: 14px; color: #666",
                    {|| format!("{} items left", todos.get().iter().filter(|t| !t.completed).count())}
                }
            }
        }
    }
}

fn main() {
    rinch::run("Todos", 600, 500, app);
}
