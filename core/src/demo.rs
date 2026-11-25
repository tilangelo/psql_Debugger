//! Тестовые объекты создаются только в новой базе, выбранная рабочая база не заполняется.
use crate::catalog::{self, Catalog, Connection, Selection};
#[cfg(target_os = "linux")]
pub use pg_debugger::demo_linux::DemoServer;

pub struct Created {
    pub connection: Connection,
    pub catalog: Catalog,
}

pub fn create(connection: &Connection, name: &str) -> Result<Created, String> {
    // Имя приходит от транспорта, но библиотека тоже запрещает произвольные SQL-идентификаторы.
    let suffix = name.strip_prefix("pgdebug_demo_").unwrap_or("");
    if suffix.len() != 16 || !suffix.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Некорректное имя новой тестовой базы.".into());
    }
    let error = |err| {
        let err = connection.request_error(err, false);
        format!("{}\n{}", err.message, err.detail)
    };
    let mut admin = connection.connect().map_err(|err| {
        let err = connection.request_error(err, true);
        format!("{}\n{}", err.message, err.detail)
    })?;
    let state = admin.query_one(
        "SELECT r.rolsuper, EXISTS (SELECT FROM pg_catalog.pg_available_extensions WHERE name = 'pldbgapi'),
                (SELECT setting FROM pg_catalog.pg_settings WHERE name = 'shared_preload_libraries')
         FROM pg_catalog.pg_roles r WHERE r.rolname = current_user", &[]).map_err(error)?;
    if !state.get::<_, bool>(0) {
        return Err("Для подготовки новой тестовой базы подключитесь от администратора PostgreSQL: в ней потребуется создать расширение pldbgapi. Обычное подключение к своей базе остаётся доступным.".into());
    }
    if !state.get::<_, bool>(1) {
        return Err("На сервере не установлен pldbgapi. Установите совместимый с версией PostgreSQL пакет pldebugger и повторите подготовку. Новая база не создавалась.".into());
    }
    if !state
        .get::<_, Option<&str>>(2)
        .is_some_and(catalog::plugin_configured)
    {
        return Err("Для тестовой отладки добавьте plugin_debugger в shared_preload_libraries и перезапустите PostgreSQL. Программа не меняет настройку сервера. Новая база не создавалась.".into());
    }
    admin
        .batch_execute("SET statement_timeout = '30s'")
        .map_err(error)?;
    admin
        .batch_execute(&format!("CREATE DATABASE \"{name}\" TEMPLATE template0"))
        .map_err(|err| {
            format!(
                "Создание базы {name} не подтверждено. Проверьте список баз перед повтором.\n{}",
                error(err)
            )
        })?;

    let mut target = connection.clone();
    target.database = name.into();
    let initialized = (|| {
        let mut client = target.connect().map_err(error)?;
        // Оба встроенных примера и расширение фиксируются вместе, без исполнения файлов с диска.
        let mut transaction = client.transaction().map_err(error)?;
        transaction
            .batch_execute("CREATE EXTENSION pldbgapi")
            .map_err(error)?;
        transaction
            .batch_execute(include_str!("../../debug_smoke.sql"))
            .map_err(error)?;
        let shop = include_str!("../../examples/shop/schema.sql").replace("\r\n", "\n");
        // Убираем только внешнюю транзакцию встроенного файла; тела процедур не переписываем.
        let shop = shop
            .split_once("\nBEGIN;\n")
            .and_then(|(_, body)| body.trim_end().strip_suffix("COMMIT;"))
            .ok_or("У встроенного SQL-примера изменился формат транзакции.")?;
        transaction.batch_execute(shop).map_err(error)?;
        transaction.commit().map_err(error)?;
        browse(&target)
    })();
    let catalog = initialized.map_err(|err| format!(
        "База {name} создана, но подготовка или проверка примера не завершилась. Она не удалялась автоматически. Подключитесь к ней для проверки; рабочая база не заполнялась.\n{err}"))?;
    Ok(Created {
        connection: target,
        catalog,
    })
}

pub fn browse(connection: &Connection) -> Result<Catalog, String> {
    let selection = Selection {
        schema: Some("public".into()),
        routine_oid: None,
    };
    let error = |err: catalog::RequestError| format!("{}\n{}", err.message, err.detail);
    let catalog = catalog::browse(connection, &selection).map_err(error)?;
    let routine = catalog
        .routines
        .iter()
        .find(|routine| routine.name == "debug_smoke")
        .ok_or("Тестовая функция не найдена.")?;
    catalog::browse(
        connection,
        &Selection {
            routine_oid: Some(routine.oid),
            ..selection
        },
    )
    .map_err(error)
}
