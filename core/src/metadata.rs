//! Физическая структура базы. Читаем каталог от имени текущего пользователя, без строк таблиц.
use crate::catalog::{Connection, RequestError};
use postgres::{IsolationLevel, Transaction};
use serde::Serialize;

#[derive(Serialize)]
pub struct Relation {
    pub oid: u32,
    pub schema: String,
    pub name: String,
    pub kind: String,
    pub comment: Option<String>,
    pub can_select_all: bool,
}

#[derive(Serialize)]
pub struct Column {
    pub number: i16,
    pub name: String,
    pub data_type: String,
    pub type_oid: u32,
    pub nullable: bool,
    pub comment: Option<String>,
    pub can_update: bool,
}

#[derive(Serialize)]
pub struct PrimaryKey {
    pub name: String,
    pub columns: Vec<String>,
}

#[derive(Serialize)]
pub struct ForeignKey {
    pub name: String,
    pub columns: Vec<String>,
    pub referenced_oid: u32,
    pub referenced_schema: String,
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
}

#[derive(Serialize)]
pub struct Details {
    pub oid: u32,
    pub columns: Vec<Column>,
    pub primary_keys: Vec<PrimaryKey>,
    pub foreign_keys: Vec<ForeignKey>,
}

#[derive(Serialize)]
pub struct Metadata {
    pub schemas: Vec<String>,
    pub schema: Option<String>,
    pub relations: Vec<Relation>,
    pub selected: Option<Details>,
}

fn invalid(message: &'static str) -> RequestError {
    RequestError {
        message,
        detail: String::new(),
    }
}

/// Один согласованный снимок; старый OID и недоступная схема не подменяются другим объектом.
pub fn browse(
    connection: &Connection,
    schema: Option<&str>,
    relation_oid: Option<u32>,
) -> Result<Metadata, RequestError> {
    let mut client = connection
        .connect()
        .map_err(|e| connection.request_error(e, true))?;
    let mut tx = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .map_err(|e| connection.request_error(e, false))?;
    let rows = tx
        .query(
            "SELECT nspname::text FROM pg_catalog.pg_namespace
         WHERE nspname !~ '^pg_' AND nspname <> 'information_schema'
           AND pg_catalog.has_schema_privilege(oid, 'USAGE') ORDER BY nspname LIMIT 501",
            &[],
        )
        .map_err(|e| connection.request_error(e, false))?;
    if rows.len() > 500 {
        return Err(invalid(
            "Слишком много доступных схем: максимум 500 для этого просмотра.",
        ));
    }
    let schemas = rows
        .iter()
        .map(|r| r.get::<_, String>(0))
        .collect::<Vec<_>>();
    let selected_schema = match schema {
        Some(name) if schemas.iter().any(|s| s == name) => Some(name.to_owned()),
        Some(_) => {
            return Err(invalid(
                "Схема не найдена или нет права USAGE. Обновите каталог.",
            ));
        }
        None => schemas
            .iter()
            .find(|s| s.as_str() == "public")
            .or(schemas.first())
            .cloned(),
    };
    let relations = match &selected_schema {
        Some(schema) => {
            relations(&mut tx, schema).map_err(|e| connection.request_error(e, false))?
        }
        None => Vec::new(),
    };
    if relations.len() > 2000 {
        return Err(invalid(
            "В схеме больше 2000 доступных таблиц и представлений; просмотр ограничен.",
        ));
    }
    let selected = match relation_oid {
        Some(oid) => {
            if !relations.iter().any(|r| r.oid == oid) {
                return Err(invalid(
                    "Таблица не найдена в выбранной схеме или нет прав SELECT. Обновите структуру.",
                ));
            }
            Some(details(&mut tx, oid).map_err(|e| connection.request_error(e, false))?)
        }
        None => None,
    };
    tx.commit()
        .map_err(|e| connection.request_error(e, false))?;
    Ok(Metadata {
        schemas,
        schema: selected_schema,
        relations,
        selected,
    })
}

fn relations(tx: &mut Transaction<'_>, schema: &str) -> Result<Vec<Relation>, postgres::Error> {
    tx.query(
        "SELECT c.oid, n.nspname::text, c.relname::text, c.relkind::text,
                pg_catalog.left(pg_catalog.obj_description(c.oid, 'pg_class'), 4096),
                pg_catalog.has_table_privilege(c.oid, 'SELECT')
         FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname::text = $1 AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
           AND (pg_catalog.has_table_privilege(c.oid, 'SELECT')
                OR pg_catalog.has_any_column_privilege(c.oid, 'SELECT'))
         ORDER BY c.relname, c.oid LIMIT 2001",
        &[&schema],
    )?
    .iter()
    .map(|row| {
        Ok(Relation {
            oid: row.try_get(0)?,
            schema: row.try_get(1)?,
            name: row.try_get(2)?,
            kind: row.try_get(3)?,
            comment: row.try_get(4)?,
            can_select_all: row.try_get(5)?,
        })
    })
    .collect()
}

fn details(tx: &mut Transaction<'_>, oid: u32) -> Result<Details, postgres::Error> {
    let columns = tx.query(
        "SELECT a.attnum, a.attname::text, pg_catalog.format_type(a.atttypid, a.atttypmod),
                a.atttypid, NOT (a.attnotnull OR t.typnotnull), pg_catalog.left(pg_catalog.col_description(a.attrelid, a.attnum), 4096),
                pg_catalog.has_column_privilege(a.attrelid, a.attnum, 'UPDATE')
         FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_type t ON t.oid = a.atttypid
         WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped
           AND pg_catalog.has_column_privilege(a.attrelid, a.attnum, 'SELECT') ORDER BY a.attnum", &[&oid])?
        .iter().map(|row| Ok(Column {
            number: row.try_get(0)?, name: row.try_get(1)?, data_type: row.try_get(2)?,
            type_oid: row.try_get(3)?, nullable: row.try_get(4)?, comment: row.try_get(5)?, can_update: row.try_get(6)?,
        })).collect::<Result<Vec<_>, postgres::Error>>()?;
    // Составные ключи сохраняют порядок колонок. Скрытые колонки в подсказки не попадают.
    let primary_keys = tx.query(
        "SELECT c.conname::text, ARRAY(
             SELECT a.attname::text FROM pg_catalog.unnest(c.conkey) WITH ORDINALITY k(num, pos)
             JOIN pg_catalog.pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.num ORDER BY k.pos)
         FROM pg_catalog.pg_constraint c WHERE c.conrelid = $1 AND c.contype = 'p'
           AND NOT EXISTS (SELECT FROM pg_catalog.unnest(c.conkey) k(num)
             WHERE NOT pg_catalog.has_column_privilege(c.conrelid, k.num, 'SELECT')) ORDER BY c.conname", &[&oid])?
        .iter().map(|r| Ok(PrimaryKey { name: r.try_get(0)?, columns: r.try_get(1)? }))
        .collect::<Result<Vec<_>, postgres::Error>>()?;
    let foreign_keys = tx.query(
        "SELECT c.conname::text,
             ARRAY(SELECT a.attname::text FROM pg_catalog.unnest(c.conkey) WITH ORDINALITY k(num, pos)
               JOIN pg_catalog.pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.num ORDER BY k.pos),
             c.confrelid, n.nspname::text, r.relname::text,
             ARRAY(SELECT a.attname::text FROM pg_catalog.unnest(c.confkey) WITH ORDINALITY k(num, pos)
               JOIN pg_catalog.pg_attribute a ON a.attrelid = c.confrelid AND a.attnum = k.num ORDER BY k.pos)
         FROM pg_catalog.pg_constraint c JOIN pg_catalog.pg_class r ON r.oid = c.confrelid
         JOIN pg_catalog.pg_namespace n ON n.oid = r.relnamespace
         WHERE c.conrelid = $1 AND c.contype = 'f' AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
           AND NOT EXISTS (SELECT FROM pg_catalog.unnest(c.conkey) WITH ORDINALITY k(src, pos)
             WHERE NOT pg_catalog.has_column_privilege(c.conrelid, k.src, 'SELECT')
                OR NOT pg_catalog.has_column_privilege(c.confrelid, c.confkey[k.pos::int], 'SELECT')) ORDER BY c.conname", &[&oid])?
        .iter().map(|r| Ok(ForeignKey {
            name: r.try_get(0)?, columns: r.try_get(1)?, referenced_oid: r.try_get(2)?,
            referenced_schema: r.try_get(3)?, referenced_table: r.try_get(4)?, referenced_columns: r.try_get(5)?,
        })).collect::<Result<Vec<_>, postgres::Error>>()?;
    Ok(Details {
        oid,
        columns,
        primary_keys,
        foreign_keys,
    })
}
