use gtk::glib::{self, ControlFlow};
use gtk::prelude::*;
use pg_debugger::demo_linux::DemoServer;
use pg_debugger_gui::catalog::{Catalog, Connection, DebuggerStatus, Selection, browse};
use pg_debugger_gui::outcome::{CallOutcome, SessionOutcome};
use pg_debugger_gui::session::{Call, Command, Event, Session};
use pg_debugger_gui::ui_state::{Action, UiState};
use sourceview5::prelude::*;
use std::cell::{Cell, RefCell};
#[cfg(test)]
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::mpsc::{TryRecvError, channel};
use std::time::Duration;

mod ui;
#[cfg(test)]
use ui::code::POINT;
use ui::{arguments::ArgumentsPane, code::CodePane, widgets::*};

struct Screen {
    fields: gtk::Grid,
    host: gtk::Entry,
    port: gtk::Entry,
    database_input: gtk::Entry,
    user: gtk::Entry,
    password: gtk::PasswordEntry,
    connect: gtk::Button,
    database: gtk::ComboBoxText,
    schema: gtk::ComboBoxText,
    routine: gtk::ListBox,
    search: gtk::SearchEntry,
    visible_routines: RefCell<Vec<u32>>,
    pages: gtk::Stack,
    connect_panel: gtk::Box,
    demo_button: gtk::Button,
    own_button: gtk::Button,
    demo: RefCell<Option<(DemoServer, Connection)>>,
    arguments: ArgumentsPane,
    editor: CodePane,
    request_status: gtk::Label,
    debugger_status: gtk::Label,
    error_detail: gtk::Label,
    details: gtk::Expander,
    rendering: Cell<bool>,
    state: Cell<UiState>,
    current: RefCell<Option<(Connection, Catalog)>>,
    session: RefCell<Option<Session>>,
    close_after_stop: Cell<bool>,
    catalog_message: RefCell<Option<String>>,
    window: glib::WeakRef<gtk::ApplicationWindow>,
    start: gtk::Button,
    step: gtk::Button,
    resume: gtk::Button,
    stop: gtk::Button,
    execution: gtk::Label,
    inspection: gtk::Label,
    inspector: gtk::Notebook,
    result: gtk::Label,
    object_title: gtk::Label,
}

fn main() -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("io.github.tilangelo.PgDebugger")
        .build();
    app.connect_activate(|app| {
        if let Some(window) = app.active_window() {
            window.present();
        } else {
            Screen::show(app);
        }
    });
    app.run()
}

impl Screen {
    fn show(app: &gtk::Application) -> Rc<Self> {
        let css = gtk::CssProvider::new();
        css.load_from_data(".routine-list row:selected:disabled { background: #204e72; } .routine-list row:selected:disabled label { color: #ffffff; opacity: 1; }");
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        let fields = gtk::Grid::builder()
            .column_spacing(12)
            .row_spacing(8)
            .build();
        let host = entry("127.0.0.1");
        let port = entry("5432");
        let database_input = entry("postgres");
        let user = entry("postgres");
        let password = gtk::PasswordEntry::builder()
            .show_peek_icon(true)
            .hexpand(true)
            .build();
        attach_field(&fields, 0, "Адрес", &host);
        attach_field(&fields, 1, "Порт", &port);
        attach_field(&fields, 2, "База данных", &database_input);
        attach_field(&fields, 3, "Пользователь", &user);
        attach_field(&fields, 4, "Пароль", &password);
        let connect = gtk::Button::with_label("Проверить подключение");
        connect.add_css_class("suggested-action");
        fields.attach(&connect, 1, 5, 1, 1);

        let request_status = label("Выберите, с какой базы начать.");
        let debugger_status = label("");
        let error_detail = label("");
        error_detail.set_selectable(true);
        let details = gtk::Expander::builder()
            .label("Подробности PostgreSQL")
            .child(&error_detail)
            .visible(false)
            .build();

        let editor = CodePane::new();
        let view = editor.view.clone();
        let source_scroll = gtk::ScrolledWindow::builder()
            .min_content_height(160)
            .vexpand(true)
            .child(&view)
            .build();

        let database = gtk::ComboBoxText::new();
        let schema = gtk::ComboBoxText::new();
        let routine = gtk::ListBox::new();
        routine.set_selection_mode(gtk::SelectionMode::Single);
        routine.add_css_class("boxed-list");
        routine.add_css_class("routine-list");
        routine.set_placeholder(Some(&heading("Подходящих объектов нет.", "dim-label")));
        let search = gtk::SearchEntry::builder()
            .placeholder_text("Имя функции или процедуры")
            .build();
        for selector in [&database, &schema] {
            selector.set_hexpand(true);
        }
        let selectors = gtk::Grid::builder()
            .column_spacing(12)
            .row_spacing(8)
            .build();
        attach_field(&selectors, 0, "База данных", &database);
        attach_field(&selectors, 1, "Схема", &schema);
        let objects_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&routine)
            .build();

        let arguments = ArgumentsPane::new();
        let args_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&arguments.root)
            .build();

        let start = gtk::Button::with_label("Начать отладку");
        start.add_css_class("suggested-action");
        let step = gtk::Button::with_label("Шаг");
        step.set_tooltip_text(Some(
            "Выполнить текущий оператор и остановиться перед следующим.",
        ));
        let resume = gtk::Button::with_label("Продолжить");
        resume.set_tooltip_text(Some("Выполнять до следующей точки или конца вызова."));
        let stop = gtk::Button::with_label("Остановить");
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        for button in [&start, &step, &resume, &stop] {
            actions.append(button);
        }
        let execution = label("Выберите объект для отладки.");
        let inspection = label("Переменные и стек появятся на точке остановки.");
        inspection.set_selectable(true);
        inspection.set_valign(gtk::Align::Start);
        inspection.set_margin_top(10);
        inspection.set_margin_start(10);
        inspection.set_margin_end(10);
        let inspection_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&inspection)
            .build();
        let inspector = gtk::Notebook::new();
        inspector.append_page(&args_scroll, Some(&gtk::Label::new(Some("Аргументы"))));
        inspector.append_page(
            &inspection_scroll,
            Some(&gtk::Label::new(Some("Состояние"))),
        );
        let point_list = editor.point_list.clone();
        let point_feedback = editor.feedback.clone();
        let clear_points = editor.clear.clone();
        let point_panel = panel(10);
        point_panel.set_margin_top(10);
        point_panel.set_margin_start(10);
        point_panel.set_margin_end(10);
        point_panel.append(&point_list);
        point_panel.append(&clear_points);
        point_panel.append(&label("Щелчок в полосе слева от кода ставит или снимает точку. Внутри многострочного оператора точка переносится к его началу. Она сработает, только если выполнение дойдёт до оператора."));
        let point_scroll = gtk::ScrolledWindow::builder().child(&point_panel).build();
        inspector.append_page(&point_scroll, Some(&gtk::Label::new(Some("Точки"))));
        inspector.set_width_request(225);
        let result = label("");
        result.set_selectable(true);
        result.set_valign(gtk::Align::Start);
        let result_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .max_content_height(106)
            .propagate_natural_height(true)
            .child(&result)
            .build();
        let object_title = heading("Исходный код", "title-4");

        let library = panel(8);
        library.set_width_request(214);
        library.append(&heading("Объекты базы", "title-4"));
        library.append(&selectors);
        library.append(&search);
        library.append(&objects_scroll);
        let code = panel(8);
        code.set_hexpand(true);
        code.append(&object_title);
        code.append(&source_scroll);
        code.append(&point_feedback);
        code.append(&execution);
        code.append(&result_scroll);
        let columns = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        columns.set_vexpand(true);
        columns.append(&library);
        columns.append(&code);
        columns.append(&inspector);
        let workspace = gtk::Box::new(gtk::Orientation::Vertical, 10);
        workspace.append(&actions);
        workspace.append(&columns);

        let welcome = panel(12);
        welcome.set_halign(gtk::Align::Center);
        welcome.set_valign(gtk::Align::Start);
        welcome.set_margin_top(38);
        welcome.append(&heading("Отладка PL/pgSQL", "title-1"));
        welcome.append(&label("Тестовый пример открывает готовую функцию. Для своих функций и процедур подключитесь к локальной базе."));
        let connect_panel = panel(8);
        connect_panel.set_visible(false);
        connect_panel.append(&heading("Подключение к своей базе", "title-4"));
        connect_panel.append(&fields);
        welcome.append(&connect_panel);
        let pages = gtk::Stack::new();
        pages.set_hhomogeneous(false);
        pages.set_vhomogeneous(false);
        pages.set_vexpand(true);
        pages.add_named(&welcome, Some("welcome"));
        pages.add_named(&workspace, Some("workspace"));
        pages.set_visible_child_name("welcome");
        let demo_button = gtk::Button::with_label("Запустить тестовый пример");
        demo_button.add_css_class("suggested-action");
        let own_button = gtk::Button::with_label("Подключиться к своей базе");
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        toolbar.append(&demo_button);
        toolbar.append(&own_button);
        let content = panel(9);
        content.set_margin_top(14);
        content.set_margin_bottom(14);
        content.set_margin_start(14);
        content.set_margin_end(14);
        content.append(&toolbar);
        content.append(&request_status);
        content.append(&debugger_status);
        content.append(&details);
        content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        content.append(&pages);

        let screen = Rc::new(Self {
            fields,
            host,
            port,
            database_input,
            user,
            password,
            connect,
            database,
            schema,
            routine,
            search,
            visible_routines: RefCell::new(Vec::new()),
            pages,
            connect_panel,
            demo_button,
            own_button,
            demo: RefCell::new(None),
            arguments,
            editor,
            request_status,
            debugger_status,
            error_detail,
            details,
            rendering: Cell::new(false),
            state: Cell::new(UiState::Idle),
            current: RefCell::new(None),
            session: RefCell::new(None),
            close_after_stop: Cell::new(false),
            catalog_message: RefCell::new(None),
            window: glib::WeakRef::new(),
            start,
            step,
            resume,
            stop,
            execution,
            inspection,
            inspector,
            result,
            object_title,
        });
        screen.refresh_controls();
        screen.wire();
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("PostgreSQL Debugger")
            .default_width(1080)
            .default_height(680)
            .child(&content)
            .build();
        screen.window.set(Some(&window));
        let state = Rc::clone(&screen);
        window.connect_close_request(move |_| {
            if state.state.get() == UiState::ClosingDemo {
                glib::Propagation::Stop
            } else if state.session.borrow().is_some() {
                state.close_after_stop.set(true);
                state.stop_debug();
                glib::Propagation::Stop
            } else if matches!(
                state.state.get(),
                UiState::StartingDemo | UiState::LoadingCatalog
            ) {
                state.close_after_stop.set(true);
                state
                    .request_status
                    .set_label("Ждём завершения текущей загрузки…");
                glib::Propagation::Stop
            } else {
                if state.demo.borrow().is_some() {
                    state.shutdown_demo();
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        });
        window.present();
        screen
    }

    fn wire(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.editor.clear.connect_clicked(move |_| {
            if let Some(screen) = weak.upgrade() {
                let lines = screen.editor.points();
                for line in lines {
                    screen.set_point(line, false);
                }
                screen
                    .editor
                    .feedback
                    .set_label("Все точки сняты. При следующем запуске - остановка на входе.");
            }
        });
        let weak = Rc::downgrade(self);
        self.demo_button.connect_clicked(move |_| {
            if let Some(screen) = weak.upgrade() {
                screen.start_demo();
            }
        });
        let weak = Rc::downgrade(self);
        self.own_button.connect_clicked(move |_| {
            if let Some(screen) = weak.upgrade() {
                screen.open_connection();
            }
        });
        let weak = Rc::downgrade(self);
        self.start.connect_clicked(move |_| {
            if let Some(screen) = weak.upgrade() {
                screen.start_debug();
            }
        });
        let weak = Rc::downgrade(self);
        self.step.connect_clicked(move |_| {
            if let Some(screen) = weak.upgrade() {
                screen.resume_debug(Command::Step);
            }
        });
        let weak = Rc::downgrade(self);
        self.resume.connect_clicked(move |_| {
            if let Some(screen) = weak.upgrade() {
                screen.resume_debug(Command::Continue);
            }
        });
        let weak = Rc::downgrade(self);
        self.stop.connect_clicked(move |_| {
            if let Some(screen) = weak.upgrade() {
                screen.stop_debug();
            }
        });
        let weak = Rc::downgrade(self);
        self.editor
            .view
            .connect_line_mark_activated(move |_, iter, button, _, presses| {
                if button == 1 && presses == 1 {
                    if let Some(screen) = weak.upgrade() {
                        screen.toggle_point(iter);
                    }
                }
            });
        let weak = Rc::downgrade(self);
        self.connect.connect_clicked(move |_| {
            let Some(screen) = weak.upgrade() else { return };
            screen.connect_from_form();
        });
        let weak = Rc::downgrade(self);
        self.database.connect_changed(move |widget| {
            if let Some(screen) = weak.upgrade() {
                if let Some(database) = widget.active_id() {
                    screen.change_database(&database);
                }
            }
        });
        let weak = Rc::downgrade(self);
        self.schema.connect_changed(move |widget| {
            if let Some(screen) = weak.upgrade() {
                if let Some(schema) = widget.active_id() {
                    screen.change_selection(Selection {
                        schema: Some(schema.to_string()),
                        routine_oid: None,
                    });
                }
            }
        });
        let weak = Rc::downgrade(self);
        self.routine.connect_row_selected(move |_, row| {
            if let Some(screen) = weak.upgrade() {
                if screen.rendering.get() || screen.state.get() != UiState::Idle {
                    return;
                }
                let oid = row.and_then(|row| {
                    screen
                        .visible_routines
                        .borrow()
                        .get(row.index() as usize)
                        .copied()
                });
                if let Some(oid) = oid {
                    screen.change_selection(Selection {
                        schema: screen.schema.active_id().map(|id| id.to_string()),
                        routine_oid: Some(oid),
                    });
                }
            }
        });
        let weak = Rc::downgrade(self);
        self.search.connect_search_changed(move |_| {
            if let Some(screen) = weak.upgrade() {
                let state = screen.current.borrow();
                if let Some((_, catalog)) = state.as_ref() {
                    screen.render_routines(catalog);
                }
            }
        });
    }

    fn open_connection(&self) {
        self.pages.set_visible_child_name("welcome");
        self.connect_panel.set_visible(true);
        self.clear_details();
        self.debugger_status.set_label("");
        self.request_status
            .set_label("Введи параметры своего локального PostgreSQL.");
        self.host.grab_focus();
    }

    fn shutdown_demo(self: &Rc<Self>) {
        let Some((mut server, connection)) = self.demo.borrow_mut().take() else {
            return;
        };
        self.transition(Action::CloseDemo);
        self.request_status
            .set_label("Останавливаем тестовый сервер…");
        let (send, receive) = channel();
        std::thread::spawn(move || {
            let result = server.shutdown().map_err(|error| error.to_string());
            let _ = send.send((server, connection, result));
        });
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(25), move || {
            let Some(screen) = weak.upgrade() else {
                return ControlFlow::Break;
            };
            match receive.try_recv() {
                Ok((server, connection, result)) => {
                    screen.transition(Action::DemoClosed);
                    match result {
                        Ok(()) => {
                            drop(server);
                            if let Some(window) = screen.window.upgrade() {
                                window.close();
                            }
                        }
                        Err(error) => {
                            screen.demo.replace(Some((server, connection)));
                            screen.request_status.set_label("Не удалось остановить тестовый сервер. Можно повторить закрытие окна.");
                            screen.error_detail.set_label(&error);
                            screen.details.set_visible(true);
                        }
                    }
                }
                Err(TryRecvError::Empty) => return ControlFlow::Continue,
                Err(TryRecvError::Disconnected) => {
                    screen.transition(Action::DemoClosed);
                    screen
                        .request_status
                        .set_label("Поток остановки сервера прервался. Закрытие не подтверждено.");
                }
            }
            ControlFlow::Break
        });
    }

    fn start_demo(self: &Rc<Self>) {
        if self.state.get() != UiState::Idle {
            return;
        }
        let existing = self
            .demo
            .borrow()
            .as_ref()
            .map(|(_, connection)| connection.clone());
        self.transition(Action::StartDemo);
        self.clear_details();
        self.debugger_status.set_label("");
        self.request_status.set_label("Готовим тестовую базу…");
        let (send, receive) = channel();
        let worker = std::thread::Builder::new().spawn(move || {
            let result = (|| {
                let (server, connection) = if let Some(connection) = existing {
                    (None, connection)
                } else {
                    let (server, password) =
                        DemoServer::start_isolated().map_err(|err| err.to_string())?;
                    let connection = Connection::new(
                        "127.0.0.1",
                        &server.port().to_string(),
                        "postgres",
                        "debugger_test",
                        &password,
                    )
                    .map_err(|err| err.message.to_owned())?;
                    (Some(server), connection)
                };
                let mut selection = Selection {
                    schema: Some("public".into()),
                    routine_oid: None,
                };
                let catalog = browse(&connection, &selection).map_err(|err| err.detail)?;
                selection.routine_oid = catalog
                    .routines
                    .iter()
                    .find(|item| item.name == "debug_smoke")
                    .map(|item| item.oid);
                let catalog = browse(&connection, &selection).map_err(|err| err.detail)?;
                Ok::<_, String>((server, connection, catalog))
            })();
            let _ = send.send(result);
        });
        if worker.is_err() {
            self.transition(Action::Loaded);
            self.request_status
                .set_label("Не удалось начать подготовку тестовой базы.");
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(25), move || {
            let Some(screen) = weak.upgrade() else {
                return ControlFlow::Break;
            };
            match receive.try_recv() {
                Ok(Ok((server, connection, catalog))) => {
                    if let Some(server) = server {
                        screen.demo.replace(Some((server, connection.clone())));
                    }
                    screen.accept_catalog(connection, catalog);
                    screen
                        .request_status
                        .set_label("Тестовый пример · debug_smoke(3)");
                    screen.arguments.set_value(0, "3");
                    screen.transition(Action::Loaded);
                    if !screen.close_after_stop.get() {
                        screen.start_debug();
                    }
                }
                Ok(Err(error)) => {
                    screen.request_status.set_label("Не удалось подготовить тестовую базу. Проверьте установку PostgreSQL и pldebugger.");
                    screen.error_detail.set_label(&error);
                    screen.details.set_visible(true);
                }
                Err(TryRecvError::Disconnected) => screen
                    .request_status
                    .set_label("Подготовка тестовой базы прервалась."),
                Err(TryRecvError::Empty) => return ControlFlow::Continue,
            }
            if screen.state.get() == UiState::StartingDemo {
                screen.transition(Action::Loaded);
            }
            screen.close_if_requested();
            ControlFlow::Break
        });
    }

    fn connect_from_form(self: &Rc<Self>) {
        let fields = Connection::new(
            &self.host.text(),
            &self.port.text(),
            &self.database_input.text(),
            &self.user.text(),
            &self.password.text(),
        );
        match fields {
            Ok(connection) => {
                self.connect.grab_focus();
                self.request(connection, Selection::default());
            }
            Err(error) => {
                self.clear_details();
                self.request_status
                    .set_label(&format!("{}: {}", error.field, error.message));
            }
        }
    }

    fn change_database(self: &Rc<Self>, database: &str) {
        if self.rendering.get() || self.state.get() != UiState::Idle {
            return;
        }
        let Some(mut connection) = self.connection() else {
            return;
        };
        connection.database = database.to_owned();
        self.request(connection, Selection::default());
    }

    fn change_selection(self: &Rc<Self>, selection: Selection) {
        if self.rendering.get() || self.state.get() != UiState::Idle {
            return;
        }
        if let Some(connection) = self.connection() {
            self.request(connection, selection);
        }
    }

    fn connection(&self) -> Option<Connection> {
        self.current
            .borrow()
            .as_ref()
            .map(|(connection, _)| connection.clone())
    }

    fn request(self: &Rc<Self>, connection: Connection, selection: Selection) {
        if self.state.get() != UiState::Idle {
            return;
        }
        self.transition(Action::LoadCatalog);
        self.clear_details();
        self.request_status.set_label("Обращаемся к PostgreSQL…");
        let (sender, receiver) = channel();
        let started = std::thread::Builder::new().spawn(move || {
            let result = browse(&connection, &selection);
            let _ = sender.send((connection, result));
        });
        if started.is_err() {
            self.request_status
                .set_label("Не удалось начать проверку подключения.");
            self.transition(Action::Loaded);
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(25), move || {
            let Some(screen) = weak.upgrade() else {
                return ControlFlow::Break;
            };
            match receiver.try_recv() {
                Ok((connection, Ok(catalog))) => {
                    screen.database_input.set_text(&connection.database);
                    screen.accept_catalog(connection, catalog);
                    if let Some(message) = screen.catalog_message.borrow_mut().take() {
                        screen.request_status.set_label(&message);
                    }
                }
                Ok((_, Err(error))) => {
                    screen.catalog_message.borrow_mut().take();
                    screen.request_status.set_label(error.message);
                    screen.error_detail.set_label(&error.detail);
                    screen.details.set_visible(true);
                    screen.restore_selectors();
                }
                Err(TryRecvError::Empty) => return ControlFlow::Continue,
                Err(TryRecvError::Disconnected) => {
                    screen
                        .request_status
                        .set_label("Проверка подключения прервалась.");
                    screen.restore_selectors();
                }
            }
            screen.transition(Action::Loaded);
            screen.close_if_requested();
            ControlFlow::Break
        });
    }

    fn accept_catalog(&self, connection: Connection, catalog: Catalog) {
        let status = if catalog.schemas.is_empty() {
            "Доступных пользовательских схем нет."
        } else if catalog.routines.is_empty() {
            "В выбранной схеме нет доступных функций и процедур PL/pgSQL."
        } else if catalog.selected.is_some() {
            "Открыт исходный код выбранного объекта."
        } else {
            "Подключено. Выберите функцию или процедуру."
        };
        self.request_status.set_label(status);
        self.current.replace(Some((connection, catalog)));
        self.render_catalog();
        self.pages.set_visible_child_name("workspace");
    }

    fn render_catalog(&self) {
        let state = self.current.borrow();
        let Some((connection, catalog)) = state.as_ref() else {
            return;
        };
        self.rendering.set(true);
        self.database.remove_all();
        for name in &catalog.databases {
            self.database.append(Some(name), name);
        }
        self.database.set_active_id(Some(&connection.database));
        self.schema.remove_all();
        for name in &catalog.schemas {
            self.schema.append(Some(name), name);
        }
        self.schema.set_active_id(catalog.schema.as_deref());
        self.render_routines(catalog);
        self.rendering.set(false);

        self.debugger_status.set_label(debugger_message(catalog));
        self.editor.load(
            catalog
                .selected
                .as_ref()
                .map(|object| object.source.as_str()),
        );
        self.arguments.load(
            catalog
                .selected
                .as_ref()
                .map(|object| object.arguments.as_slice()),
        );
        self.inspection.set_label("");
        self.result.set_label("");
        self.inspector.set_current_page(Some(0));
        self.object_title.set_label("Исходный код");
        self.execution.set_label("Выберите объект для отладки.");
        if let Some(object) = &catalog.selected {
            if let Some(routine) = catalog
                .routines
                .iter()
                .find(|routine| routine.oid == object.oid)
            {
                self.object_title.set_label(&format!(
                    "{}.{}({})",
                    catalog.schema.as_deref().unwrap_or_default(),
                    routine.name,
                    routine.signature
                ));
            }
            self.execution
                .set_label("Щёлкни между номером строки и кодом, чтобы выбрать точку.");
        }
    }

    fn render_routines(&self, catalog: &Catalog) {
        let previous = self.rendering.replace(true);
        while let Some(row) = self.routine.row_at_index(0) {
            self.routine.remove(&row);
        }
        let query = self.search.text().trim().to_lowercase();
        let mut ids = self.visible_routines.borrow_mut();
        ids.clear();
        let selected = catalog.selected.as_ref().map(|item| item.oid);
        for object in &catalog.routines {
            let title = format!("{}({})", object.name, object.signature);
            if !title.to_lowercase().contains(&query) {
                continue;
            }
            let cell = panel(2);
            cell.set_margin_top(6);
            cell.set_margin_bottom(6);
            cell.set_margin_start(8);
            cell.set_margin_end(8);
            cell.append(&heading(&title, "monospace"));
            cell.append(&heading(object.kind.label(), "dim-label"));
            self.routine.append(&cell);
            ids.push(object.oid);
            if selected == Some(object.oid) {
                self.routine
                    .select_row(self.routine.row_at_index(ids.len() as i32 - 1).as_ref());
            }
        }
        self.rendering.set(previous);
    }

    fn restore_selectors(&self) {
        let state = self.current.borrow();
        if let Some((connection, catalog)) = state.as_ref() {
            self.rendering.set(true);
            self.database.set_active_id(Some(&connection.database));
            self.schema.set_active_id(catalog.schema.as_deref());
            self.render_routines(catalog);
            self.rendering.set(false);
        }
    }

    fn transition(&self, action: Action) -> bool {
        let Some(state) = self.state.get().transition(action) else {
            return false;
        };
        self.state.set(state);
        self.refresh_controls();
        true
    }

    fn refresh_controls(&self) {
        let debug_active = self.state.get().is_debugging();
        let idle = self.state.get() == UiState::Idle;
        self.editor
            .clear
            .set_sensitive(self.state.get().can_edit_points() && self.editor.has_points());
        self.debugger_status
            .set_visible(!debug_active && !self.debugger_status.text().is_empty());
        self.fields.set_sensitive(idle);
        self.demo_button.set_sensitive(idle);
        self.own_button.set_sensitive(idle);
        self.arguments.root.set_sensitive(idle);
        let state = self.current.borrow();
        let catalog = state.as_ref().map(|(_, catalog)| catalog);
        self.search.set_sensitive(idle && catalog.is_some());
        self.database.set_sensitive(idle && catalog.is_some());
        self.schema
            .set_sensitive(idle && catalog.is_some_and(|c| !c.schemas.is_empty()));
        self.routine
            .set_sensitive(idle && catalog.is_some_and(|c| !c.routines.is_empty()));
        self.start
            .set_sensitive(idle && catalog.is_some_and(can_debug));
        self.start
            .set_tooltip_text(catalog.filter(|c| !can_debug(c)).map(debugger_message));
        self.step.set_sensitive(self.state.get() == UiState::Paused);
        self.resume
            .set_sensitive(self.state.get() == UiState::Paused);
        self.stop
            .set_sensitive(debug_active && self.state.get() != UiState::Stopping);
    }

    fn clear_details(&self) {
        self.details.set_expanded(false);
        self.details.set_visible(false);
        self.error_detail.set_label("");
    }

    fn start_debug(self: &Rc<Self>) {
        if self.state.get() != UiState::Idle {
            return;
        }
        let state = self.current.borrow();
        let Some((connection, catalog)) = state.as_ref() else {
            return;
        };
        if !can_debug(catalog) {
            self.execution.set_label(debugger_message(catalog));
            return;
        }
        let Some(details) = &catalog.selected else {
            return;
        };
        let Some(schema) = &catalog.schema else {
            return;
        };
        let Some(routine) = catalog.routines.iter().find(|item| item.oid == details.oid) else {
            return;
        };
        let values = self.arguments.values();
        let call = match Call::new(
            routine.oid,
            routine.kind,
            schema,
            &routine.name,
            &details.arguments,
            values,
        ) {
            Ok(call) => call.with_definition(details.definition.clone()),
            Err(error) => {
                self.execution.set_label(&error);
                return;
            }
        };
        let session = match Session::start(connection.clone(), call, self.editor.points()) {
            Ok(session) => session,
            Err(error) => {
                self.execution
                    .set_label(&format!("Не удалось начать отладку: {error}"));
                return;
            }
        };
        drop(state);
        self.session.replace(Some(session));
        self.transition(Action::StartDebug);
        self.editor.begin_run();
        self.execution.set_label("Запускаем выбранный объект…");
        self.inspection.set_label("");
        self.result.set_label("");
        self.refresh_controls();
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(25), move || {
            let Some(screen) = weak.upgrade() else {
                return ControlFlow::Break;
            };
            screen.poll_debug()
        });
    }

    fn poll_debug(self: &Rc<Self>) -> ControlFlow {
        loop {
            let event = {
                let session = self.session.borrow();
                let Some(session) = session.as_ref() else {
                    return ControlFlow::Break;
                };
                session.events.try_recv()
            };
            match event {
                Ok(Event::Paused {
                    position,
                    stack,
                    variables,
                    ..
                }) if self.state.get() != UiState::Stopping => {
                    self.inspector.set_current_page(Some(1));
                    self.transition(Action::Pause);
                    self.editor.clear_current();
                    if let Some(frame) = stack.first() {
                        self.execution.set_label(&format!(
                            "Остановка: {}, строка {}",
                            frame.target_name, frame.line_number
                        ));
                        let selected = self.current.borrow().as_ref().and_then(|(_, catalog)| {
                            catalog.selected.as_ref().map(|item| item.oid)
                        });
                        if selected == Some(frame.function_oid) {
                            self.editor.observe_stop(frame.line_number);
                        }
                    } else {
                        self.execution
                            .set_label(&format!("Остановка: {}", position.target_name));
                    }
                    let vars = variables
                        .iter()
                        .map(|var| {
                            format!(
                                "{} = {}{}",
                                var.name,
                                var.value.as_deref().unwrap_or("NULL"),
                                if var.value_truncated {
                                    " … [показан фрагмент]"
                                } else {
                                    ""
                                }
                            )
                        })
                        .collect::<Vec<_>>();
                    let frames = stack
                        .iter()
                        .map(|frame| {
                            format!(
                                "#{} {} · строка {}",
                                frame.level, frame.target_name, frame.line_number
                            )
                        })
                        .collect::<Vec<_>>();
                    self.inspection.set_label(&format!(
                        "Переменные\n{}\n\nСтек вызовов\n{}",
                        vars.join("\n"),
                        frames.join("\n")
                    ));
                    self.refresh_controls();
                }
                Ok(
                    Event::Paused { .. }
                    | Event::FrameSelected { .. }
                    | Event::NestedBreakpointsChanged { .. },
                ) => {}
                Ok(Event::BreakpointRejected {
                    line,
                    enabled,
                    message,
                }) => {
                    self.editor.set_point(line, !enabled);
                    self.editor.feedback.set_label(&message);
                    self.refresh_controls();
                }
                Ok(Event::Warning(warning)) => {
                    self.execution.set_label(&warning);
                    let previous = self.inspection.text();
                    self.inspection.set_label(&format!("{previous}\n{warning}"));
                }
                Ok(Event::Finished(report)) => {
                    let reload = matches!(report.call, CallOutcome::DefinitionChanged)
                        && !self.close_after_stop.get();
                    let text = report.render();
                    let done = self.finish_debug(&report);
                    if reload {
                        let request =
                            self.current.borrow().as_ref().map(|(connection, catalog)| {
                                (
                                    connection.clone(),
                                    Selection {
                                        schema: catalog.schema.clone(),
                                        routine_oid: catalog.selected.as_ref().map(|d| d.oid),
                                    },
                                )
                            });
                        if let Some((connection, selection)) = request {
                            self.catalog_message.replace(Some(text));
                            self.request(connection, selection);
                        }
                    }
                    return done;
                }
                Err(TryRecvError::Empty) => return ControlFlow::Continue,
                Err(TryRecvError::Disconnected) => {
                    return self.finish_debug(&SessionOutcome::unknown(
                        "Потеряно соединение с рабочим потоком отладки.",
                    ));
                }
            }
        }
    }

    fn finish_debug(&self, report: &SessionOutcome) -> ControlFlow {
        self.session.replace(None);
        self.transition(Action::Finished);
        self.editor.clear_current();
        self.execution.set_label("Сеанс завершён.");
        self.result.set_label(&report.render());
        self.refresh_controls();
        self.close_if_requested();
        ControlFlow::Break
    }

    fn close_if_requested(&self) {
        if self.close_after_stop.replace(false) {
            if let Some(window) = self.window.upgrade() {
                window.close();
            }
        }
    }

    fn resume_debug(&self, command: Command) {
        if !self.transition(Action::Resume) {
            return;
        }
        self.editor.clear_current();
        self.execution.set_label("Выполняется…");
        if let Some(session) = self.session.borrow().as_ref() {
            session.command(command);
        }
        self.refresh_controls();
    }

    fn stop_debug(&self) {
        if let Some(session) = self.session.borrow().as_ref() {
            if !self.transition(Action::Stop) {
                return;
            }
            self.execution.set_label("Останавливаем вызов…");
            session.stop();
        }
        self.refresh_controls();
    }

    fn toggle_point(&self, iter: &gtk::TextIter) {
        if !self.state.get().can_edit_points() {
            return;
        }
        if self
            .current
            .borrow()
            .as_ref()
            .is_none_or(|(_, catalog)| catalog.selected.is_none())
        {
            return;
        }
        let requested = iter.line() + 1;
        let line = match self.editor.resolve(requested) {
            Ok(line) => line,
            Err(message) => {
                self.editor
                    .feedback
                    .set_label(&format!("Строка {requested}: {message}"));
                return;
            }
        };
        let enabled = !self.editor.has_point(line);
        self.set_point(line, enabled);
        self.inspector.set_current_page(Some(2));
        let prefix = if requested == line {
            format!("Строка {line}")
        } else {
            format!("Строка {requested} → {line} (начало оператора)")
        };
        self.editor.feedback.set_label(&format!(
            "{prefix}: точка {}.",
            if enabled {
                "задана"
            } else {
                "снята"
            }
        ));
    }

    fn set_point(&self, line: i32, enabled: bool) {
        self.editor.set_point(line, enabled);
        if let Some(session) = self.session.borrow().as_ref() {
            session.command(Command::Breakpoint { line, enabled });
        }
        self.refresh_controls();
    }
}

fn selected_routine(catalog: &Catalog) -> Option<&pg_debugger_gui::catalog::Routine> {
    let oid = catalog.selected.as_ref()?.oid;
    catalog.routines.iter().find(|routine| routine.oid == oid)
}

fn can_debug(catalog: &Catalog) -> bool {
    matches!(
        catalog.debugger,
        DebuggerStatus::PluginConfigured | DebuggerStatus::PluginUnverified
    ) && selected_routine(catalog).is_some_and(|routine| routine.debug_allowed)
}

fn debugger_message(catalog: &Catalog) -> &'static str {
    if !matches!(
        catalog.debugger,
        DebuggerStatus::PluginConfigured | DebuggerStatus::PluginUnverified
    ) {
        return catalog.debugger.message();
    }
    match selected_routine(catalog) {
        Some(routine) if !routine.debug_allowed => {
            "Для этой функции или процедуры подключитесь как её владелец или администратор PostgreSQL."
        }
        Some(_) if catalog.debugger == DebuggerStatus::PluginUnverified => {
            catalog.debugger.message()
        }
        Some(_) => "Можно начать отладку выбранного объекта.",
        None => "Выберите функцию или процедуру, чтобы начать отладку.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    #[ignore = "requires the PostgreSQL fixture and an X11 or Wayland session"]
    fn window_connects_and_debugs() {
        gtk::init().expect("GTK");
        let app = gtk::Application::builder()
            .application_id("io.github.tilangelo.PgDebugger.Test")
            .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk::gio::Cancellable>)
            .expect("GTK app");
        let screen = Screen::show(&app);
        screen.own_button.emit_clicked();
        screen
            .port
            .set_text(&std::env::var("PGDEBUG_GUI_TEST_PORT").expect("fixture port"));
        screen.database_input.set_text("gui_plugin_test");
        screen.user.set_text("debugger_test");
        screen
            .password
            .set_text(&std::env::var("PGDEBUG_GUI_TEST_PASSWORD").expect("fixture password"));
        screen.connect.emit_clicked();
        wait_until(|| screen.current.borrow().is_some());
        let oid = screen
            .current
            .borrow()
            .as_ref()
            .unwrap()
            .1
            .routines
            .iter()
            .find(|routine| routine.name == "debug_smoke")
            .unwrap()
            .oid;
        screen.search.set_text("debug_smoke");
        wait_until(|| screen.visible_routines.borrow().len() == 2);
        let index = screen
            .visible_routines
            .borrow()
            .iter()
            .position(|id| *id == oid)
            .unwrap();
        screen
            .routine
            .select_row(screen.routine.row_at_index(index as i32).as_ref());
        wait_until(|| screen.start.is_sensitive());
        screen.arguments.set_value(0, "3");
        screen.start.emit_clicked();
        wait_until(|| screen.step.is_sensitive());
        assert!(screen.execution.text().contains("строка 5"));
        assert!(screen.inspection.text().contains("running_total = 3"));
        screen.step.emit_clicked();
        wait_until(|| screen.resume.is_sensitive());
        assert!(screen.execution.text().contains("строка 6"));
        assert!(screen.inspection.text().contains("running_total = 4"));
        screen.resume.emit_clicked();
        wait_until(|| screen.session.borrow().is_none());
        assert!(
            screen
                .result
                .text()
                .contains("Результат:\n8\nТранзакция откачена.")
        );

        // Тот же путь клика по полю отметок, но на многострочном SQL.
        screen.own_button.emit_clicked();
        screen.database_input.set_text("gui_shop_test");
        screen.search.set_text("");
        screen.connect.emit_clicked();
        wait_until(|| {
            !screen.state.get().is_busy()
                && screen
                    .current
                    .borrow()
                    .as_ref()
                    .is_some_and(|(connection, _)| connection.database == "gui_shop_test")
        });
        screen.schema.set_active_id(Some("sales"));
        wait_until(|| {
            !screen.state.get().is_busy()
                && screen
                    .current
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .1
                    .schema
                    .as_deref()
                    == Some("sales")
        });
        let index = {
            let state = screen.current.borrow();
            let catalog = &state.as_ref().unwrap().1;
            let oid = catalog
                .routines
                .iter()
                .find(|r| r.name == "order_total" && r.signature.contains("integer"))
                .unwrap()
                .oid;
            let index = screen
                .visible_routines
                .borrow()
                .iter()
                .position(|id| *id == oid)
                .unwrap();
            index
        };
        screen
            .routine
            .select_row(screen.routine.row_at_index(index as i32).as_ref());
        wait_until(|| screen.start.is_sensitive());
        for line in [6, 9, 11] {
            screen.toggle_point(&screen.editor.source.iter_at_line(line - 1).unwrap());
        }
        assert_eq!(screen.editor.points(), BTreeSet::from([5, 9, 10]));
        assert!(screen.editor.feedback.text().contains("11 → 10"));
        assert!(
            screen
                .editor
                .source
                .source_marks_at_line(5, Some(POINT))
                .is_empty()
        );
        // Повторный щелчок по продолжению снимает точку с начала оператора.
        screen.toggle_point(&screen.editor.source.iter_at_line(10).unwrap());
        assert_eq!(screen.editor.points(), BTreeSet::from([5, 9]));
        screen.toggle_point(&screen.editor.source.iter_at_line(10).unwrap());
        screen.toggle_point(&screen.editor.source.iter_at_line(11).unwrap());
        assert!(
            screen
                .editor
                .feedback
                .text()
                .contains("нет отдельной остановки")
        );
        screen.arguments.set_value(0, "1001");
        screen.start.emit_clicked();
        wait_until(|| screen.step.is_sensitive());
        assert!(screen.execution.text().contains("строка 5"));
        screen.resume.emit_clicked();
        wait_until(|| screen.step.is_sensitive());
        assert!(screen.execution.text().contains("строка 9"));
        assert!(
            screen
                .editor
                .point_list
                .text()
                .contains("Строка 10 - задана")
        );
        if let Ok(path) = std::env::var("PGDEBUG_GUI_SCREENSHOT") {
            screen.inspector.set_current_page(Some(2));
            let context = glib::MainContext::default();
            for _ in 0..10 {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            let window = screen.window.upgrade().unwrap();
            let paintable = gtk::WidgetPaintable::new(Some(&window));
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
            let node = snapshot.to_node().expect("window render node");
            window
                .renderer()
                .unwrap()
                .render_texture(&node, None)
                .save_to_png(path)
                .unwrap();
        }
        screen.editor.clear.emit_clicked();
        assert!(screen.editor.points().is_empty());
        screen.resume.emit_clicked();
        wait_until(|| screen.session.borrow().is_none());
        assert!(screen.result.text().contains("Результат:\n5990"));

        // Определение изменилось после открытия редактора: вызов отклоняется,
        // затем окно получает новый код и очищает старые точки.
        let mut editor = postgres::Config::new()
            .host("127.0.0.1")
            .port(
                std::env::var("PGDEBUG_GUI_TEST_PORT")
                    .unwrap()
                    .parse::<u16>()
                    .unwrap(),
            )
            .user("debugger_test")
            .password(std::env::var("PGDEBUG_GUI_TEST_PASSWORD").unwrap())
            .dbname("gui_shop_test")
            .connect(postgres::NoTls)
            .unwrap();
        editor.batch_execute("CREATE OR REPLACE FUNCTION sales.order_total(input_order_id integer) RETURNS integer LANGUAGE plpgsql AS $$ BEGIN RETURN 777; END; $$;").unwrap();
        screen.start.emit_clicked();
        wait_until(|| {
            !screen.state.get().is_busy()
                && screen.session.borrow().is_none()
                && screen
                    .editor
                    .source
                    .text(
                        &screen.editor.source.start_iter(),
                        &screen.editor.source.end_iter(),
                        false,
                    )
                    .contains("777")
        });
        assert!(screen.editor.points().is_empty());
        assert!(screen.request_status.text().contains("Вызов не выполнялся"));
        screen.arguments.set_value(0, "1001");
        screen.start.emit_clicked();
        wait_until(|| screen.step.is_sensitive());
        screen.resume.emit_clicked();
        wait_until(|| screen.session.borrow().is_none());
        assert!(screen.result.text().contains("Результат:\n777"));

        screen.demo_button.emit_clicked();
        wait_until(|| screen.step.is_sensitive());
        assert!(screen.execution.text().contains("строка 5"));
        assert!(screen.inspection.text().contains("running_total = 3"));
        let demo_port = screen.demo.borrow().as_ref().unwrap().0.port();
        screen.resume.emit_clicked();
        wait_until(|| screen.session.borrow().is_none());
        assert!(
            screen
                .result
                .text()
                .contains("Результат:\n8\nТранзакция откачена.")
        );
        let heartbeat = Rc::new(Cell::new(false));
        let tick = heartbeat.clone();
        glib::idle_add_local_once(move || tick.set(true));
        screen.window.upgrade().unwrap().close();
        assert_eq!(screen.state.get(), UiState::ClosingDemo);
        wait_until(|| heartbeat.get());
        wait_until(|| screen.state.get() != UiState::ClosingDemo);
        assert!(screen.demo.borrow().is_none());
        assert!(std::net::TcpStream::connect(("127.0.0.1", demo_port)).is_err());
        assert!(
            std::net::TcpStream::connect((
                "127.0.0.1",
                std::env::var("PGDEBUG_GUI_TEST_PORT")
                    .unwrap()
                    .parse::<u16>()
                    .unwrap()
            ))
            .is_ok()
        );

        // Закрытие во время запроса каталога ждёт его завершения.
        let loading = Screen::show(&app);
        loading.own_button.emit_clicked();
        loading
            .port
            .set_text(&std::env::var("PGDEBUG_GUI_TEST_PORT").unwrap());
        loading.database_input.set_text("gui_plugin_test");
        loading.user.set_text("debugger_test");
        loading
            .password
            .set_text(&std::env::var("PGDEBUG_GUI_TEST_PASSWORD").unwrap());
        loading.connect.emit_clicked();
        assert_eq!(loading.state.get(), UiState::LoadingCatalog);
        let window = loading.window.upgrade().unwrap();
        window.close();
        assert!(loading.close_after_stop.get());
        assert!(!loading.start.is_sensitive());
        wait_until(|| loading.state.get() == UiState::Idle && !window.is_visible());
    }

    fn wait_until(mut done: impl FnMut() -> bool) {
        let context = glib::MainContext::default();
        let deadline = Instant::now() + Duration::from_secs(12);
        while !done() {
            while context.pending() {
                context.iteration(false);
            }
            assert!(Instant::now() < deadline, "состояние окна не обновилось");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
