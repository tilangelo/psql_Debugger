//! Результат вызова и откат хранятся отдельно от текста для интерфейса.
use serde::Serialize;
use std::error::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub sqlstate: Option<String>,
    pub message: String,
    pub context: Option<String>,
    pub detail: Option<String>,
    pub hint: Option<String>,
}

impl Diagnostic {
    pub(crate) fn from_postgres(error: postgres::Error, password: &str) -> Self {
        let redact = |text: &str| {
            if password.is_empty() {
                text.to_owned()
            } else {
                text.replace(password, "[скрыто]")
            }
        };
        if let Some(db) = error.as_db_error() {
            Self {
                sqlstate: Some(db.code().code().into()),
                message: redact(db.message()),
                context: db.where_().map(redact),
                detail: db.detail().map(redact),
                hint: db.hint().map(redact),
            }
        } else {
            let mut message = error.to_string();
            let mut cause = error.source();
            while let Some(error) = cause {
                message.push_str(&format!("\n{error}"));
                cause = error.source();
            }
            Self {
                sqlstate: None,
                message: redact(&message),
                context: None,
                detail: None,
                hint: None,
            }
        }
    }

    pub fn render(&self) -> String {
        let mut text = self
            .sqlstate
            .as_ref()
            .map(|code| format!("SQLSTATE {code}\n"))
            .unwrap_or_default();
        text.push_str(&self.message);
        for (label, value) in [
            ("Контекст", &self.context),
            ("Подробности", &self.detail),
            ("Подсказка", &self.hint),
        ] {
            if let Some(value) = value {
                text.push_str(&format!("\n{label}: {value}"));
            }
        }
        text
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ResultPreview {
    pub text: String,
    pub scalar: Option<String>,
    pub total_rows: u64,
    pub shown_rows: usize,
    pub truncated: bool,
    pub table: Option<ResultTable>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ResultTable {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<ResultCell>>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ResultCell {
    pub value: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CallOutcome {
    NotStarted(String),
    SetupFailed(Diagnostic),
    DefinitionChanged,
    Succeeded(ResultPreview),
    Failed(Diagnostic),
    Unknown(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum RollbackOutcome {
    NotNeeded,
    Confirmed,
    Unconfirmed(String),
}

#[derive(Debug)]
pub struct SessionOutcome {
    pub call: CallOutcome,
    pub rollback: RollbackOutcome,
    pub cancellation_requested: bool,
    pub debugger_error: Option<Diagnostic>,
    pub notes: Vec<String>,
    pub notices: Vec<String>,
}

impl SessionOutcome {
    pub(crate) fn new(call: CallOutcome, rollback: RollbackOutcome) -> Self {
        Self {
            call,
            rollback,
            cancellation_requested: false,
            debugger_error: None,
            notes: Vec::new(),
            notices: Vec::new(),
        }
    }
    pub(crate) fn not_started(reason: impl Into<String>) -> Self {
        Self::new(
            CallOutcome::NotStarted(reason.into()),
            RollbackOutcome::NotNeeded,
        )
    }
    pub(crate) fn setup_failed(error: Diagnostic) -> Self {
        Self::new(CallOutcome::SetupFailed(error), RollbackOutcome::NotNeeded)
    }
    pub fn unknown(reason: impl Into<String>) -> Self {
        Self::new(
            CallOutcome::Unknown(reason.into()),
            RollbackOutcome::Unconfirmed("Связь с потоком вызова потеряна.".into()),
        )
    }

    pub fn render(&self) -> String {
        let mut parts = Vec::new();
        if self.cancellation_requested {
            parts.push("Запрошена остановка.".into());
        }
        if let Some(error) = &self.debugger_error {
            parts.push(format!("Ошибка сеанса отладки.\n{}", error.render()));
        }
        parts.extend(self.notes.iter().cloned());
        parts.push(match &self.call {
            CallOutcome::NotStarted(reason) | CallOutcome::Unknown(reason) => reason.clone(),
            CallOutcome::SetupFailed(error) => error.render(),
            CallOutcome::DefinitionChanged => "Определение объекта изменилось. Каталог будет обновлён, точки сняты. Проверьте аргументы и начните отладку заново. Вызов не выполнялся.".into(),
            CallOutcome::Succeeded(preview) => preview.text.clone(),
            CallOutcome::Failed(error) if error.sqlstate.as_deref() == Some("2D000") => format!("Процедура попыталась выполнить COMMIT или ROLLBACK внутри транзакции отладки.\n{}", error.render()),
            CallOutcome::Failed(error) => format!("Вызов завершился ошибкой.\n{}", error.render()),
        });
        match &self.rollback {
            RollbackOutcome::NotNeeded => {}
            RollbackOutcome::Confirmed => parts.push("Транзакция откачена.".into()),
            RollbackOutcome::Unconfirmed(detail) => {
                parts.push(format!("Не удалось подтвердить откат.\n{detail}"))
            }
        }
        if !self.notices.is_empty() {
            parts.push(format!(
                "\nСообщения PostgreSQL:\n{}",
                self.notices.join("\n")
            ));
        }
        parts.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sql_failure_and_rollback_are_independent() {
        let error = Diagnostic {
            sqlstate: Some("22012".into()),
            message: "division by zero".into(),
            context: Some("function line 3".into()),
            detail: None,
            hint: None,
        };
        let outcome = SessionOutcome::new(CallOutcome::Failed(error), RollbackOutcome::Confirmed);
        assert!(matches!(outcome.call, CallOutcome::Failed(_)));
        assert_eq!(outcome.rollback, RollbackOutcome::Confirmed);
        assert!(outcome.render().contains("Контекст: function line 3"));
    }
    #[test]
    fn cancellation_does_not_claim_rollback() {
        let mut outcome = SessionOutcome::unknown("Нет подтверждения завершения");
        outcome.cancellation_requested = true;
        assert!(matches!(outcome.rollback, RollbackOutcome::Unconfirmed(_)));
        assert!(!outcome.render().contains("Транзакция откачена."));
    }
}
