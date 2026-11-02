use postgres::{Client, NoTls};
use std::{
    env,
    error::Error,
    fs,
    io::{self, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

// Демонстрационный кластер принадлежит приложению и не затрагивает основную службу PostgreSQL.
pub struct DemoServer {
    pg_home: PathBuf,
    data_dir: PathBuf,
    connection_string: String,
    port: u16,
    running: bool,
}

impl DemoServer {
    // Находим установленный PostgreSQL, готовим отдельный кластер и тестовые функции.
    pub fn start(port: u16) -> Result<Self, Box<dyn Error>> {
        let pg_home = find_pg_home()?;
        let major = postgres_major(&pg_home)?;
        let local_data = env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir);
        let state_dir = local_data.join("pg-debugger");
        let demo_name = format!("demo-pg{major}-{}", env!("CARGO_PKG_VERSION"));
        let data_dir = state_dir.join(&demo_name);
        let log_file = state_dir.join(format!("{demo_name}.log"));
        fs::create_dir_all(&state_dir)?;

        if !data_dir.join("PG_VERSION").exists() {
            if data_dir.exists() && fs::read_dir(&data_dir)?.next().is_some() {
                return Err(io::Error::other(format!(
                    "Папка {} не пуста, но не содержит готового кластера PostgreSQL",
                    data_dir.display()
                ))
                .into());
            }
            println!("Создаю тестовый кластер PostgreSQL {major}...");
            checked_command(
                Command::new(pg_home.join("bin/initdb.exe"))
                    .arg("-D")
                    .arg(&data_dir)
                    .arg("-U")
                    .arg("debugger_test")
                    .arg("-A")
                    .arg("trust")
                    .arg("--no-instructions"),
                "initdb",
            )?;
        }

        let pg_ctl = pg_home.join("bin/pg_ctl.exe");
        let mut running = Command::new(&pg_ctl)
            .arg("-D")
            .arg(&data_dir)
            .arg("status")
            .output()?
            .status
            .success();

        if running {
            let options = fs::read_to_string(data_dir.join("postmaster.opts"))?;
            let expected_port = format!("\"-p\" \"{port}\"");
            if !options.contains(&expected_port)
                || !options.contains("shared_preload_libraries=plugin_debugger")
            {
                println!("Перезапускаю свой тестовый кластер с выбранным портом...");
                checked_command(
                    Command::new(&pg_ctl)
                        .arg("-D")
                        .arg(&data_dir)
                        .arg("-m")
                        .arg("fast")
                        .arg("-w")
                        .arg("stop"),
                    "pg_ctl stop",
                )?;
                running = false;
            }
        }

        if !running {
            // Не подключаемся случайно к чужому серверу на выбранном порту.
            let free_port = TcpListener::bind(("127.0.0.1", port)).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("Порт {port} занят. Выберите другой порт: {error}"),
                )
            })?;
            drop(free_port);

            println!("Запускаю тестовый PostgreSQL на 127.0.0.1:{port}...");
            let options = format!(
                "-p {port} -c listen_addresses=127.0.0.1 -c shared_preload_libraries=plugin_debugger"
            );
            // Сервер остаётся работать после pg_ctl; output() ждал бы закрытия его канала вывода.
            let status = Command::new(&pg_ctl)
                .arg("-D")
                .arg(&data_dir)
                .arg("-l")
                .arg(&log_file)
                .arg("-o")
                .arg(options)
                .arg("-w")
                .arg("start")
                .status()?;
            if !status.success() {
                return Err(io::Error::other(format!(
                    "pg_ctl не запустил тестовый сервер. Подробности: {}",
                    log_file.display()
                ))
                .into());
            }
        }

        let mut server = Self {
            pg_home,
            data_dir,
            connection_string: format!(
                "host=127.0.0.1 port={port} user=debugger_test dbname=postgres"
            ),
            port,
            running: true,
        };
        server.prepare_database()?;
        Ok(server)
    }

    pub fn connection_string(&self) -> &str {
        &self.connection_string
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    // Показываем полный путь, чтобы команда работала и без psql в PATH.
    pub fn psql_path(&self) -> PathBuf {
        self.pg_home.join("bin").join("psql.exe")
    }

    // Запускаем psql как отдельный клиент, но показываем его ответ в нашем окне.
    pub fn launch_psql(&self, sql: &str) -> io::Result<Child> {
        Command::new(self.pg_home.join("bin/psql.exe"))
            .args(["-h", "127.0.0.1", "-p"])
            .arg(self.port.to_string())
            .args([
                "-U",
                "debugger_test",
                "-d",
                "postgres",
                "-X",
                "-v",
                "ON_ERROR_STOP=1",
                "-c",
                sql,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    }

    // Останавливаем только собственный тестовый кластер.
    pub fn shutdown(&mut self) -> Result<(), Box<dyn Error>> {
        if self.running {
            checked_command(
                Command::new(self.pg_home.join("bin/pg_ctl.exe"))
                    .arg("-D")
                    .arg(&self.data_dir)
                    .arg("-m")
                    .arg("fast")
                    .arg("-w")
                    .arg("stop"),
                "pg_ctl stop",
            )?;
            self.running = false;
        }
        Ok(())
    }

    // Сверяем каталог данных до создания расширения, чтобы не изменить чужую БД.
    fn prepare_database(&mut self) -> Result<(), Box<dyn Error>> {
        let mut client = Client::connect(&self.connection_string, NoTls)?;
        let row = client.query_one(
            "SELECT current_setting('data_directory'), current_setting('shared_preload_libraries')",
            &[],
        )?;
        let actual: String = row.try_get(0)?;
        let preload: String = row.try_get(1)?;
        if !same_path(Path::new(&actual), &self.data_dir)? {
            return Err(io::Error::other(format!(
                "На порту {} работает другой PostgreSQL: {}",
                self.port, actual
            ))
            .into());
        }
        if !preload
            .split(',')
            .any(|item| item.trim() == "plugin_debugger")
        {
            return Err(io::Error::other("Тестовый PostgreSQL запущен без plugin_debugger").into());
        }

        client.batch_execute("CREATE EXTENSION IF NOT EXISTS pldbgapi;")?;
        client.batch_execute(include_str!("../debug_smoke.sql"))?;
        client.batch_execute(include_str!("../debug_failure.sql"))?;
        println!("pldbgapi и тестовые функции готовы.");
        Ok(())
    }
}

impl Drop for DemoServer {
    fn drop(&mut self) {
        // При ошибке в меню тоже пытаемся завершить тестовый сервер.
        let _ = self.shutdown();
    }
}

// Пустой ввод оставляет порт 15439, неверное значение просим исправить.
pub fn choose_port() -> io::Result<u16> {
    loop {
        print!("Порт PostgreSQL [15439]: ");
        io::stdout().flush()?;
        let mut input = String::new();
        if io::stdin().read_line(&mut input)? == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Ввод закрыт"));
        }
        let input = input.trim();
        if input.is_empty() {
            return Ok(15439);
        }
        match input.parse::<u16>() {
            Ok(port) if port > 0 => return Ok(port),
            _ => println!("Введите порт от 1 до 65535."),
        }
    }
}

// Ищем установку с отладочным плагином, прежде всего в стандартных папках Windows.
fn find_pg_home() -> Result<PathBuf, Box<dyn Error>> {
    if let Some(home) = env::var_os("PGDEBUG_PG_HOME") {
        let home = PathBuf::from(home);
        if pg_files_present(&home) {
            println!("Использую PostgreSQL из {}", home.display());
            return Ok(home);
        }
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "В {} нет нужных файлов PostgreSQL и pldbgapi",
                home.display()
            ),
        )
        .into());
    }
    let mut homes = Vec::new();
    for key in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(base) = env::var_os(key) {
            let root = PathBuf::from(base).join("PostgreSQL");
            if let Ok(entries) = fs::read_dir(root) {
                homes.extend(entries.flatten().map(|entry| entry.path()));
            }
        }
    }
    if let Ok(output) = Command::new("reg")
        .args([
            "query",
            "HKLM\\SOFTWARE\\PostgreSQL\\Installations",
            "/s",
            "/v",
            "Base Directory",
        ])
        .output()
    {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Some((_, path)) = line.split_once("REG_SZ") {
                homes.push(PathBuf::from(path.trim()));
            }
        }
    }
    if let Ok(output) = Command::new("pg_config").arg("--bindir").output() {
        if output.status.success() {
            let bin = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if let Some(home) = Path::new(&bin).parent() {
                homes.push(home.to_path_buf());
            }
        }
    }
    homes.sort_by(|a, b| b.cmp(a));
    homes.dedup();
    if let Some(home) = homes.into_iter().find(|home| pg_files_present(home)) {
        println!("Использую PostgreSQL из {}", home.display());
        return Ok(home);
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "Не нашёл установленный PostgreSQL с plugin_debugger и pldbgapi. Проверьте, что расширение установлено для этой версии PostgreSQL.",
    )
    .into())
}

fn pg_files_present(home: &Path) -> bool {
    [
        "bin/postgres.exe",
        "bin/initdb.exe",
        "bin/pg_ctl.exe",
        "bin/psql.exe",
        "lib/plugin_debugger.dll",
        "share/extension/pldbgapi.control",
    ]
    .iter()
    .all(|file| home.join(file).is_file())
}

fn postgres_major(home: &Path) -> Result<String, Box<dyn Error>> {
    let output = Command::new(home.join("bin/postgres.exe"))
        .arg("--version")
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other("Не удалось определить версию PostgreSQL").into());
    }
    let output = String::from_utf8_lossy(&output.stdout);
    let major = output
        .split_whitespace()
        .last()
        .and_then(|version| version.split('.').next())
        .filter(|major| major.chars().all(|c| c.is_ascii_digit()))
        .ok_or_else(|| io::Error::other("Неизвестный формат версии PostgreSQL"))?;
    Ok(major.to_owned())
}

fn same_path(first: &Path, second: &Path) -> io::Result<bool> {
    let first = fs::canonicalize(first)?;
    let second = fs::canonicalize(second)?;
    Ok(first
        .to_string_lossy()
        .eq_ignore_ascii_case(&second.to_string_lossy()))
}

fn checked_command(command: &mut Command, label: &str) -> io::Result<()> {
    let output = command.output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "{label} завершился с ошибкой.\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}
