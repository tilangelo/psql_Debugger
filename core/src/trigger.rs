//! Подготовка одного DML-оператора, вызывающего выбранную триггерную функцию.
use postgres::{Error, GenericClient, Transaction, types::ToSql};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum Operation {
    Insert,
    Update,
    Delete,
}

impl Operation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Insert => "INSERT",
            Self::Update => "UPDATE",
            Self::Delete => "DELETE",
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct TriggerColumn {
    pub name: String,
    pub type_name: String,
    pub writable: bool,
    #[serde(skip)]
    number: i16,
    #[serde(skip)]
    type_oid: u32,
    #[serde(skip)]
    cast: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct TriggerTarget {
    pub oid: u32,
    pub name: String,
    pub schema: String,
    pub table: String,
    pub definition: String,
    pub operations: Vec<Operation>,
    pub columns: Vec<TriggerColumn>,
    pub primary_key: Vec<String>,
    #[serde(skip)]
    relation: u32,
    #[serde(skip)]
    update_of: Vec<i16>,
}

pub fn targets(client: &mut impl GenericClient, routine: u32) -> Result<Vec<TriggerTarget>, Error> {
    let rows = client.query(
        "WITH active AS (
            SELECT * FROM pg_catalog.pg_trigger
            WHERE tgfoid = $1 AND (tgenabled = 'A'
                OR (tgenabled = 'O' AND pg_catalog.current_setting('session_replication_role') <> 'replica')
                OR (tgenabled = 'R' AND pg_catalog.current_setting('session_replication_role') = 'replica'))
         )
         SELECT t.oid, t.tgname::text, n.nspname::text, c.relname::text, c.oid,
                pg_catalog.pg_get_triggerdef(t.oid), t.tgtype, t.tgattr::smallint[],
                COALESCE((SELECT pg_catalog.bit_or(other.tgtype) FROM active other
                    WHERE other.tgrelid = t.tgrelid AND other.oid <> t.oid), 0)::smallint,
                COALESCE((SELECT conkey FROM pg_catalog.pg_constraint
                    WHERE conrelid = c.oid AND contype = 'p'), ARRAY[]::smallint[])
         FROM active t
         JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
         WHERE c.relkind = 'r' AND NOT c.relispartition AND NOT t.tgisinternal
           AND NOT t.tgdeferrable AND (t.tgtype & 1) <> 0
           AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
         ORDER BY n.nspname, c.relname, t.tgname",
        &[&routine],
    )?;
    let mut result = Vec::new();
    for row in rows {
        let flags: i16 = row.try_get(6)?;
        let ambiguous: i16 = row.try_get(8)?;
        let key_numbers: Vec<i16> = row.try_get(9)?;
        // Биты событий определены в pg_trigger.h PostgreSQL.
        let operations: Vec<_> = [
            (Operation::Insert, 4),
            (Operation::Update, 16),
            (Operation::Delete, 8),
        ]
        .into_iter()
        .filter(|(operation, bit)| {
            flags & bit != 0
                && ambiguous & bit == 0
                && (*operation == Operation::Insert || !key_numbers.is_empty())
        })
        .map(|(operation, _)| operation)
        .collect();
        if operations.is_empty() {
            continue;
        }
        let relation: u32 = row.try_get(4)?;
        let columns: Vec<_> = client
            .query(
                "SELECT a.attname::text, pg_catalog.format_type(a.atttypid, a.atttypmod),
                    a.attgenerated = '' AND a.attidentity <> 'a', a.attnum, a.atttypid,
                    ns.nspname::text, ty.typname::text
             FROM pg_catalog.pg_attribute a
             JOIN pg_catalog.pg_type ty ON ty.oid = a.atttypid
             JOIN pg_catalog.pg_namespace ns ON ns.oid = ty.typnamespace
             WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum",
                &[&relation],
            )?
            .into_iter()
            .map(|column| {
                let schema: String = column.try_get(5)?;
                let name: String = column.try_get(6)?;
                Ok(TriggerColumn {
                    name: column.try_get(0)?,
                    type_name: column.try_get(1)?,
                    writable: column.try_get(2)?,
                    number: column.try_get(3)?,
                    type_oid: column.try_get(4)?,
                    cast: qualified(&schema, &name),
                })
            })
            .collect::<Result<_, Error>>()?;
        let primary_key = key_numbers
            .iter()
            .filter_map(|number| {
                columns
                    .iter()
                    .find(|column| column.number == *number)
                    .map(|column| column.name.clone())
            })
            .collect();
        result.push(TriggerTarget {
            oid: row.try_get(0)?,
            name: row.try_get(1)?,
            schema: row.try_get(2)?,
            table: row.try_get(3)?,
            definition: row.try_get(5)?,
            update_of: row.try_get(7)?,
            relation,
            operations,
            columns,
            primary_key,
        });
    }
    Ok(result)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriggerInput {
    pub oid: u32,
    pub expected_definition: String,
    pub operation: Operation,
    #[serde(default)]
    pub values: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub key: BTreeMap<String, String>,
}

pub struct TriggerCall {
    target: TriggerTarget,
    operation: Operation,
    query: String,
    params: Vec<Option<String>>,
}

impl TriggerCall {
    pub fn new(target: TriggerTarget, input: TriggerInput) -> Result<Self, &'static str> {
        let operation = input.operation;
        if !target.operations.contains(&operation) {
            return Err("Операция недоступна для выбранного триггера.");
        }
        if operation == Operation::Insert && !input.key.is_empty()
            || operation != Operation::Insert
                && (input.key.len() != target.primary_key.len()
                    || !target
                        .primary_key
                        .iter()
                        .all(|name| input.key.contains_key(name)))
        {
            return Err("Для UPDATE и DELETE заполните все столбцы первичного ключа.");
        }
        if operation == Operation::Delete && !input.values.is_empty()
            || operation == Operation::Update && input.values.is_empty()
        {
            return Err("Для UPDATE укажите изменяемые поля. Для DELETE нужен только ключ.");
        }
        let mut params = Vec::new();
        let mut add = |value: Option<String>, column: &TriggerColumn| {
            params.push(value);
            format!("(${}::pg_catalog.text)::{}", params.len(), column.cast)
        };
        let mut values = Vec::new();
        let mut matches_update_of = target.update_of.is_empty();
        for (name, value) in input.values {
            let column = target
                .columns
                .iter()
                .find(|column| column.name == name && column.writable)
                .ok_or("Указано неизвестное поле или поле, недоступное для записи.")?;
            matches_update_of |= target.update_of.contains(&column.number);
            values.push((quote(&name), add(value, column)));
        }
        if operation == Operation::Update && !matches_update_of {
            return Err("Укажите хотя бы одно поле из списка UPDATE OF выбранного триггера.");
        }
        let mut terms = Vec::new();
        for (name, value) in input.key {
            let column = target
                .columns
                .iter()
                .find(|column| column.name == name)
                .ok_or("Первичный ключ таблицы изменился.")?;
            terms.push(format!(
                "target.{} = {}",
                quote(&name),
                add(Some(value), column)
            ));
        }
        let table = qualified(&target.schema, &target.table);
        let query = match operation {
            Operation::Insert if values.is_empty() => format!("INSERT INTO {table} DEFAULT VALUES"),
            Operation::Insert => {
                let (columns, expressions): (Vec<_>, Vec<_>) = values.into_iter().unzip();
                format!(
                    "INSERT INTO {table} ({}) VALUES ({})",
                    columns.join(", "),
                    expressions.join(", ")
                )
            }
            Operation::Update => {
                let assignments: Vec<_> = values
                    .into_iter()
                    .map(|(name, expr)| format!("{name} = {expr}"))
                    .collect();
                format!(
                    "UPDATE ONLY {table} AS target SET {} WHERE {}",
                    assignments.join(", "),
                    terms.join(" AND ")
                )
            }
            Operation::Delete => format!(
                "DELETE FROM ONLY {table} AS target WHERE {}",
                terms.join(" AND ")
            ),
        };
        Ok(Self {
            target,
            operation,
            query,
            params,
        })
    }

    pub(crate) fn validate(
        &self,
        transaction: &mut Transaction<'_>,
        function: u32,
    ) -> Result<bool, Error> {
        let table = qualified(&self.target.schema, &self.target.table);
        transaction.batch_execute(&format!("LOCK TABLE ONLY {table} IN ROW EXCLUSIVE MODE"))?;
        Ok(targets(transaction, function)?.contains(&self.target))
    }

    pub(crate) fn operation(&self) -> Operation {
        self.operation
    }

    pub(crate) fn execute(&self, transaction: &mut Transaction<'_>) -> Result<u64, Error> {
        let params: Vec<&(dyn ToSql + Sync)> = self.params.iter().map(|value| value as _).collect();
        transaction.execute(&self.query, &params)
    }
}

fn qualified(schema: &str, name: &str) -> String {
    format!("{}.{}", quote(schema), quote(name))
}

fn quote(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}
