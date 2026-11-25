use super::widgets::{entry, label};
use gtk::prelude::*;
use pg_debugger_gui::catalog::Argument;
use std::cell::RefCell;
type ArgumentInput = Option<(gtk::Entry, gtk::CheckButton)>;

pub(crate) struct ArgumentsPane {
    pub root: gtk::Box,
    inputs: RefCell<Vec<ArgumentInput>>,
}
impl ArgumentsPane {
    pub fn new() -> Self {
        Self {
            root: gtk::Box::new(gtk::Orientation::Vertical, 8),
            inputs: RefCell::new(Vec::new()),
        }
    }
    pub fn load(&self, arguments: Option<&[Argument]>) {
        self.inputs.borrow_mut().clear();
        while let Some(child) = self.root.first_child() {
            self.root.remove(&child);
        }
        let Some(arguments) = arguments else {
            self.root.append(&label("Выберите функцию или процедуру."));
            return;
        };
        if arguments.is_empty() {
            self.root.append(&label("У объекта нет аргументов."));
        }
        for (index, argument) in arguments.iter().enumerate() {
            let (field, input) = argument_field(index + 1, argument);
            self.root.append(&field);
            self.inputs.borrow_mut().push(input);
        }
    }
    pub fn values(&self) -> Vec<Option<String>> {
        self.inputs
            .borrow()
            .iter()
            .map(|input| {
                input
                    .as_ref()
                    .and_then(|(entry, null)| (!null.is_active()).then(|| entry.text().to_string()))
            })
            .collect()
    }
    pub fn set_value(&self, index: usize, value: &str) -> bool {
        if let Some(Some((input, _))) = self.inputs.borrow().get(index) {
            input.set_text(value);
            true
        } else {
            false
        }
    }
}
fn argument_field(index: usize, argument: &Argument) -> (gtk::Box, ArgumentInput) {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let name = argument
        .name
        .clone()
        .unwrap_or_else(|| format!("Аргумент {index}"));
    column.append(&label(&format!(
        "{name} · {} · {}",
        argument.type_name, argument.mode
    )));
    if matches!(argument.mode.as_str(), "OUT" | "TABLE") {
        column.append(&label("Выходное значение."));
    } else if argument.accepts_input() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let input = entry("");
        let null = gtk::CheckButton::with_label("Передать NULL");
        let hint = label("");
        hint.add_css_class("error");
        let argument = argument.clone();
        let weak_hint = hint.downgrade();
        input.connect_changed(move |input| {
            let Some(hint) = weak_hint.upgrade() else {
                return;
            };
            let text = input.text();
            hint.set_label(if text.is_empty() || argument.value_is_valid(&text) {
                ""
            } else {
                "Проверьте формат значения для указанного типа."
            });
        });
        let weak_input = input.downgrade();
        let weak_hint = hint.downgrade();
        null.connect_toggled(move |null| {
            if let Some(input) = weak_input.upgrade() {
                input.set_sensitive(!null.is_active());
            }
            if let Some(hint) = weak_hint.upgrade() {
                hint.set_visible(!null.is_active());
            }
        });
        row.append(&input);
        row.append(&null);
        column.append(&row);
        column.append(&hint);
        return (column, Some((input, null)));
    } else {
        column.append(&label(
            "Ввод для этого типа или режима пока не поддерживается.",
        ));
    }
    (column, None)
}
