use postgres::{Client, NoTls};
use std::{
    env,
    error::Error,
    fs,
    io::{self, Read, Write},
    net::TcpListener,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

struct PgInstall {
    bin_dir: PathBuf,
    major: String,
}

impl PgInstall {
    fn binary(&self, name: &str) -> PathBuf {
        self.bin_dir.join(name)
    }
}

// Учебный кластер создаётся в каталоге пользователя и не трогает системный PostgreSQL.
pub struct DemoServer {
    install: PgInstall,
    data_dir: PathBuf,
    connection_string: String,
    port: u16,
    running: bool,
    isolated_dir: Option<PathBuf>,
}

impl DemoServer {
    pub fn start(port: u16) -> Result<Self, Box<dyn Error>> {
        let install = find_install()?;
        let state_dir = user_data_dir()?.join("pg-debugger");
        let demo_name = format!("demo-pg{}-{}", install.major, env!("CARGO_PKG_VERSION"));
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
            println!("Создаю тестовый кластер PostgreSQL {}...", install.major);
            checked_command(
                Command::new(install.binary("initdb"))
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

        let pg_ctl = install.binary("pg_ctl");
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
            // Перед стартом ещё раз проверяем порт: между выбором и запуском его могли занять.
            let free_port = TcpListener::bind(("127.0.0.1", port)).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("Порт {port} занят. Выберите другой порт: {error}"),
                )
            })?;
            drop(free_port);

            println!("Запускаю тестовый PostgreSQL на 127.0.0.1:{port}...");
            // Системный каталог Unix-сокетов Debian недоступен обычному пользователю.
            // Учебный кластер принимает только TCP на loopback, поэтому сокет не нужен.
            let options = format!(
                "-p {port} -c listen_addresses=127.0.0.1 -c unix_socket_directories= -c shared_preload_libraries=plugin_debugger"
            );
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
            install,
            data_dir,
            connection_string: format!(
                "host=127.0.0.1 port={port} user=debugger_test dbname=postgres"
            ),
            port,
            running: true,
            isolated_dir: None,
        };
        server.prepare_database()?;
        Ok(server)
    }

    // GUI получает отдельный экземпляр с собственными данными и паролем.
    pub fn start_isolated() -> Result<(Self, String), Box<dyn Error>> {
        let install = find_install()?;
        let mut entropy = [0; 32];
        fs::File::open("/dev/urandom")?.read_exact(&mut entropy)?;
        let hex = |bytes: &[u8]| {
            bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        };
        let password = hex(&entropy[..24]);
        let dir = env::temp_dir().join(format!("pg-debugger-demo-{}", hex(&entropy[24..])));
        fs::DirBuilder::new().mode(0o700).create(&dir)?;
        let data_dir = dir.join("data");
        let log = dir.join("postgres.log");
        let password_path = dir.join("password");
        {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&password_path)?;
            writeln!(file, "{password}")?;
        }
        let init = checked_command(
            Command::new(install.binary("initdb"))
                .arg("-D")
                .arg(&data_dir)
                .args([
                    "-U",
                    "debugger_test",
                    "--auth-host=scram-sha-256",
                    "--auth-local=peer",
                    "--no-instructions",
                ])
                .arg("--pwfile")
                .arg(&password_path),
            "initdb",
        );
        fs::remove_file(&password_path)?;
        init?;

        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        drop(listener);
        let options = format!(
            "-p {port} -c listen_addresses=127.0.0.1 -c unix_socket_directories= -c shared_preload_libraries=plugin_debugger"
        );
        let mut server = Self {
            install,
            data_dir,
            connection_string: format!(
                "host=127.0.0.1 port={port} user=debugger_test dbname=postgres password={password}"
            ),
            port,
            running: true,
            isolated_dir: Some(dir),
        };
        let started = checked_command(
            Command::new(server.install.binary("pg_ctl"))
                .arg("-D")
                .arg(&server.data_dir)
                .arg("-l")
                .arg(&log)
                .arg("-o")
                .arg(options)
                .args(["-w", "-t", "25", "start"]),
            "pg_ctl start",
        );
        if let Err(error) = started {
            // Если сервер ещё запускается после таймаута, guard попытается его остановить.
            // Журнал оставляем для диагностики ошибки запуска.
            server.isolated_dir = None;
            return Err(
                io::Error::other(format!("{error}\nЖурнал PostgreSQL: {}", log.display())).into(),
            );
        }
        server.prepare_database()?;
        Ok((server, password))
    }

    pub fn connection_string(&self) -> &str {
        &self.connection_string
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn psql_path(&self) -> PathBuf {
        self.install.binary("psql")
    }

    pub fn launch_psql(&self, sql: &str) -> io::Result<Child> {
        Command::new(self.psql_path())
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

    pub fn shutdown(&mut self) -> Result<(), Box<dyn Error>> {
        if self.running {
            crate::pg_process::stop(&self.install.binary("pg_ctl"), &self.data_dir)?;
            self.running = false;
        }
        if let Some(dir) = &self.isolated_dir {
            fs::remove_dir_all(dir)?;
            self.isolated_dir = None;
        }
        Ok(())
    }

    fn prepare_database(&mut self) -> Result<(), Box<dyn Error>> {
        let mut client = Client::connect(&self.connection_string, NoTls)?;
        let row = client.query_one(
            "SELECT current_setting('data_directory'), current_setting('shared_preload_libraries')",
            &[],
        )?;
        let actual: String = row.try_get(0)?;
        let preload: String = row.try_get(1)?;
        if fs::canonicalize(&actual)? != fs::canonicalize(&self.data_dir)? {
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
        let _ = self.shutdown();
    }
}

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

fn user_data_dir() -> io::Result<PathBuf> {
    if let Some(base) = env::var_os("XDG_DATA_HOME") {
        let base = PathBuf::from(base);
        if base.is_absolute() {
            return Ok(base);
        }
    }
    env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".local/share"))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "Не задан HOME для учебного кластера",
            )
        })
}

fn find_install() -> Result<PgInstall, Box<dyn Error>> {
    if let Some(home) = env::var_os("PGDEBUG_PG_HOME") {
        let home = PathBuf::from(home);
        return check_install(&home)?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "В {} нет PostgreSQL с plugin_debugger и pldbgapi",
                    home.display()
                ),
            )
            .into()
        });
    }

    let root = Path::new("/usr/lib/postgresql");
    let mut homes: Vec<_> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let major = entry.file_name().to_string_lossy().parse::<u32>().ok()?;
            Some((major, entry.path()))
        })
        .collect();
    homes.sort_by(|left, right| right.0.cmp(&left.0));
    for (_, home) in homes {
        if let Some(install) = check_install(&home)? {
            println!(
                "Использую PostgreSQL {} из {}",
                install.major,
                home.display()
            );
            return Ok(install);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "Не найден PostgreSQL с plugin_debugger и pldbgapi. Установите пакет postgresql-<версия>-pldebugger.",
    )
    .into())
}

fn check_install(home: &Path) -> Result<Option<PgInstall>, Box<dyn Error>> {
    let bin_dir = home.join("bin");
    if !["postgres", "initdb", "pg_ctl", "psql"]
        .iter()
        .all(|name| bin_dir.join(name).is_file())
    {
        return Ok(None);
    }
    let major = postgres_major(&bin_dir.join("postgres"))?;
    let local_share = home.join("share");
    let share_dir = if local_share.join("extension/pldbgapi.control").is_file() {
        local_share
    } else {
        PathBuf::from(format!("/usr/share/postgresql/{major}"))
    };
    if !home.join("lib/plugin_debugger.so").is_file()
        || !share_dir.join("extension/pldbgapi.control").is_file()
    {
        return Ok(None);
    }
    Ok(Some(PgInstall { bin_dir, major }))
}

fn postgres_major(binary: &Path) -> Result<String, Box<dyn Error>> {
    let output = Command::new(binary).arg("--version").output()?;
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
