use gtk::prelude::*;

pub(crate) fn entry(value: &str) -> gtk::Entry {
    gtk::Entry::builder().text(value).hexpand(true).build()
}

pub(crate) fn panel(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, spacing)
}

pub(crate) fn heading(text: &str, class: &str) -> gtk::Label {
    let widget = label(text);
    widget.add_css_class(class);
    widget
}

pub(crate) fn label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .build()
}

pub(crate) fn attach_field(grid: &gtk::Grid, row: i32, name: &str, widget: &impl IsA<gtk::Widget>) {
    let label = label(name);
    label.set_mnemonic_widget(Some(widget));
    grid.attach(&label, 0, row, 1, 1);
    grid.attach(widget, 1, row, 1, 1);
}
