use postgres::{Client, NoTls};

// Сведения о сервере и версии API отладчика.
pub struct ProxyInfo {
    pub server_version: String,
    pub server_version_num: i32,
    pub proxy_api_version: i32,
    pub proxy_pid: i32,
}

// Позиция, на которой остановился целевой процесс.
pub struct Breakpoint {
    pub line_number: i32,
    pub target_name: String,
}

// Установленная точка: функция и строка в её исходном тексте.
pub struct BreakpointInfo {
    pub function_oid: u32,
    pub line_number: i32,
    pub target_name: String,
}

// После команды получаем новую остановку или закрытый канал.
pub enum DebugEvent {
    Paused(Breakpoint),
    ChannelClosed,
}
// Значение переменной и строка, где она объявлена.
pub struct Variable {
    pub name: String,
    pub declaration_line: i32,
    pub value: Option<String>,
}
// Один кадр стека вызовов PL/pgSQL.
pub struct StackFrame {
    pub level: i32,
    pub target_name: String,
    pub line_number: i32,
    pub arguments: Option<String>,
}
// Одно соединение с proxy backend для всех команд отладки.
pub struct PldbgClient {
    client: Client,
}

impl PldbgClient {
    // Открываем соединение для вызовов pldbgapi.
    pub fn connect(connection_string: &str) -> Result<Self, postgres::Error> {
        let client = Client::connect(connection_string, NoTls)?;
        Ok(Self { client })
    }

    // Читаем версию сервера, версию API и PID proxy backend.
    pub fn proxy_info(&mut self) -> Result<ProxyInfo, postgres::Error> {
        let row = self.client.query_one(
            "SELECT serverversionstr, serverversionnum, proxyapiver, serverprocessid \
             FROM pldbg_get_proxy_info()",
            &[],
        )?;

        Ok(ProxyInfo {
            server_version: row.try_get("serverversionstr")?,
            server_version_num: row.try_get("serverversionnum")?,
            proxy_api_version: row.try_get("proxyapiver")?,
            proxy_pid: row.try_get("serverprocessid")?,
        })
    }

    // PostgreSQL создаёт listener и возвращает номер сеанса отладки.
    pub fn create_listener(&mut self) -> Result<i32, postgres::Error> {
        let row = self
            .client
            .query_one("SELECT pldbg_create_listener()", &[])?;
        row.try_get(0)
    }

    // Функцию ищем по имени и типам аргументов; NULL ставит точку на входе.
    pub fn set_global_breakpoint(
        &mut self,
        listener: i32,
        target: &str,
        line: Option<i32>,
    ) -> Result<bool, postgres::Error> {
        // Второй NULL разрешает остановить вызов из любого target backend.
        let row = self.client.query_one(
            "SELECT pldbg_set_global_breakpoint($1, $2::text::regprocedure::oid, $3, NULL)",
            &[&listener, &target, &line],
        )?;
        row.try_get(0)
    }

    // Добавляем точку в уже подключённом сеансе отладки.
    pub fn set_breakpoint(
        &mut self,
        listener: i32,
        target: &str,
        line: i32,
    ) -> Result<bool, postgres::Error> {
        let row = self.client.query_one(
            "SELECT pldbg_set_breakpoint($1, $2::text::regprocedure::oid, $3)",
            &[&listener, &target, &line],
        )?;
        row.try_get(0)
    }

    // Удаляем точку по той же функции и строке, на которой её поставили.
    pub fn drop_breakpoint(
        &mut self,
        listener: i32,
        target: &str,
        line: i32,
    ) -> Result<bool, postgres::Error> {
        let row = self.client.query_one(
            "SELECT pldbg_drop_breakpoint($1, $2::text::regprocedure::oid, $3)",
            &[&listener, &target, &line],
        )?;
        row.try_get(0)
    }

    // Список нужен, чтобы пользователь видел фактическое состояние в PostgreSQL.
    pub fn get_breakpoints(
        &mut self,
        listener: i32,
    ) -> Result<Vec<BreakpointInfo>, postgres::Error> {
        let mut points: Vec<BreakpointInfo> = self.client
            .query(
                "SELECT func, linenumber, func::regprocedure::text AS signature FROM pldbg_get_breakpoints($1) ORDER BY func, linenumber",
                &[&listener],
            )?
            .iter()
            .map(|row| {
                Ok(BreakpointInfo {
                    function_oid: row.try_get("func")?,
                    line_number: row.try_get("linenumber")?,
                    target_name: row.try_get("signature")?,
                })
            })
            .collect::<Result<_, postgres::Error>>()?;
        // Плагин может вернуть одну глобальную точку дважды.
        points.dedup_by(|a, b| a.function_oid == b.function_oid && a.line_number == b.line_number);
        Ok(points)
    }

    // Получаем тело функции для показа номеров строк перед установкой точки.
    pub fn get_source(&mut self, listener: i32, target: &str) -> Result<String, postgres::Error> {
        let row = self.client.query_one(
            "SELECT pldbg_get_source($1, $2::text::regprocedure::oid)",
            &[&listener, &target],
        )?;
        row.try_get(0)
    }

    // Ждём подключения target backend и получаем его PID.
    pub fn wait_for_target(&mut self, listener: i32) -> Result<i32, postgres::Error> {
        let row = self
            .client
            .query_one("SELECT pldbg_wait_for_target($1)", &[&listener])?;
        row.try_get(0)
    }

    // Забираем позицию первой остановки после подключения target.
    pub fn wait_for_breakpoint(&mut self, listener: i32) -> Result<Breakpoint, postgres::Error> {
        let row = self.client.query_one(
            "SELECT linenumber, targetname FROM pldbg_wait_for_breakpoint($1)",
            &[&listener],
        )?;
        Ok(Breakpoint {
            line_number: row.try_get("linenumber")?,
            target_name: row.try_get("targetname")?,
        })
    }

    // Получаем кадры стека остановленного вызова.
    pub fn get_stack(&mut self, listener: i32) -> Result<Vec<StackFrame>, postgres::Error> {
        let rows = self.client.query(
            "SELECT level, targetname, linenumber, args FROM pldbg_get_stack($1) ORDER BY level",
            &[&listener],
        )?;
        rows.iter()
            .map(|row| {
                Ok(StackFrame {
                    level: row.try_get("level")?,
                    target_name: row.try_get("targetname")?,
                    line_number: row.try_get("linenumber")?,
                    arguments: row.try_get("args")?,
                })
            })
            .collect()
    }
    // Получаем переменные выбранного кадра стека.
    pub fn get_variables(&mut self, listener: i32) -> Result<Vec<Variable>, postgres::Error> {
        let rows = self.client.query(
            "SELECT name, linenumber, value FROM pldbg_get_variables($1)",
            &[&listener],
        )?;
        rows.iter()
            .map(|row| {
                Ok(Variable {
                    name: row.try_get("name")?,
                    declaration_line: row.try_get("linenumber")?,
                    value: row.try_get("value")?,
                })
            })
            .collect()
    }

    // Выполняем один шаг и ждём следующую остановку.
    pub fn step_over(&mut self, listener: i32) -> Result<DebugEvent, postgres::Error> {
        self.resume(
            "SELECT linenumber, targetname FROM pldbg_step_over($1)",
            listener,
        )
    }

    // Продолжаем выполнение до следующей точки или конца вызова.
    pub fn continue_target(&mut self, listener: i32) -> Result<DebugEvent, postgres::Error> {
        self.resume(
            "SELECT linenumber, targetname FROM pldbg_continue($1)",
            listener,
        )
    }

    // Одинаково обрабатываем ответы после шага и продолжения.
    // Закрытие канала не говорит, чем завершился целевой SQL.
    fn resume(&mut self, query: &str, listener: i32) -> Result<DebugEvent, postgres::Error> {
        let row = match self.client.query_one(query, &[&listener]) {
            Ok(row) => row,
            Err(error) if is_debugger_disconnect(&error) => return Ok(DebugEvent::ChannelClosed),
            Err(error) => return Err(error),
        };

        Ok(DebugEvent::Paused(Breakpoint {
            line_number: row.try_get("linenumber")?,
            target_name: row.try_get("targetname")?,
        }))
    }
}

// Распознаём только известное сообщение плагина о закрытии канала.
fn is_debugger_disconnect(error: &postgres::Error) -> bool {
    error.as_db_error().is_some_and(|db_error| {
        db_error.code().code() == "08006" && db_error.message() == "debugger connection terminated"
    })
}
