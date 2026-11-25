//! Подписи и пояснения принадлежат проекту и не изменяют схему PostgreSQL.
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub name: String,
    pub tables: Vec<Table>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Table {
    pub schema: String,
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub description: String,
    pub columns: Vec<Column>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Column {
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub values: Vec<Value>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Value {
    pub value: String,
    pub label: String,
}
fn text(value: &str, limit: usize) -> bool {
    value.len() <= limit && !value.contains('\0')
}
fn name(value: &str) -> bool {
    !value.is_empty() && text(value, 63)
}
impl Model {
    pub fn validate(&self) -> Result<(), String> {
        let invalid =
            || "Неверные настройки модели: проверьте имена, размеры и повторы.".to_owned();
        if !text(&self.name, 128) || self.tables.len() > 200 {
            return Err(invalid());
        }
        let mut tables = HashSet::new();
        let mut total = 0;
        for table in &self.tables {
            total += table.columns.len();
            if !name(&table.schema)
                || !name(&table.name)
                || !text(&table.label, 256)
                || !text(&table.description, 4096)
                || !tables.insert((&table.schema, &table.name))
                || total > 2000
            {
                return Err(invalid());
            }
            let mut columns = HashSet::new();
            for column in &table.columns {
                if !name(&column.name)
                    || !text(&column.label, 256)
                    || !text(&column.description, 4096)
                    || column.values.len() > 100
                    || !columns.insert(&column.name)
                {
                    return Err(invalid());
                }
                let mut values = HashSet::new();
                for value in &column.values {
                    if !text(&value.value, 512)
                        || !text(&value.label, 256)
                        || !values.insert(&value.value)
                    {
                        return Err(invalid());
                    }
                }
            }
        }
        Ok(())
    }
}
