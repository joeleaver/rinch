use rinch::prelude::*;

#[derive(Clone)]
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

    let todos_clone = todos.clone();
    let input_text_clone = input_text.clone();
    let next_id_clone = next_id.clone();

    let add_todo = move || {
        let binding = input_text_clone.get();
        let text = binding.trim();
        if !text.is_empty() {
            let id = next_id_clone.get();
            next_id_clone.update(|n| *n += 1);
            todos_clone.update(|t| {
                t.push(Todo {
                    id,
                    text: text.to_string(),
                    completed: false,
                });
            });
            input_text_clone.set(String::new());
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

            For {
                each: move || ForItem::from_iter(todos.get(), |t| t.id.to_string()),
                |item: &Todo| {
                    let todo = item.clone();
                    let todos_for_toggle = todos.clone();
                    let todos_for_delete = todos.clone();

                    rsx! {
                        div {
                            style: "display: flex; align-items: center; gap: 12px; padding: 12px; background-color: #f5f5f5; border-radius: 4px",
                            Checkbox {
                                checked: todo.completed,
                                onchange: move || {
                                    todos_for_toggle.update(|t| {
                                        if let Some(todo) = t.iter_mut().find(|t| t.id == todo.id) {
                                            todo.completed = !todo.completed;
                                        }
                                    });
                                }
                            }
                            div {
                                style: {format!("flex: 1; font-size: 16px; {}",
                                    if todo.completed { "text-decoration: line-through; color: #999;" } else { "" }
                                )},
                                { todo.text.clone() }
                            }
                            Button {
                                onclick: move || {
                                    todos_for_delete.update(|t| {
                                        t.retain(|t| t.id != todo.id);
                                    });
                                },
                                style: "padding: 4px 8px; font-size: 14px; background-color: #f44336; color: white",
                                "Delete"
                            }
                        }
                    }
                }
            }

            Show {
                when: {move || todos.get().len() > 0},
                div { style: "font-size: 14px; color: #666",
                    { format!("{} items left", todos.get().iter().filter(|t| !t.completed).count()) }
                }
            }
        }
    }
}

fn main() {
    rinch::run("Todos", 600, 500, app);
}
