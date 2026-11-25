//! Поток отладки выполняет команды. Браузер получает текущее состояние.
use super::*;
use pg_debugger_core::{
    breakpoints::SourceMap,
    catalog::{ArgumentField, RoutineKind},
    outcome::{CallOutcome, RollbackOutcome, SessionOutcome},
    project::{Kind, Lines, Project},
    session::{Call, Command, Event, NestedPlan, Session, Variable},
    trigger::{TriggerCall, TriggerInput},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    time::{Duration, Instant},
};
use tokio::sync::OwnedSemaphorePermit;

const LEASE: Duration = Duration::from_secs(15);
#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    Starting,
    Running,
    Paused,
    Stopping,
    Finished,
}
impl Phase {
    fn text(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Stopping => "stopping",
            Self::Finished => "finished",
        }
    }
}
struct Live {
    frame_pending: bool,
    revision: u64,
    phase: Phase,
    data: Value,
    last_seen: Instant,
    points: BTreeSet<i32>,
    nested_points: BTreeMap<u32, NestedPoints>,
}
#[derive(serde::Serialize)]
struct NestedPoints {
    oid: u32,
    name: String,
    lines: BTreeSet<i32>,
}
pub(super) struct Control {
    id: u64,
    commands: mpsc::Sender<Command>,
    live: Mutex<Live>,
    source_map: SourceMap,
    graph: BTreeMap<String, Lines>,
    oid: u32,
}
impl Control {
    fn snapshot(&self, beat: bool, revision: Option<u64>) -> Value {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if beat {
            live.last_seen = Instant::now();
        }
        if revision == Some(live.revision) {
            return json!({"session_id":self.id,"revision":live.revision,"phase":live.phase.text(),"unchanged":true});
        }
        let mut data = live.data.clone();
        data["session_id"] = json!(self.id);
        data["revision"] = json!(live.revision);
        data["phase"] = json!(live.phase.text());
        data["breakpoints"] = json!(live.points);
        data["nested_breakpoints"] = json!(live.nested_points.values().collect::<Vec<_>>());
        data
    }
    fn stop(&self, reason: &str) {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if !matches!(live.phase, Phase::Stopping | Phase::Finished) {
            live.phase = Phase::Stopping;
            live.revision += 1;
            live.data["warning"] = json!(reason);
            let _ = self.commands.send(Command::Stop);
        }
    }
    fn finished(&self) -> bool {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).phase == Phase::Finished
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Start {
    connection_id: u64,
    database: String,
    schema: String,
    routine_oid: u32,
    expected_definition: String,
    record_fields: Option<Vec<Option<Vec<ArgumentField>>>>,
    values: Vec<Option<String>>,
    #[serde(default)]
    default_arguments: BTreeSet<usize>,
    #[serde(default)]
    breakpoints: Vec<i32>,
    #[serde(default)]
    nested_breakpoints: Vec<NestedInput>,
    trigger: Option<TriggerInput>,
    project: Option<Project>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NestedInput {
    schema: String,
    oid: u32,
    expected_definition: String,
    lines: Vec<i32>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Poll {
    session_id: u64,
    revision: Option<u64>,
}
#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    Next {},
    StepInto {},
    StepOut {},
    SelectFrame {
        level: i32,
    },
    Continue {},
    Stop {},
    Breakpoint {
        line: i32,
        enabled: bool,
    },
    NestedBreakpoints {
        oid: u32,
        lines: Vec<i32>,
        enabled: bool,
    },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CommandInput {
    session_id: u64,
    revision: u64,
    action: Action,
}

fn control(
    state: &AppState,
    id: Option<u64>,
) -> Result<Option<Arc<Control>>, (StatusCode, Json<Value>)> {
    let slot = state.inner.debug.lock().map_err(|_| {
        failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Состояние отладки недоступно.",
        )
    })?;
    match slot.as_ref() {
        Some(c) if id.is_none_or(|id| c.id == id) => Ok(Some(c.clone())),
        None if id.is_none() => Ok(None),
        _ => Err(failure(
            StatusCode::CONFLICT,
            "Сеанс уже заменён. Восстановите состояние страницы.",
        )),
    }
}
pub(super) async fn current(State(state): State<AppState>) -> Reply {
    Ok(Json(match control(&state, None)? {
        Some(c) => c.snapshot(true, None),
        None => json!({"phase":"idle"}),
    }))
}
pub(super) async fn poll(State(state): State<AppState>, Json(input): Json<Poll>) -> Reply {
    Ok(Json(
        control(&state, Some(input.session_id))?
            .unwrap()
            .snapshot(true, input.revision),
    ))
}
pub(super) async fn command(
    State(state): State<AppState>,
    Json(input): Json<CommandInput>,
) -> Reply {
    let c = control(&state, Some(input.session_id))?.unwrap();
    if matches!(input.action, Action::Stop { .. }) {
        c.stop("Останавливаем вызов и ожидаем подтверждение отката.");
        return Ok(Json(c.snapshot(true, None)));
    }
    {
        let mut live = c.live.lock().map_err(|_| {
            failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Состояние отладки недоступно.",
            )
        })?;
        if live.phase != Phase::Paused || live.revision != input.revision || live.frame_pending {
            return Err(failure(
                StatusCode::CONFLICT,
                "Состояние изменилось. Дождитесь обновления перед следующей командой.",
            ));
        }
        let action = match input.action {
            Action::Next { .. } => {
                live.phase = Phase::Running;
                Command::Step
            }
            Action::StepInto { .. } => {
                live.phase = Phase::Running;
                Command::StepInto
            }
            Action::StepOut { .. } => {
                if frame_at(&live.data, 1).is_none() {
                    return Err(failure(
                        StatusCode::BAD_REQUEST,
                        "Шаг наружу доступен во вложенном вызове PL/pgSQL.",
                    ));
                }
                live.phase = Phase::Running;
                Command::StepOut
            }
            Action::SelectFrame { level } => {
                if frame_at(&live.data, level).is_none() {
                    return Err(failure(
                        StatusCode::BAD_REQUEST,
                        "Такого кадра нет на текущей остановке.",
                    ));
                }
                live.frame_pending = true;
                live.data["frame_pending"] = json!(true);
                Command::SelectFrame { level }
            }
            Action::Continue { .. } => {
                live.phase = Phase::Running;
                Command::Continue
            }
            Action::Breakpoint { line, enabled } => {
                let line = c
                    .source_map
                    .resolve(line)
                    .map_err(|e| failure(StatusCode::BAD_REQUEST, e))?;
                if enabled {
                    live.points.insert(line);
                } else {
                    live.points.remove(&line);
                }
                Command::Breakpoint { line, enabled }
            }
            Action::NestedBreakpoints {
                oid,
                lines,
                enabled,
            } => {
                let frame = &live.data["frame"];
                let allowed = if enabled {
                    lines.len() == 1 && frame["oid"] == oid && frame["source"].is_string()
                } else {
                    live.nested_points.get(&oid).is_some_and(|points| {
                        !lines.is_empty() && lines.iter().all(|line| points.lines.contains(line))
                    })
                };
                if oid == c.oid || !allowed {
                    return Err(failure(
                        StatusCode::BAD_REQUEST,
                        "Для добавления выберите вложенный кадр с исходником. Снять можно только установленную точку.",
                    ));
                }
                live.frame_pending = true;
                live.data["frame_pending"] = json!(true);
                live.data["warning"] = Value::Null;
                Command::NestedBreakpoints {
                    oid,
                    lines,
                    enabled,
                }
            }
            Action::Stop { .. } => unreachable!(),
        };
        if live.phase == Phase::Running {
            live.data["warning"] = Value::Null;
        }
        c.commands
            .send(action)
            .map_err(|_| failure(StatusCode::CONFLICT, "Поток управления завершился."))?;
        live.last_seen = Instant::now();
        live.revision += 1;
    }
    Ok(Json(c.snapshot(false, None)))
}

fn prepare_nested(
    connection: &catalog::Connection,
    root: u32,
    groups: Vec<NestedInput>,
) -> Result<Vec<NestedPlan>, (StatusCode, Json<Value>)> {
    if groups.len() > 8 || groups.iter().map(|group| group.lines.len()).sum::<usize>() > 32 {
        return Err(failure(
            StatusCode::BAD_REQUEST,
            "Можно подготовить до 32 точек в восьми вложенных объектах.",
        ));
    }
    let mut visited = BTreeSet::new();
    groups.into_iter().map(|group| {
        if group.oid == root || group.lines.is_empty() || !visited.insert(group.oid) {
            return Err(failure(StatusCode::BAD_REQUEST, "Укажите вложенные объекты без повторов. Точки основного объекта задаются отдельно."));
        }
        let catalog = catalog::browse(connection, &Selection {
            schema: Some(group.schema.clone()),
            routine_oid: Some(group.oid),
        }).map_err(catalog_error)?;
        let routine = catalog.routines.iter()
            .find(|entry| entry.oid == group.oid && catalog.schema.as_deref() == Some(group.schema.as_str()))
            .ok_or_else(|| failure(StatusCode::CONFLICT, "Вложенный объект изменился. Откройте его заново."))?;
        if !routine.debug_allowed {
            return Err(failure(StatusCode::FORBIDDEN, "Для вложенной точки нужны права владельца объекта или суперпользователя."));
        }
        let selected = catalog.selected
            .filter(|selected| selected.definition == group.expected_definition)
            .ok_or_else(|| failure(StatusCode::CONFLICT, "Определение вложенного объекта изменилось. Откройте его заново и обновите точки."))?;
        let name = format!("{}.{}({})", group.schema, routine.name, routine.signature);
        NestedPlan::new(group.oid, name, selected.definition, &selected.source, group.lines)
            .map_err(|error| failure(StatusCode::BAD_REQUEST, error))
    }).collect()
}

pub(super) async fn start(State(state): State<AppState>, Json(input): Json<Start>) -> Reply {
    let permit = state
        .inner
        .database_slot
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            failure(
                StatusCode::CONFLICT,
                "Дождитесь завершения текущего сеанса или запроса.",
            )
        })?;
    if control(&state, None)?.is_some_and(|c| !c.finished()) {
        return Err(failure(
            StatusCode::CONFLICT,
            "Предыдущий сеанс ещё не завершён.",
        ));
    }
    let mut connection = state
        .inner
        .connection
        .lock()
        .map_err(|_| failure(StatusCode::INTERNAL_SERVER_ERROR, "Подключение недоступно."))?
        .as_ref()
        .filter(|(id, _)| *id == input.connection_id)
        .map(|(_, c)| c.clone())
        .ok_or_else(|| failure(StatusCode::CONFLICT, "Подключитесь к базе заново."))?;
    if input.database.is_empty() {
        return Err(failure(StatusCode::BAD_REQUEST, "Выберите базу данных."));
    }
    connection.database = input.database.clone();
    tokio::task::spawn_blocking(move||{
        let mut catalog=catalog::browse(&connection,&Selection{schema:Some(input.schema.clone()),routine_oid:Some(input.routine_oid)}).map_err(catalog_error)?;
        if catalog.schema.as_deref()!=Some(&input.schema){return Err(failure(StatusCode::CONFLICT,"Схема изменилась. Обновите каталог."));}
        let routine=catalog.routines.iter().find(|r|r.oid==input.routine_oid).ok_or_else(||failure(StatusCode::CONFLICT,"Объект не найден в выбранной схеме."))?;
        if !routine.debug_allowed{return Err(failure(StatusCode::FORBIDDEN,"Для отладки войдите от владельца объекта или суперпользователя PostgreSQL."));}
        let selected=catalog.selected.take().ok_or_else(||failure(StatusCode::CONFLICT,"Обновите выбранный объект."))?;
        if selected.definition!=input.expected_definition{return Err(failure(StatusCode::CONFLICT,"Определение изменилось после открытия. Обновите каталог перед запуском."));}
        let trigger = match (selected.trigger_targets, input.trigger) {
            (Some(targets), Some(input)) => {
                let target = targets.into_iter().find(|target| target.oid == input.oid &&
                    target.definition == input.expected_definition)
                    .ok_or_else(|| failure(StatusCode::CONFLICT, "Триггер изменился или недоступен. Обновите каталог."))?;
                Some(TriggerCall::new(target, input).map_err(|error| failure(StatusCode::BAD_REQUEST, error))?)
            }
            (None, None) => None,
            _ => return Err(failure(StatusCode::BAD_REQUEST, "Выберите триггер таблицы и операцию для триггерной функции.")),
        };
        if let Some(expected) = &input.record_fields {
            let actual = selected.arguments.iter().map(|arg|
                arg.custom_type.as_ref().and_then(|ty| ty.fields.as_ref())
            );
            if !expected.iter().map(Option::as_ref).eq(actual) {
                return Err(failure(StatusCode::CONFLICT,"Поля составного аргумента изменились. Выберите объект заново."));
            }
        }
        let prepared=prepare_nested(&connection, selected.oid, input.nested_breakpoints)?;
        let call=Call::with_default_arguments(selected.oid,routine.kind,&input.schema,&routine.name,&selected.arguments,input.values,&input.default_arguments)
            .map_err(|e|failure(StatusCode::BAD_REQUEST,e))?.with_definition(selected.definition.clone())
            .with_table_result(selected.returns_set).with_nested_breakpoints(prepared).with_trigger(trigger);
        let source_map=SourceMap::new(&selected.source);
        let points=input.breakpoints.into_iter().map(|line|source_map.resolve(line)).collect::<Result<BTreeSet<_>,_>>().map_err(|e|failure(StatusCode::BAD_REQUEST,e))?;
        let project_for_client=input.project.clone();
        let graph=input.project.and_then(|p|{
            let correct_kind=matches!((p.kind,routine.kind),(Kind::Function,RoutineKind::Function)|(Kind::Procedure,RoutineKind::Procedure));
            let correct_args=p.parameters.len()==selected.arguments.len()&&p.parameters.iter().zip(&selected.arguments).all(|(p,a)|a.mode=="IN"&&p.data_type.oid().is_none_or(|oid|oid==a.type_oid));
            if p.name!=routine.name||p.schema!=input.schema||!correct_kind||!correct_args{return None;}
            p.generate().ok().filter(|g|g.body==selected.source
                && (p.parameters.iter().all(|param|param.data_type.oid().is_some())
                    || catalog::matches_signature(&connection,&g.signature,selected.oid)))
                .map(|g|g.source_map)
        }).unwrap_or_default();
        let id=state.inner.next_id.fetch_add(1,Ordering::Relaxed);let (tx,rx)=mpsc::channel();
        let c=Arc::new(Control{id,commands:tx,oid:selected.oid,source_map,graph,
            live:Mutex::new(Live{frame_pending:false,revision:1,phase:Phase::Starting,last_seen:Instant::now(),points,nested_points:BTreeMap::new(),
                data:json!({"server_version":catalog.server_version,"current_user":catalog.current_user,"debugger_status":catalog.debugger.message(),"database":input.database,"routine":{"oid":selected.oid,"name":routine.name,"signature":routine.signature,"schema":input.schema,"kind":routine.kind.label(),"source":selected.source,"definition":selected.definition},"project":project_for_client,"graph_mapping":false,"pause_id":0,"stops":[],"position":null,"variables":[],"stack":[],"outcome":null,"warning":null})})});
        {let mut live=c.live.lock().map_err(|_|failure(StatusCode::INTERNAL_SERVER_ERROR,"Состояние отладки недоступно."))?;live.data["graph_mapping"]=json!(!c.graph.is_empty());}
        let mut slot=state.inner.debug.lock().map_err(|_|failure(StatusCode::INTERNAL_SERVER_ERROR,"Состояние отладки недоступно."))?;
        if state.inner.closing.load(Ordering::Acquire){return Err(failure(StatusCode::SERVICE_UNAVAILABLE,"Приложение закрывается."));}
        let mut pending=state.inner.pending.lock().map_err(|_|failure(StatusCode::INTERNAL_SERVER_ERROR,"Предпросмотр недоступен."))?;
        let actor=c.clone();
        std::thread::Builder::new().name("web-debug-controller".into()).spawn(move||run(actor,connection,call,rx,permit)).map_err(|_|failure(StatusCode::INTERNAL_SERVER_ERROR,"Не удалось начать поток отладки."))?;
        *pending=None;
        *slot=Some(c.clone());drop(slot);drop(pending);
        Ok(Json(c.snapshot(false,None)))
    }).await.map_err(|_|failure(StatusCode::INTERNAL_SERVER_ERROR,"Запуск отладки не завершился."))?
}

fn outcome(report: &SessionOutcome) -> Value {
    let call = match &report.call {
        CallOutcome::NotStarted(_) => "not_started",
        CallOutcome::SetupFailed(_) => "setup_failed",
        CallOutcome::DefinitionChanged => "definition_changed",
        CallOutcome::Succeeded(_) => "succeeded",
        CallOutcome::Failed(_) => "failed",
        CallOutcome::Unknown(_) => "unknown",
    };
    let rollback = match &report.rollback {
        RollbackOutcome::NotNeeded => "not_needed",
        RollbackOutcome::Confirmed => "confirmed",
        RollbackOutcome::Unconfirmed(_) => "unconfirmed",
    };
    let sqlstate = match &report.call {
        CallOutcome::Failed(d) | CallOutcome::SetupFailed(d) => d.sqlstate.as_deref(),
        _ => None,
    };
    let result = match &report.call {
        CallOutcome::Succeeded(preview) => preview.table.as_ref().map(|table| {
            json!({
                "columns": table.columns, "rows": table.rows, "total_rows": preview.total_rows,
                "shown_rows": preview.shown_rows, "truncated": preview.truncated
            })
        }),
        _ => None,
    };
    let scalar = match &report.call {
        CallOutcome::Succeeded(preview) => preview.scalar.as_deref(),
        _ => None,
    };
    json!({"call":call,"rollback":rollback,"sqlstate":sqlstate,"cancellation_requested":report.cancellation_requested,"text":report.render(),"result":result,"scalar":scalar})
}
// Номер меняется только при новой остановке, а не при опросе или правке точки.
fn record_pause(data: &mut Value, line: Option<i32>, target: &str) {
    let number = data["pause_id"].as_u64().unwrap_or(0) + 1;
    data["pause_id"] = json!(number);
    let stops = data["stops"]
        .as_array_mut()
        .expect("stop history initialized");
    stops.push(json!({"number":number,"line":line,"target":target}));
    if stops.len() > 20 {
        stops.remove(0);
    }
}

fn finish(c: &Control, report: Value, permit: OwnedSemaphorePermit) {
    let mut live = c.live.lock().unwrap_or_else(|e| e.into_inner());
    live.phase = Phase::Finished;
    live.revision += 1;
    live.data["outcome"] = report;
    live.nested_points.clear();
    live.frame_pending = false;
    live.data["frame_pending"] = json!(false);
    drop(permit);
}

fn frame_at(data: &Value, level: i32) -> Option<&Value> {
    data["stack"]
        .as_array()?
        .iter()
        .find(|frame| frame["level"] == level)
}

fn inspect_frame(live: &mut Live, level: i32, source: Option<String>, variables: Vec<Variable>) {
    let mut frame = frame_at(&live.data, level).cloned().unwrap_or(Value::Null);
    if !frame.is_null() {
        frame["source"] = json!(source);
    }
    live.data["frame"] = frame;
    live.data["variables"] = json!(variables.iter().map(|v| json!({
        "name":v.name,"value":v.value,"declaration_line":v.declaration_line,"truncated":v.value_truncated
    })).collect::<Vec<_>>());
    live.frame_pending = false;
    live.data["frame_pending"] = json!(false);
}
fn run(
    c: Arc<Control>,
    connection: Connection,
    call: Call,
    commands: Receiver<Command>,
    permit: OwnedSemaphorePermit,
) {
    let points = c
        .live
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .points
        .clone();
    let session = match Session::start(connection, call, points) {
        Ok(session) => session,
        Err(_) => {
            finish(
                &c,
                json!({"call":"not_started","rollback":"not_needed","cancellation_requested":false,"text":"Не удалось создать поток вызова. Тело не выполнялось."}),
                permit,
            );
            return;
        }
    };
    loop {
        if c.live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last_seen
            .elapsed()
            >= LEASE
        {
            c.stop("Связь со страницей потеряна на 15 секунд. Отменяем вызов и ожидаем откат.");
        }
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::Stop => session.stop(),
                other => session.command(other),
            }
        }
        match session.events.recv_timeout(Duration::from_millis(100)) {
            Ok(Event::Finished(report)) => {
                finish(&c, outcome(&report), permit);
                return;
            }
            Ok(Event::Paused {
                position,
                stack,
                variables,
                source,
            }) => {
                let frame = stack.first();
                let root_line = frame
                    .filter(|f| f.function_oid == c.oid)
                    .map(|f| f.line_number);
                let block_id = root_line.and_then(|line| {
                    c.graph
                        .iter()
                        .filter(|(_, r)| line >= r.body_start as i32 && line <= r.body_end as i32)
                        .min_by_key(|(_, r)| r.body_end - r.body_start)
                        .map(|(id, _)| id.clone())
                });
                let mut live = c.live.lock().unwrap_or_else(|e| e.into_inner());
                if live.phase == Phase::Stopping {
                    continue;
                }
                live.phase = Phase::Paused;
                live.revision += 1;
                let line = frame.map(|f| f.line_number).filter(|line| *line > 0);
                record_pause(&mut live.data, line, &position.target_name);
                live.data["position"] = json!({"line":line,"target":position.target_name,"oid":frame.map(|f| f.function_oid),"block_id":block_id});
                live.data["stack"]=json!(stack.iter().map(|f|json!({"level":f.level,"oid":f.function_oid,"name":f.target_name,"line":f.line_number,"arguments":f.arguments,"truncated":f.arguments_truncated})).collect::<Vec<_>>());
                inspect_frame(&mut live, frame.map_or(0, |f| f.level), source, variables);
            }
            Ok(Event::FrameSelected {
                level,
                variables,
                source,
            }) => {
                let mut live = c.live.lock().unwrap_or_else(|e| e.into_inner());
                if live.phase == Phase::Paused {
                    inspect_frame(&mut live, level, source, variables);
                    live.revision += 1;
                }
            }
            Ok(Event::Warning(message)) => {
                let mut live = c.live.lock().unwrap_or_else(|e| e.into_inner());
                live.revision += 1;
                live.data["warning"] = json!(message);
            }
            Ok(Event::NestedBreakpointsChanged {
                oid,
                name,
                lines,
                enabled,
                warning,
            }) => {
                let mut live = c.live.lock().unwrap_or_else(|e| e.into_inner());
                if live.phase != Phase::Paused && !(live.phase == Phase::Starting && name.is_some())
                {
                    continue;
                }
                if enabled {
                    if !lines.is_empty() {
                        let name = name.unwrap_or_else(|| {
                            live.data["frame"]["name"]
                                .as_str()
                                .unwrap_or_default()
                                .to_owned()
                        });
                        live.nested_points
                            .entry(oid)
                            .or_insert_with(|| NestedPoints {
                                oid,
                                name,
                                lines: BTreeSet::new(),
                            })
                            .lines
                            .extend(lines);
                    }
                } else if let Some(points) = live.nested_points.get_mut(&oid) {
                    for line in lines {
                        points.lines.remove(&line);
                    }
                    if points.lines.is_empty() {
                        live.nested_points.remove(&oid);
                    }
                }
                live.frame_pending = false;
                live.data["frame_pending"] = json!(false);
                live.data["warning"] = json!(warning);
                live.revision += 1;
            }
            Ok(Event::BreakpointRejected {
                line,
                enabled,
                message,
            }) => {
                let mut live = c.live.lock().unwrap_or_else(|e| e.into_inner());
                if enabled {
                    live.points.remove(&line);
                } else {
                    live.points.insert(line);
                }
                live.revision += 1;
                live.data["warning"] = json!(message);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                // Без Finished нельзя объявить вызов завершённым или освободить слот.
                c.stop("Поток отладки потерян. Завершение вызова и откат не подтверждены; новый запуск заблокирован.");
                session.stop();
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}
pub(super) async fn shutdown(state: &AppState) {
    state.inner.closing.store(true, Ordering::Release);
    if let Ok(Some(c)) = control(state, None) {
        c.stop("Приложение закрывается. Ожидаем завершение вызова и откат.");
        while !c.finished() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commands_reject_unknown_values() {
        assert!(
            serde_json::from_value::<CommandInput>(
                json!({"session_id":1,"revision":1,"action":{"command":"next"}})
            )
            .is_ok()
        );
        assert!(serde_json::from_value::<CommandInput>(json!({"session_id":1,"revision":1,"action":{"command":"evaluate","sql":"SELECT 1"}})).is_err());
        assert!(
            serde_json::from_value::<CommandInput>(
                json!({"session_id":1,"revision":1,"action":{"command":"next","sql":"SELECT 1"}})
            )
            .is_err()
        );
    }
    #[test]
    fn pause_numbers_and_stop_history_are_bounded() {
        let mut data = json!({"pause_id":0,"stops":[]});
        for _ in 0..25 {
            record_pause(&mut data, Some(5), "f(integer)");
        }
        assert_eq!(data["pause_id"], 25);
        assert_eq!(data["stops"].as_array().unwrap().len(), 20);
        assert_eq!(data["stops"][0]["number"], 6);
        record_pause(&mut data, None, "nested");
        assert!(data["stops"][19]["line"].is_null());
    }

    #[test]
    fn outcome_does_not_infer_success_from_text() {
        let report = SessionOutcome::unknown("Откат не подтверждён");
        let value = outcome(&report);
        assert_eq!(value["call"], "unknown");
        assert_eq!(value["rollback"], "unconfirmed");
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Resolve {
    source: String,
    line: i32,
}
pub(super) async fn resolve(Json(input): Json<Resolve>) -> Reply {
    if input.source.len() > 1_000_000 {
        return Err(failure(
            StatusCode::BAD_REQUEST,
            "Тело слишком большое для карты строк.",
        ));
    }
    let line = SourceMap::new(&input.source)
        .resolve(input.line)
        .map_err(|e| failure(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(json!({"line":line,"requested_line":input.line})))
}
