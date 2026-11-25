//! Состояние SELECT-помощника переносится по именам, без OID и учётных данных.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub schema: String,
    pub table: Option<String>,
    pub mode: Mode,
    pub column: Option<Field>,
    pub filter: Option<Filter>,
    pub join: Option<Link>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub joins: Vec<JoinStep>,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Value,
    Count,
    Sum,
    Min,
    Max,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Source,
    Joined,
    #[serde(rename = "join_2")]
    Join2,
    #[serde(rename = "join_3")]
    Join3,
    #[serde(rename = "join_4")]
    Join4,
}
impl Side {
    fn index(self) -> usize {
        match self {
            Self::Source => 0,
            Self::Joined => 1,
            Self::Join2 => 2,
            Self::Join3 => 3,
            Self::Join4 => 4,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinStep {
    pub from: Side,
    pub link: Link,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub side: Side,
    pub name: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    pub side: Side,
    pub name: String,
    pub parameter: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Link {
    pub constraint: String,
    pub kind: JoinKind,
    pub referenced_schema: String,
    pub referenced_table: String,
    pub columns: Vec<String>,
    pub referenced_columns: Vec<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JoinKind {
    Inner,
    Left,
}
fn name(value: &str) -> bool {
    !value.is_empty() && value.len() <= 63 && !value.contains('\0')
}
impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        let last = usize::from(self.join.is_some()) + self.joins.len();
        let valid_link = |j: &Link| {
            name(&j.constraint)
                && name(&j.referenced_schema)
                && name(&j.referenced_table)
                && !j.columns.is_empty()
                && j.columns.len() <= 1600
                && j.columns.len() == j.referenced_columns.len()
                && j.columns
                    .iter()
                    .chain(&j.referenced_columns)
                    .all(|v| name(v))
        };
        let valid = name(&self.schema)
            && self.table.as_deref().is_none_or(name)
            && self
                .column
                .as_ref()
                .is_none_or(|f| name(&f.name) && f.side.index() <= last)
            && self.filter.as_ref().is_none_or(|f| {
                name(&f.name) && f.parameter.as_deref().is_none_or(name) && f.side.index() <= last
            })
            && self.join.as_ref().is_none_or(valid_link)
            && (self.joins.is_empty() || self.join.is_some())
            && self.joins.len() <= 3
            && self
                .joins
                .iter()
                .enumerate()
                .all(|(i, step)| step.from.index() <= i + 1 && valid_link(&step.link));
        if valid {
            Ok(())
        } else {
            Err("Неверные настройки SELECT-помощника.".into())
        }
    }
}
