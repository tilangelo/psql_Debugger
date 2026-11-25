//! Проверка текстового ввода. Десятичные значения не переводятся в float.
use postgres::types::{Kind, Type};

pub(crate) fn call_type(oid: u32) -> Option<Type> {
    let ty = Type::from_oid(oid)?;
    if let Kind::Array(element) = ty.kind() {
        return call_type(element.oid()).map(|_| ty);
    }
    [
        Type::BOOL,
        Type::INT2,
        Type::INT4,
        Type::INT8,
        Type::FLOAT4,
        Type::FLOAT8,
        Type::TEXT,
        Type::VARCHAR,
        Type::NUMERIC,
        Type::DATE,
        Type::TIMESTAMP,
        Type::UUID,
        Type::JSON,
        Type::JSONB,
        Type::TIMESTAMPTZ,
        Type::TIME,
        Type::INTERVAL,
        Type::BYTEA,
    ]
    .contains(&ty)
    .then_some(ty)
}

pub fn supported(oid: u32) -> bool {
    call_type(oid).is_some()
}

pub fn input_format(oid: u32) -> &'static str {
    match call_type(oid) {
        Some(Type::JSON | Type::JSONB) => "json",
        Some(Type::TIMESTAMPTZ) => "timestamptz",
        Some(Type::TIME) => "time",
        Some(Type::INTERVAL) => "interval",
        Some(Type::BYTEA) => "hex",
        Some(ty) if matches!(ty.kind(), Kind::Array(_)) => "array",
        _ => "text",
    }
}

fn decimal(value: &str) -> bool {
    let mut rest = value.strip_prefix(['+', '-']).unwrap_or(value);
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    rest = &rest[digits..];
    let mut total = digits;
    if let Some(fraction) = rest.strip_prefix('.') {
        let count = fraction.bytes().take_while(u8::is_ascii_digit).count();
        total += count;
        rest = &fraction[count..];
    }
    if total == 0 {
        return false;
    }
    if let Some(exponent) = rest.strip_prefix(['e', 'E']) {
        let exponent = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        return !exponent.is_empty() && exponent.bytes().all(|c| c.is_ascii_digit());
    }
    rest.is_empty()
}

fn date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    {
        return false;
    }
    let year: u32 = value[..4].parse().unwrap();
    let month: usize = value[5..7].parse().unwrap();
    let day: u32 = value[8..].parse().unwrap();
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    year > 0 && (1..=12).contains(&month) && day > 0 && day <= days[month - 1]
}

fn timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 19
        || !matches!(bytes[10], b' ' | b'T')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return false;
    }
    if !bytes[..10].is_ascii() || !date(&value[..10]) {
        return false;
    }
    if ![11..13, 14..16, 17..19]
        .iter()
        .all(|r| bytes[r.clone()].iter().all(u8::is_ascii_digit))
    {
        return false;
    }
    let hour: u8 = value[11..13].parse().unwrap();
    let minute: u8 = value[14..16].parse().unwrap();
    let second: u8 = value[17..19].parse().unwrap();
    let fraction = &bytes[19..];
    hour < 24
        && minute < 60
        && second < 60
        && (fraction.is_empty()
            || (fraction[0] == b'.'
                && (2..=7).contains(&fraction.len())
                && fraction[1..].iter().all(u8::is_ascii_digit)))
}

pub fn value_error(oid: u32, value: &str) -> Option<&'static str> {
    if value.contains('\0') {
        return Some("Нулевой символ недопустим.");
    }
    let value = value.trim();
    let (valid, message) = match oid {
        16 => (
            matches!(value.to_ascii_lowercase().as_str(), "true" | "false"),
            "Введите true или false.",
        ),
        20 => (
            value.parse::<i64>().is_ok(),
            "Введите целое число от -9223372036854775808 до 9223372036854775807.",
        ),
        21 => (
            value.parse::<i16>().is_ok(),
            "Введите целое число от -32768 до 32767.",
        ),
        23 => (
            value.parse::<i32>().is_ok(),
            "Введите целое число от -2147483648 до 2147483647.",
        ),
        700 => (
            value.parse::<f32>().is_ok_and(f32::is_finite),
            "Введите конечное число real.",
        ),
        701 => (
            value.parse::<f64>().is_ok_and(f64::is_finite),
            "Введите конечное число double precision.",
        ),
        25 | 1043 => (true, ""),
        1700 => (
            decimal(value),
            "Введите конечное десятичное число с точкой, например 123.456 или 1e-8.",
        ),
        1082 => (
            date(value),
            "Введите дату ГГГГ-ММ-ДД, годы от 0001 до 9999.",
        ),
        1114 => (
            timestamp(value),
            "Введите ГГГГ-ММ-ДД ЧЧ:ММ:СС, до 6 цифр после точки, без часового пояса.",
        ),
        2950 => (
            value.len() == 36
                && value.bytes().enumerate().all(|(i, c)| {
                    if [8, 13, 18, 23].contains(&i) {
                        c == b'-'
                    } else {
                        c.is_ascii_hexdigit()
                    }
                }),
            "Введите UUID в формате 123e4567-e89b-12d3-a456-426614174000.",
        ),
        _ => (supported(oid), "Входной тип пока не поддерживается."),
    };
    (!valid).then_some(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precise_values_and_canonical_dates() {
        for (oid, value) in [
            (20, "9223372036854775807"),
            (1700, "9007199254740993.1234567890123456789"),
            (1700, "-.5e+20"),
            (1082, "2000-02-29"),
            (1114, "2026-11-18T01:02:03.123456"),
            (2950, "123e4567-e89b-12d3-a456-426614174000"),
        ] {
            assert!(supported(oid));
            assert_eq!(value_error(oid, value), None, "{value}");
        }
        for (oid, value) in [
            (20, "9223372036854775808"),
            (1700, "NaN"),
            (1700, "1; SELECT 1"),
            (1082, "1900-02-29"),
            (1082, "2026-11-31"),
            (1114, "2026-11-18 01:02:03Z"),
            (1114, "2026-11-18 24:00:00"),
            (1114, "абвгдежзий 00:00:00"),
            (2950, "not-a-uuid"),
        ] {
            assert!(value_error(oid, value).is_some(), "{value}");
        }
    }
}
