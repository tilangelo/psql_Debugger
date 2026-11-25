use super::widgets::label;
use gtk::prelude::*;
use pg_debugger_gui::breakpoints::SourceMap;
use sourceview5::prelude::*;
use std::{cell::RefCell, collections::BTreeSet};

pub(crate) const POINT: &str = "breakpoint";
const CURRENT: &str = "current-line";

pub(crate) struct CodePane {
    pub source: sourceview5::Buffer,
    pub view: sourceview5::View,
    pub point_list: gtk::Label,
    pub feedback: gtk::Label,
    pub clear: gtk::Button,
    map: RefCell<Option<SourceMap>>,
    points: RefCell<BTreeSet<i32>>,
    hit: RefCell<BTreeSet<i32>>,
}

impl CodePane {
    pub fn new() -> Self {
        let source = sourceview5::Buffer::new(None);
        let view = sourceview5::View::with_buffer(&source);
        view.set_editable(false);
        view.set_monospace(true);
        view.set_show_line_numbers(true);
        view.set_show_line_marks(true);
        let point_style = sourceview5::MarkAttributes::new();
        point_style.set_icon_name("media-record-symbolic");
        view.set_mark_attributes(POINT, &point_style, 0);
        let line_style = sourceview5::MarkAttributes::new();
        line_style.set_background(&gtk::gdk::RGBA::new(0.93, 0.70, 0.13, 0.32));
        view.set_mark_attributes(CURRENT, &line_style, 1);
        let pane = Self {
            source,
            view,
            point_list: label(""),
            feedback: label(""),
            clear: gtk::Button::with_label("Снять все точки"),
            map: RefCell::new(None),
            points: RefCell::new(BTreeSet::new()),
            hit: RefCell::new(BTreeSet::new()),
        };
        pane.refresh();
        pane
    }
    pub fn load(&self, text: Option<&str>) {
        self.points.borrow_mut().clear();
        self.hit.borrow_mut().clear();
        self.feedback.set_label("");
        self.source
            .remove_source_marks(&self.source.start_iter(), &self.source.end_iter(), None);
        self.source.set_text(text.unwrap_or_default());
        self.map.replace(text.map(SourceMap::new));
        self.refresh();
    }
    pub fn points(&self) -> BTreeSet<i32> {
        self.points.borrow().clone()
    }
    pub fn has_point(&self, line: i32) -> bool {
        self.points.borrow().contains(&line)
    }
    pub fn has_points(&self) -> bool {
        !self.points.borrow().is_empty()
    }
    pub fn resolve(&self, line: i32) -> Result<i32, &'static str> {
        self.map
            .borrow()
            .as_ref()
            .ok_or("Выберите объект для отладки.")?
            .resolve(line)
    }
    pub fn set_point(&self, line: i32, enabled: bool) {
        let marks = self.source.source_marks_at_line(line - 1, Some(POINT));
        if enabled {
            self.points.borrow_mut().insert(line);
            if marks.is_empty() {
                if let Some(iter) = self.source.iter_at_line(line - 1) {
                    self.source.create_source_mark(None, POINT, &iter);
                }
            }
        } else {
            self.points.borrow_mut().remove(&line);
            for mark in marks {
                self.source.delete_mark(&mark);
            }
        }
        self.refresh();
    }
    pub fn begin_run(&self) {
        self.feedback.set_label("");
        self.hit.borrow_mut().clear();
        self.clear_current();
        self.refresh();
    }
    pub fn clear_current(&self) {
        self.source.remove_source_marks(
            &self.source.start_iter(),
            &self.source.end_iter(),
            Some(CURRENT),
        );
    }
    pub fn observe_stop(&self, line: i32) {
        if self.points.borrow().contains(&line) {
            self.hit.borrow_mut().insert(line);
            self.refresh();
        }
        if let Some(mut iter) = self.source.iter_at_line(line.saturating_sub(1)) {
            self.source.create_source_mark(None, CURRENT, &iter);
            self.view.scroll_to_iter(&mut iter, 0.12, false, 0.0, 0.0);
        }
    }
    fn refresh(&self) {
        let points = self.points.borrow();
        if points.is_empty() {
            self.point_list
                .set_label("Точки не заданы: при запуске остановка на входе.");
            return;
        }
        let hit = self.hit.borrow();
        let lines = points
            .iter()
            .map(|line| {
                format!(
                    "Строка {line} - {}",
                    if hit.contains(line) {
                        "остановка наблюдалась"
                    } else {
                        "задана, остановки ещё не было"
                    }
                )
            })
            .collect::<Vec<_>>();
        self.point_list.set_label(&lines.join("\n"));
    }
}
