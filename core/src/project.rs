//! Серверная модель конструктора. Входной SQL от браузера не используется как DDL.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Project {
    pub schema_version: u32,
    pub kind: Kind,
    pub schema: String,
    pub name: String,
    pub return_type: DataType,
    pub parameters: Vec<Parameter>,
    pub variables: Vec<Variable>,
    pub blocks: Vec<Block>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<crate::hints::Model>,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Function,
    Procedure,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DataType {
    Builtin(BuiltinType),
    Named(NamedType),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedType {
    pub schema: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub array: bool,
}
impl NamedType {
    pub(crate) fn valid(&self) -> bool {
        [&self.schema, &self.name].iter().all(|value| {
            !value.is_empty() && value.len() <= 63 && !value.chars().any(char::is_control)
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuiltinType {
    Integer,
    Text,
    Boolean,
    Bigint,
    Numeric,
    Date,
    Timestamp,
    Uuid,
    Smallint,
    Real,
    #[serde(rename = "double precision")]
    Double,
    Varchar,
    Json,
    Jsonb,
    Timestamptz,
    Time,
    Interval,
    Bytea,
}
impl DataType {
    pub fn oid(&self) -> Option<u32> {
        match self {
            Self::Builtin(ty) => Some(ty.info().1),
            Self::Named(_) => None,
        }
    }
    fn qualified(&self) -> String {
        match self {
            Self::Builtin(ty) => {
                let name =
                    postgres::types::Type::from_oid(ty.info().1).expect("known PostgreSQL type");
                format!("pg_catalog.{}", name.name())
            }
            Self::Named(ty) => format!(
                "{}.{}{}",
                quote(&ty.schema),
                quote(&ty.name),
                if ty.array { "[]" } else { "" }
            ),
        }
    }
}
impl BuiltinType {
    pub fn info(self) -> (&'static str, u32) {
        match self {
            Self::Integer => ("integer", 23),
            Self::Text => ("text", 25),
            Self::Boolean => ("boolean", 16),
            Self::Bigint => ("bigint", 20),
            Self::Numeric => ("numeric", 1700),
            Self::Date => ("date", 1082),
            Self::Timestamp => ("timestamp", 1114),
            Self::Uuid => ("uuid", 2950),
            Self::Smallint => ("smallint", 21),
            Self::Real => ("real", 700),
            Self::Double => ("double precision", 701),
            Self::Varchar => ("varchar", 1043),
            Self::Json => ("json", 114),
            Self::Jsonb => ("jsonb", 3802),
            Self::Timestamptz => ("timestamptz", 1184),
            Self::Time => ("time", 1083),
            Self::Interval => ("interval", 1186),
            Self::Bytea => ("bytea", 17),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub name: String,
    #[serde(rename = "type")]
    pub data_type: DataType,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variable {
    pub name: String,
    #[serde(rename = "type")]
    pub data_type: DataType,
    #[serde(default)]
    pub initial: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Block {
    Assign {
        id: String,
        target: String,
        expression: String,
    },
    Select {
        id: String,
        target: String,
        query: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        assistant: Option<crate::assistant::Settings>,
    },
    If {
        id: String,
        condition: String,
        then: Vec<Block>,
        r#else: Vec<Block>,
    },
    While {
        id: String,
        condition: String,
        body: Vec<Block>,
    },
    For {
        id: String,
        iterator: String,
        lower: String,
        upper: String,
        body: Vec<Block>,
    },
    Return {
        id: String,
        #[serde(default)]
        expression: String,
    },
    Sql {
        id: String,
        sql: String,
    },
}
impl Block {
    fn id(&self) -> &str {
        match self {
            Self::Assign { id, .. }
            | Self::Select { id, .. }
            | Self::If { id, .. }
            | Self::While { id, .. }
            | Self::For { id, .. }
            | Self::Return { id, .. }
            | Self::Sql { id, .. } => id,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lines {
    pub body_start: usize,
    pub body_end: usize,
    pub start: usize,
    pub end: usize,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Generated {
    pub sql: String,
    pub body: String,
    pub signature: String,
    pub source_map: BTreeMap<String, Lines>,
}
fn identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    value.len() <= 63
        && bytes
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}
fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
fn expr(value: &str) -> &str {
    value.trim().trim_end_matches(';').trim_end()
}
impl Project {
    pub(crate) fn named_types(&self) -> Vec<String> {
        self.types()
            .filter_map(|ty| match ty {
                DataType::Named(_) => Some(ty.qualified()),
                DataType::Builtin(_) => None,
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    fn types(&self) -> impl Iterator<Item = &DataType> {
        std::iter::once(&self.return_type)
            .filter(|_| self.kind == Kind::Function)
            .chain(self.parameters.iter().map(|p| &p.data_type))
            .chain(self.variables.iter().map(|v| &v.data_type))
    }

    pub fn generate(&self) -> Result<Generated, String> {
        if !matches!(self.schema_version, 1 | 2 | 3)
            || !identifier(&self.schema)
            || !identifier(&self.name)
            || (self.schema_version == 1 && self.model.is_some())
            || (self.schema_version >= 2 && self.model.is_none())
        {
            return Err("Неверная версия проекта, схема или имя объекта.".into());
        }
        if let Some(model) = &self.model {
            model.validate()?;
        }
        if self.parameters.len() > 100 || self.variables.len() > 100 {
            return Err("Допускается до 100 параметров и переменных.".into());
        }
        if self
            .types()
            .any(|ty| matches!(ty, DataType::Named(name) if !name.valid()))
        {
            return Err("Некорректная ссылка на тип PostgreSQL.".into());
        }
        let mut names = HashSet::new();
        for name in self
            .parameters
            .iter()
            .map(|p| &p.name)
            .chain(self.variables.iter().map(|v| &v.name))
        {
            if !identifier(name) || !names.insert(name) {
                return Err(
                    "Имена параметров и переменных должны быть корректными и уникальными.".into(),
                );
            }
        }
        fn nonempty(value: &str) -> Result<(), String> {
            if value.trim().is_empty() {
                Err("Заполните выражение или SQL блока.".into())
            } else {
                Ok(())
            }
        }
        fn check(
            list: &[Block],
            project: &Project,
            depth: usize,
            ids: &mut HashSet<String>,
        ) -> Result<(), String> {
            if depth > 12 {
                return Err("Слишком глубокие ветви (максимум 12).".into());
            }
            for b in list {
                if let Block::Select {
                    assistant: Some(settings),
                    ..
                } = b
                {
                    if project.schema_version < 3 {
                        return Err("Настройки SELECT требуют версию проекта 3.".into());
                    }
                    settings.validate()?;
                }
                if b.id().is_empty()
                    || b.id().len() > 128
                    || !ids.insert(b.id().to_owned())
                    || ids.len() > 500
                {
                    return Err("Допускается до 500 блоков с уникальными идентификаторами.".into());
                }
                match b {
                    Block::Assign {
                        target, expression, ..
                    }
                    | Block::Select {
                        target,
                        query: expression,
                        ..
                    } => {
                        if !project.variables.iter().any(|v| v.name == *target) {
                            return Err(
                                "Результат блока должен записываться в объявленную переменную."
                                    .into(),
                            );
                        }
                        nonempty(expression)?;
                        if matches!(b, Block::Select { .. })
                            && !expression
                                .trim()
                                .split_whitespace()
                                .next()
                                .is_some_and(|s| s.eq_ignore_ascii_case("SELECT"))
                        {
                            return Err("Запрос должен начинаться с SELECT.".into());
                        }
                    }
                    Block::If {
                        condition,
                        then,
                        r#else,
                        ..
                    } => {
                        nonempty(condition)?;
                        check(then, project, depth + 1, ids)?;
                        check(r#else, project, depth + 1, ids)?;
                    }
                    Block::While {
                        condition, body, ..
                    } => {
                        nonempty(condition)?;
                        check(body, project, depth + 1, ids)?;
                    }
                    Block::For {
                        iterator,
                        lower,
                        upper,
                        body,
                        ..
                    } => {
                        if !identifier(iterator) {
                            return Err("Укажите корректное имя счётчика цикла FOR.".into());
                        }
                        nonempty(lower)?;
                        nonempty(upper)?;
                        check(body, project, depth + 1, ids)?;
                    }
                    Block::Return { expression, .. } if project.kind == Kind::Function => {
                        nonempty(expression)?
                    }
                    Block::Sql { sql, .. } => nonempty(sql)?,
                    _ => {}
                }
            }
            Ok(())
        }
        check(&self.blocks, self, 0, &mut HashSet::new())?;
        fn returns(list: &[Block]) -> bool {
            list.iter().any(|b| match b {
                Block::Return { .. } => true,
                Block::If { then, r#else, .. } => returns(then) && returns(r#else),
                _ => false,
            })
        }
        if self.kind == Kind::Function && !returns(&self.blocks) {
            return Err("Добавьте RETURN для всех путей функции.".into());
        }
        let mut body = Vec::new();
        if !self.variables.is_empty() {
            body.push("DECLARE".into());
            for v in &self.variables {
                let declaration = format!(
                    "    {} {}{};",
                    quote(&v.name),
                    v.data_type.qualified(),
                    if v.initial.trim().is_empty() {
                        String::new()
                    } else {
                        format!(" := {}", v.initial)
                    }
                );
                body.extend(declaration.split('\n').map(str::to_owned));
            }
        }
        body.push("BEGIN".into());
        let mut source_map = BTreeMap::new();
        fn push_line(body: &mut Vec<String>, line: String) {
            body.extend(line.split('\n').map(str::to_owned));
        }
        fn emit(
            list: &[Block],
            depth: usize,
            kind: Kind,
            body: &mut Vec<String>,
            map: &mut BTreeMap<String, Lines>,
        ) {
            let pad = "    ".repeat(depth);
            if list.is_empty() {
                push_line(body, format!("{pad}NULL;"));
            }
            for block in list {
                let start = body.len() + 1;
                match block {
                    Block::Assign {
                        target, expression, ..
                    } => push_line(
                        body,
                        format!("{pad}{} := {};", quote(target), expr(expression)),
                    ),
                    Block::Select { target, query, .. } => {
                        push_line(body, format!("{pad}SELECT * INTO {}", quote(target)));
                        push_line(body, format!("{pad}FROM ("));
                        body.extend(expr(query).lines().map(|s| format!("{pad}    {s}")));
                        push_line(body, format!("{pad}) AS selected_value;"));
                    }
                    Block::Sql { sql, .. } => {
                        let lines = expr(sql).lines().collect::<Vec<_>>();
                        for (i, line) in lines.iter().enumerate() {
                            push_line(
                                body,
                                format!(
                                    "{pad}{line}{}",
                                    if i + 1 == lines.len() { ";" } else { "" }
                                ),
                            );
                        }
                    }
                    Block::Return { expression, .. } => push_line(
                        body,
                        format!(
                            "{pad}RETURN{};",
                            if kind == Kind::Function {
                                format!(" {}", expr(expression))
                            } else {
                                String::new()
                            }
                        ),
                    ),
                    Block::If {
                        condition,
                        then,
                        r#else,
                        ..
                    } => {
                        push_line(body, format!("{pad}IF {} THEN", expr(condition)));
                        emit(then, depth + 1, kind, body, map);
                        push_line(body, format!("{pad}ELSE"));
                        emit(r#else, depth + 1, kind, body, map);
                        push_line(body, format!("{pad}END IF;"));
                    }
                    Block::While {
                        condition,
                        body: children,
                        ..
                    } => {
                        push_line(body, format!("{pad}WHILE {} LOOP", expr(condition)));
                        emit(children, depth + 1, kind, body, map);
                        push_line(body, format!("{pad}END LOOP;"));
                    }
                    Block::For {
                        iterator,
                        lower,
                        upper,
                        body: children,
                        ..
                    } => {
                        push_line(
                            body,
                            format!(
                                "{pad}FOR {} IN ({})..({}) LOOP",
                                quote(iterator),
                                expr(lower),
                                expr(upper),
                            ),
                        );
                        emit(children, depth + 1, kind, body, map);
                        push_line(body, format!("{pad}END LOOP;"));
                    }
                }
                map.insert(
                    block.id().to_owned(),
                    Lines {
                        body_start: start + 1,
                        body_end: body.len() + 1,
                        start: 0,
                        end: 0,
                    },
                );
            }
        }
        emit(&self.blocks, 1, self.kind, &mut body, &mut source_map);
        body.push("END;".into());
        let body_text = body.join("\n");
        let mut delimiter = "$body$".to_owned();
        while body_text.contains(&delimiter) {
            delimiter.insert(delimiter.len() - 1, '_');
        }
        let object = format!("{}.{}", quote(&self.schema), quote(&self.name));
        let signature = format!(
            "{}({})",
            object,
            self.parameters
                .iter()
                .map(|p| p.data_type.qualified())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let mut lines = vec![format!(
            "CREATE OR REPLACE {} {object}(",
            if self.kind == Kind::Function {
                "FUNCTION"
            } else {
                "PROCEDURE"
            }
        )];
        for (i, p) in self.parameters.iter().enumerate() {
            lines.push(format!(
                "    {} {}{}",
                quote(&p.name),
                p.data_type.qualified(),
                if i + 1 == self.parameters.len() {
                    ""
                } else {
                    ","
                }
            ));
        }
        lines.push(")".into());
        if self.kind == Kind::Function {
            lines.push(format!("RETURNS {}", self.return_type.qualified()));
        }
        lines.push("LANGUAGE plpgsql".into());
        lines.push(format!("AS {delimiter}"));
        let offset = lines.len();
        lines.extend(body);
        lines.push(format!("{delimiter};"));
        for range in source_map.values_mut() {
            range.start = offset + range.body_start - 1;
            range.end = offset + range.body_end - 1;
        }
        Ok(Generated {
            sql: lines.join("\n"),
            body: format!("\n{body_text}\n"),
            signature,
            source_map,
        })
    }
}
