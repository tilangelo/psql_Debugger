//! Возвращаем стандартное бинарное значение серверу параметром и просим текстовый вид.
//! Числа не проходят через float, а форматы numeric/date/uuid не дублируются в приложении.
use bytes::BytesMut;
use postgres::{
    Row, Transaction,
    types::{FromSql, IsNull, Kind, ToSql, Type},
};
use std::error::Error;

#[derive(Debug)]
struct RawValue<'a> {
    data: &'a [u8],
    ty: Type,
}
pub(crate) fn supported(ty: &Type) -> bool {
    match ty.kind() {
        Kind::Enum(_) | Kind::Composite(_) => true,
        Kind::Domain(base) | Kind::Array(base) => supported(base),
        _ => crate::scalar::supported(ty.oid()),
    }
}
impl<'a> FromSql<'a> for RawValue<'a> {
    fn from_sql(ty: &Type, data: &'a [u8]) -> Result<Self, Box<dyn Error + Sync + Send>> {
        Ok(Self {
            data,
            ty: ty.clone(),
        })
    }
    fn accepts(ty: &Type) -> bool {
        supported(ty)
    }
}
impl ToSql for RawValue<'_> {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn Error + Sync + Send>> {
        if ty != &self.ty {
            return Err("Тип выходного значения изменился.".into());
        }
        out.extend_from_slice(self.data);
        Ok(IsNull::No)
    }
    fn accepts(ty: &Type) -> bool {
        supported(ty)
    }
    postgres::types::to_sql_checked!();
}
pub(crate) fn render(
    transaction: &mut Transaction<'_>,
    row: &Row,
    index: usize,
) -> Result<String, postgres::Error> {
    let ty = row.columns()[index].type_();
    let value: Option<RawValue<'_>> = row.try_get(index)?;
    // OID параметра берём из результата CALL, поэтому имя типа не нужно вставлять в SQL.
    let rows = transaction.query_typed(
        "SELECT pg_catalog.left($1::pg_catalog.text, 16385)",
        &[(&value, ty.clone())],
    )?;
    let rendered: Option<String> = rows[0].try_get(0)?;
    Ok(rendered.unwrap_or_else(|| "NULL".into()))
}
