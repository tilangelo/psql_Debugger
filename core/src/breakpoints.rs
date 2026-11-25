//! Привязка строк редактора к началу оператора PL/pgSQL.
//! Это разбор границ операторов, а не замена компилятора PostgreSQL.
//! При незнакомой структуре перенос отключён: угадывать точку нельзя.

#[derive(Debug)]
pub struct SourceMap {
    targets: Vec<Option<i32>>,
    code: Vec<bool>,
    understood: bool,
}

impl SourceMap {
    pub fn new(source: &str) -> Self {
        let (tokens, code) = lex(source);
        let mut parser = Parser {
            tokens,
            pos: 0,
            targets: vec![None; code.len()],
        };
        let understood = parser.block().is_ok() && parser.pos == parser.tokens.len();
        Self {
            targets: parser.targets,
            code,
            understood,
        }
    }

    pub fn resolve(&self, line: i32) -> Result<i32, &'static str> {
        let index =
            usize::try_from(line.saturating_sub(1)).map_err(|_| "Строка вне исходного кода.")?;
        let has_code = self.code.get(index).ok_or("Строка вне исходного кода.")?;
        if !has_code {
            return Err("На пустой строке, в комментарии или внутри текста строки остановки нет.");
        }
        if !self.understood {
            return Err(
                "Не удалось разобрать границы операторов этого тела. Точки по строкам недоступны; доступны остановка на входе и шаги.",
            );
        }
        self.targets[index].ok_or(
            "На этой строке нет отдельной остановки. Выберите начало исполняемого оператора.",
        )
    }

    pub(crate) fn entry_line(&self) -> Option<i32> {
        self.targets.iter().find_map(|line| *line)
    }
}

#[derive(Debug)]
struct Token {
    text: String,
    line: i32,
}

// Кавычки и комментарии скрывают ключевые слова и точки с запятой внутри них.
fn lex(source: &str) -> (Vec<Token>, Vec<bool>) {
    let bytes = source.as_bytes();
    let mut code = vec![false; source.bytes().filter(|&b| b == b'\n').count() + 1];
    let mut tokens = Vec::new();
    let (mut i, mut line) = (0, 1);
    while i < bytes.len() {
        let start = i;
        let token_line = line;
        let mut literal = false;
        if bytes[i].is_ascii_whitespace() {
            if bytes[i] == b'\n' {
                line += 1;
            }
            i += 1;
            continue;
        }
        if bytes[i..].starts_with(b"--") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i..].starts_with(b"/*") {
            i += 2;
            let mut depth = 1;
            while i < bytes.len() && depth > 0 {
                if bytes[i..].starts_with(b"/*") {
                    depth += 1;
                    i += 2;
                } else if bytes[i..].starts_with(b"*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    if bytes[i] == b'\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
            continue;
        }
        let escaped = matches!(bytes[i], b'e' | b'E') && bytes.get(i + 1) == Some(&b'\'');
        if matches!(bytes[i], b'\'' | b'"') || escaped {
            if escaped {
                i += 1;
            }
            let quote = bytes[i];
            literal = quote == b'\'';
            i += 1;
            while i < bytes.len() {
                if escaped && bytes[i] == b'\\' {
                    i += 1;
                    if bytes.get(i) == Some(&b'\n') {
                        line += 1;
                    }
                    i = (i + 1).min(bytes.len());
                } else if bytes[i] == quote {
                    i += 1;
                    if bytes.get(i) == Some(&quote) {
                        i += 1;
                    } else {
                        break;
                    }
                } else {
                    if bytes[i] == b'\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
        } else if let Some(end) = dollar_end(bytes, i) {
            literal = true;
            let delimiter = &bytes[i..=end];
            i = end + 1;
            while i < bytes.len() && !bytes[i..].starts_with(delimiter) {
                if bytes[i] == b'\n' {
                    line += 1;
                }
                i += 1;
            }
            i = (i + delimiter.len()).min(bytes.len());
        } else if bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] >= 128 {
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric()
                    || matches!(bytes[i], b'_' | b'$')
                    || bytes[i] >= 128)
            {
                i += 1;
            }
        } else {
            i += 1;
        }
        if !literal {
            code[(token_line - 1) as usize] = true;
        }
        let quoted = literal || bytes[start] == b'"';
        tokens.push(Token {
            text: if quoted {
                "<quoted>".into()
            } else {
                source[start..i].to_ascii_uppercase()
            },
            line: token_line,
        });
    }
    (tokens, code)
}

fn dollar_end(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes[start] != b'$' {
        return None;
    }
    let mut i = start + 1;
    if bytes.get(i) == Some(&b'$') {
        return Some(i);
    }
    if !bytes
        .get(i)
        .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
    {
        return None;
    }
    while bytes
        .get(i)
        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
    {
        i += 1;
    }
    (bytes.get(i) == Some(&b'$')).then_some(i)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    targets: Vec<Option<i32>>,
}

impl Parser {
    fn at(&self, value: &str) -> bool {
        self.tokens.get(self.pos).is_some_and(|t| t.text == value)
    }
    fn take(&mut self, value: &str) -> Result<(), ()> {
        if !self.at(value) {
            return Err(());
        }
        self.pos += 1;
        Ok(())
    }
    fn mark(&mut self, start: usize, end: usize, target: i32) {
        if start >= end {
            return;
        }
        for line in self.tokens[start].line..=self.tokens[end - 1].line {
            self.targets[(line - 1) as usize].get_or_insert(target);
        }
    }
    // CASE внутри SQL/выражения не должен завершать заголовок IF или цикла.
    fn through(&mut self, delimiter: &str) -> Result<(), ()> {
        let (mut parens, mut cases) = (0, 0);
        while let Some(token) = self.tokens.get(self.pos) {
            let text = token.text.as_str();
            if parens == 0 && cases == 0 && text == delimiter {
                self.pos += 1;
                return Ok(());
            }
            match text {
                "(" | "[" => parens += 1,
                ")" | "]" => parens -= 1,
                "CASE" => cases += 1,
                "END" if cases > 0 => cases -= 1,
                ";" => return Err(()),
                _ => {}
            }
            self.pos += 1;
        }
        Err(())
    }
    fn block(&mut self) -> Result<(), ()> {
        let start = self.pos;
        if self.at("DECLARE") {
            // Имена в кавычках и строковые значения не превращаются в BEGIN.
            while !self.at("BEGIN") {
                if self.pos >= self.tokens.len() {
                    return Err(());
                }
                self.pos += 1;
            }
        }
        self.take("BEGIN")?;
        let body_start = self.pos;
        self.sequence()?;
        if let Some(target) = self
            .tokens
            .get(body_start)
            .and_then(|t| self.targets[(t.line - 1) as usize])
        {
            self.mark(start, body_start, target);
        }
        if self.at("EXCEPTION") {
            self.pos += 1;
            while self.at("WHEN") {
                self.through("THEN")?;
                self.sequence()?;
            }
        }
        self.take("END")?;
        // После последнего END PostgreSQL допускает отсутствие точки с запятой.
        if self.pos == self.tokens.len() {
            Ok(())
        } else {
            self.take(";")
        }
    }
    fn sequence(&mut self) -> Result<(), ()> {
        while self.pos < self.tokens.len()
            && !["END", "ELSE", "ELSIF", "WHEN", "EXCEPTION"]
                .iter()
                .any(|t| self.at(t))
        {
            self.statement()?;
        }
        Ok(())
    }
    fn statement(&mut self) -> Result<(), ()> {
        let start = self.pos;
        let line = self.tokens[start].line;
        match self.tokens[start].text.as_str() {
            "DECLARE" | "BEGIN" => self.block(),
            "IF" => {
                self.through("THEN")?;
                self.mark(start, self.pos, line);
                self.sequence()?;
                while self.at("ELSIF") {
                    self.through("THEN")?;
                    self.sequence()?;
                }
                if self.at("ELSE") {
                    self.pos += 1;
                    self.sequence()?;
                }
                self.take("END")?;
                self.take("IF")?;
                self.take(";")
            }
            "CASE" => {
                self.pos += 1;
                // Точка ставится на CASE. У WHEN своей точки нет.
                let header = self.pos;
                while !self.at("WHEN") {
                    if self.pos >= self.tokens.len() || self.at(";") {
                        return Err(());
                    }
                    self.pos += 1;
                }
                self.mark(start, self.pos.max(header), line);
                while self.at("WHEN") {
                    self.through("THEN")?;
                    self.sequence()?;
                }
                if self.at("ELSE") {
                    self.pos += 1;
                    self.sequence()?;
                }
                self.take("END")?;
                self.take("CASE")?;
                self.take(";")
            }
            "FOR" | "FOREACH" | "WHILE" | "LOOP" => {
                if self.at("LOOP") {
                    self.pos += 1;
                } else {
                    self.through("LOOP")?;
                }
                self.mark(start, self.pos, line);
                self.sequence()?;
                self.take("END")?;
                self.take("LOOP")?;
                self.take(";")
            }
            // Метки требуют отдельного разбора, их нельзя выдавать за оператор.
            "<" => Err(()),
            // NULL компилятор убирает: отдельного события отладки у него нет.
            "NULL" => {
                self.pos += 1;
                self.take(";")
            }
            _ => {
                self.through(";")?;
                self.mark(start, self.pos, line);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shop_multiline_and_branch() {
        let schema = include_str!("../../examples/shop/schema.sql");
        let source = schema
            .split("RETURNS integer LANGUAGE plpgsql AS $$")
            .nth(1)
            .unwrap()
            .split("$$;")
            .next()
            .unwrap();
        let map = SourceMap::new(source);
        assert_eq!(map.resolve(6), Ok(5));
        assert_eq!(map.resolve(9), Ok(9));
        assert_eq!(map.resolve(11), Ok(10));
        assert_eq!(map.resolve(13), Ok(13));
        assert_eq!(map.resolve(3), Ok(5));
        assert!(map.resolve(1).is_err());
        assert!(map.resolve(12).is_err());
        assert!(map.resolve(14).is_err());
    }

    #[test]
    fn literals_comments_and_sql_case_do_not_split_statements() {
        let map = SourceMap::new(
            "BEGIN\n/* SELECT; /* BEGIN */ END */\nx := CASE WHEN true THEN 1 ELSE 2 END;\nEXECUTE $sql$SELECT ';', 'END'\nFROM elsewhere$sql$;\nRAISE NOTICE E'quote\\\' ; BEGIN';\nEND;",
        );
        assert_eq!(map.resolve(3), Ok(3));
        assert_eq!(map.resolve(4), Ok(4));
        assert_eq!(map.resolve(6), Ok(6));
        assert!(map.resolve(2).is_err());
    }

    #[test]
    fn loops_nested_blocks_and_exception_handlers() {
        let map = SourceMap::new(
            "DECLARE\nx int;\nBEGIN\nFOR x IN 1..2 LOOP\nBEGIN\nIF x = 1 THEN\nRAISE NOTICE 'first';\nELSIF x = 2 THEN\nx := 2;\nELSE\nx := 3;\nEND IF;\nEXCEPTION WHEN OTHERS THEN\nx := 4;\nEND;\nEND LOOP;\nEND;",
        );
        assert_eq!(map.resolve(4), Ok(4));
        assert_eq!(map.resolve(5), Ok(6));
        assert_eq!(map.resolve(7), Ok(7));
        assert_eq!(map.resolve(9), Ok(9));
        assert_eq!(map.resolve(14), Ok(14));
        for line in [8, 10, 12, 13, 15, 16, 17] {
            assert!(map.resolve(line).is_err(), "{line}");
        }
    }

    #[test]
    fn unknown_structure_disables_guessing() {
        let map = SourceMap::new("<<block>>\nBEGIN\nNULL;\nEND block;");
        assert!(map.resolve(3).unwrap_err().contains("Не удалось"));
    }

    #[test]
    fn null_and_literal_only_lines_have_no_stop() {
        let map = SourceMap::new("BEGIN\nNULL;\nRAISE NOTICE 'hello\nworld\nend';\nEND");
        assert!(map.resolve(2).is_err());
        assert_eq!(map.resolve(3), Ok(3));
        assert!(map.resolve(4).is_err());
        assert!(map.resolve(i32::MIN).is_err());
    }
}
