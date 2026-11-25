//! Редактирование только тела: заголовок определения берётся из PostgreSQL.
use crate::{
    catalog::Connection,
    definition::{self, ChangeError, Snapshot},
};
use postgres::error::ErrorPosition;
use postgres::{GenericClient, NoTls};

pub struct Prepared {
    pub before: Snapshot,
    pub body: String,
    pub after: String,
    sql: String,
}

fn snapshot(client: &mut impl GenericClient, oid: u32) -> Result<(Snapshot, String), ChangeError> {
    let row = client
        .query_opt(
            "SELECT p.xmin::text, pg_catalog.pg_get_functiondef(p.oid), p.prosrc,
                l.lanname = 'plpgsql' AND p.prokind IN ('f','p'),
                r.rolsuper OR r.oid = p.proowner
         FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_language l ON l.oid=p.prolang
         JOIN pg_catalog.pg_roles r ON r.rolname=current_user WHERE p.oid=$1",
            &[&oid],
        )
        .map_err(|_| {
            ChangeError::Invalid("Не удалось прочитать свойства выбранного объекта.".into())
        })?
        .ok_or(ChangeError::Conflict)?;
    if !row.get::<_, bool>(3) {
        return Err(ChangeError::Invalid(
            "Редактор тела поддерживает только функции и процедуры на PL/pgSQL.".into(),
        ));
    }
    if !row.get::<_, bool>(4) {
        return Err(ChangeError::Invalid(
            "Редактировать тело может владелец объекта либо администратор PostgreSQL.".into(),
        ));
    }
    Ok((
        Snapshot {
            oid,
            revision: row.get(0),
            definition: row.get(1),
        },
        row.get(2),
    ))
}

// Это не разбор пользовательского DDL: принимается только проверенный вывод pg_get_functiondef.
fn replace_body(definition: &str, old: &str, body: &str) -> Result<(String, usize), ChangeError> {
    if body.len() > 1_000_000 || body.contains('\0') || body.trim().is_empty() {
        return Err(ChangeError::Invalid(
            "Тело должно быть непустым, без нулевого символа и не больше 1 МБ.".into(),
        ));
    }
    let mut candidate = None;
    for (index, _) in definition.match_indices("\nAS $") {
        let tag_start = index + 4;
        let Some(tag_end) = definition[tag_start + 1..]
            .find('$')
            .map(|n| tag_start + 1 + n)
        else {
            continue;
        };
        let tag = &definition[tag_start..=tag_end];
        if !tag[1..tag.len() - 1]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            continue;
        }
        let content = tag_end + 1;
        let Some(rest) = definition[content..]
            .strip_prefix(old)
            .and_then(|s| s.strip_prefix(tag))
        else {
            continue;
        };
        if !matches!(rest.trim(), "" | ";") {
            continue;
        }
        if candidate.replace((tag_start, rest)).is_some() {
            return Err(ChangeError::Invalid(
                "Формат определения неоднозначен. Изменение не выполнялось.".into(),
            ));
        }
    }
    let (start, rest) = candidate.ok_or_else(|| {
        ChangeError::Invalid(
            "Не удалось отделить тело от определения PostgreSQL. Изменение не выполнялось.".into(),
        )
    })?;
    let mut tag = "$pgdebug_body$".to_string();
    while body.contains(&tag) {
        tag.insert(tag.len() - 1, '_');
    }
    Ok((
        format!("{}{tag}{body}{tag}{rest}", &definition[..start]),
        start + tag.len(),
    ))
}

fn error_position(
    position: Option<&ErrorPosition>,
    sql: &str,
    start: usize,
    body: &str,
) -> Option<(usize, usize)> {
    let body_start = sql[..start].chars().count();
    let relative = match position? {
        ErrorPosition::Original(position) => (*position as usize)
            .checked_sub(1)?
            .checked_sub(body_start)?,
        ErrorPosition::Internal { position, query } if query == body => {
            (*position as usize).checked_sub(1)?
        }
        ErrorPosition::Internal { position, query } if query == sql => (*position as usize)
            .checked_sub(1)?
            .checked_sub(body_start)?,
        _ => return None,
    };
    if relative > body.chars().count() {
        return None;
    }
    Some(
        body.chars()
            .take(relative)
            .fold((1, 1), |(line, column), c| {
                if c == '\n' {
                    (line + 1, 1)
                } else {
                    (line, column + 1)
                }
            }),
    )
}

pub fn prepare(
    connection: &Connection,
    oid: u32,
    expected_definition: &str,
    body: &str,
) -> Result<Prepared, ChangeError> {
    let db = |err| ChangeError::Database(connection.diagnostic(err));
    let mut client = connection.config().connect(NoTls).map_err(db)?;
    let mut tx = client.transaction().map_err(db)?;
    definition::setup(&mut tx, &format!("body:{oid}")).map_err(db)?;
    let (before, source) = snapshot(&mut tx, oid)?;
    if before.definition != expected_definition {
        return Err(ChangeError::Conflict);
    }
    let (sql, start) = replace_body(&before.definition, &source, body)?;
    tx.batch_execute(&sql).map_err(|error| {
        let position = error_position(
            error.as_db_error().and_then(|e| e.position()),
            &sql,
            start,
            body,
        );
        let diagnostic = connection.diagnostic(error);
        if let Some((line, column)) = position {
            ChangeError::DatabaseAt {
                diagnostic,
                line,
                column,
            }
        } else {
            ChangeError::Database(diagnostic)
        }
    })?;
    let (after, after_source) = snapshot(&mut tx, oid)?;
    if after_source != body {
        return Err(ChangeError::Invalid(
            "Проверенное тело отличается от черновика.".into(),
        ));
    }
    tx.rollback().map_err(db)?;
    Ok(Prepared {
        before,
        body: body.into(),
        after: after.definition,
        sql,
    })
}

pub fn apply(connection: &Connection, expected: &Prepared) -> Result<(), ChangeError> {
    let db = |err| ChangeError::Database(connection.diagnostic(err));
    let mut client = connection.config().connect(NoTls).map_err(db)?;
    let mut tx = client.transaction().map_err(db)?;
    definition::setup(&mut tx, &format!("body:{}", expected.before.oid)).map_err(db)?;
    if snapshot(&mut tx, expected.before.oid)?.0 != expected.before {
        return Err(ChangeError::Conflict);
    }
    tx.batch_execute(&expected.sql).map_err(db)?;
    tx.commit().map_err(db)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn body_is_quoted_and_header_is_preserved() {
        let old = "\nBEGIN RETURN 1; END;\n";
        let definition = format!(
            "CREATE OR REPLACE FUNCTION x()\n RETURNS integer\n LANGUAGE plpgsql\n SECURITY DEFINER\nAS $function${old}$function$\n"
        );
        let body = "BEGIN RETURN '$pgdebug_body$; DROP TABLE x;'; END;";
        let (sql, _) = replace_body(&definition, old, body).unwrap();
        assert!(sql.starts_with("CREATE OR REPLACE FUNCTION x()\n RETURNS integer\n LANGUAGE plpgsql\n SECURITY DEFINER\nAS $pgdebug_body_$"));
        assert!(sql.ends_with("$pgdebug_body_$\n"));
        assert!(replace_body(&definition, "different body", body).is_err());
        assert!(replace_body(&definition, old, "\0").is_err());
    }
    #[test]
    fn error_location_uses_characters_and_only_matching_query() {
        let body = "BEGIN\n -- 😀 Пример\n RETURN ;\nEND;";
        let prefix = "CREATE OR REPLACE FUNCTION \"Функция\"() AS $x$";
        let sql = format!("{prefix}{body}$x$");
        let position = prefix.chars().count() + body[..body.find(';').unwrap()].chars().count() + 1;
        assert_eq!(
            error_position(
                Some(&ErrorPosition::Original(position as u32)),
                &sql,
                prefix.len(),
                body
            ),
            Some((3, 9))
        );
        assert_eq!(
            error_position(
                Some(&ErrorPosition::Internal {
                    position: 1,
                    query: "unrelated SQL".into()
                }),
                &sql,
                prefix.len(),
                body
            ),
            None
        );
        assert_eq!(
            error_position(Some(&ErrorPosition::Original(1)), &sql, prefix.len(), body),
            None
        );
    }
}
