//! Parameterized query and keyset pagination shared by generated resolvers.
use async_graphql::InputObject;
use serde::{Deserialize, Serialize};
use sqlx::{postgres::PgRow, PgPool, Postgres, QueryBuilder};

/// A schema-owned column. Names and casts come from code generation, not requests.
#[derive(Clone, Copy)]
pub struct QueryColumn {
    pub name: &'static str,
    pub column: &'static str,
    pub cast: &'static str,
    pub filterable: bool,
}

/// An indexed-field predicate. Values are bound parameters, including numeric values.
#[derive(InputObject, Clone, Default)]
pub struct EntityFilter {
    pub field: String,
    pub eq: Option<String>,
    pub gt: Option<String>,
    pub lt: Option<String>,
    pub is_null: Option<bool>,
}

#[derive(Serialize, Deserialize)]
struct Cursor {
    field: String,
    descending: bool,
    value: Option<String>,
    id: String,
}

fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
fn column<'a>(columns: &'a [QueryColumn], name: &str) -> async_graphql::Result<&'a QueryColumn> {
    columns
        .iter()
        .find(|c| c.name == name || c.column == name)
        .ok_or_else(|| "unknown query column".into())
}
fn bind_cast(builder: &mut QueryBuilder<Postgres>, value: String, cast: &str) {
    if let Some(element) = cast.strip_suffix("[]") {
        builder
            .push("ARRAY(SELECT v::")
            .push(element.to_string())
            .push(" FROM jsonb_array_elements_text(")
            .push_bind(value)
            .push("::jsonb) AS v)");
    } else {
        builder
            .push_bind(value)
            .push("::text::")
            .push(cast.to_string());
    }
}

/// Produce an opaque JSON cursor from a generated read model.
pub fn encode_cursor(
    row: &serde_json::Value,
    columns: &[QueryColumn],
    order_by: Option<&str>,
    descending: bool,
) -> async_graphql::Result<String> {
    let col = column(columns, order_by.unwrap_or("id"))?;
    let value = &row[col.column];
    let value = if value.is_null() {
        None
    } else if let Some(s) = value.as_str() {
        Some(s.to_string())
    } else {
        Some(value.to_string())
    };
    let id = row["id"]
        .as_str()
        .ok_or("entity ID is not a string")?
        .to_string();
    Ok(serde_json::to_string(&Cursor {
        field: col.column.into(),
        descending,
        value,
        id,
    })?)
}

/// Fetch one bounded page. Ordering is whitelisted; cursors use the order value
/// plus ID as a deterministic tiebreaker. NULLs sort last in both directions.
#[allow(clippy::too_many_arguments)]
pub async fn query_entities(
    pool: &PgPool,
    table: &'static str,
    select: &'static str,
    columns: &[QueryColumn],
    filters: Vec<EntityFilter>,
    order_by: Option<String>,
    descending: bool,
    after: Option<String>,
    limit: i32,
) -> async_graphql::Result<Vec<PgRow>> {
    if filters.len() > 32 {
        return Err("too many query filters (maximum 32)".into());
    }
    let col = column(columns, order_by.as_deref().unwrap_or("id"))?;
    let id_column = column(columns, "id")?;
    let order = ident(col.column);
    let id = ident(id_column.column);
    let mut query = QueryBuilder::new(format!("SELECT {select} FROM {} WHERE TRUE", ident(table)));
    for filter in filters {
        let c = column(columns, &filter.field)?;
        if !c.filterable {
            return Err("filter field must be indexed, unique, an ID or relation".into());
        }
        let operators = usize::from(filter.eq.is_some())
            + usize::from(filter.gt.is_some())
            + usize::from(filter.lt.is_some())
            + usize::from(filter.is_null.is_some());
        if operators != 1 {
            return Err("each filter needs exactly one operator".into());
        }
        query.push(format!(" AND {} ", ident(c.column)));
        if let Some(null) = filter.is_null {
            query.push(if null { "IS NULL" } else { "IS NOT NULL" });
        } else {
            let (op, value) = if let Some(v) = filter.eq {
                ("=", v)
            } else if let Some(v) = filter.gt {
                (">", v)
            } else {
                ("<", filter.lt.unwrap())
            };
            query.push(op).push(" ");
            bind_cast(&mut query, value, c.cast);
        }
    }
    if let Some(after) = after {
        let cursor: Cursor = serde_json::from_str(&after)
            .map_err(|_| async_graphql::Error::new("invalid page cursor"))?;
        if cursor.field != col.column || cursor.descending != descending {
            return Err("cursor ordering does not match query".into());
        }
        let op = if descending { " < " } else { " > " };
        if let Some(value) = cursor.value {
            query.push(format!(" AND ({order} IS NULL OR {order}{op}"));
            bind_cast(&mut query, value.clone(), col.cast);
            query.push(format!(" OR ({order} = "));
            bind_cast(&mut query, value, col.cast);
            query.push(format!(" AND {id}{op}"));
            bind_cast(&mut query, cursor.id, id_column.cast);
            query.push("))");
        } else {
            query.push(format!(" AND ({order} IS NULL AND {id}{op}"));
            bind_cast(&mut query, cursor.id, id_column.cast);
            query.push(")");
        }
    }
    let direction = if descending {
        " DESC NULLS LAST"
    } else {
        " ASC NULLS LAST"
    };
    query
        .push(format!(
            " ORDER BY {order}{direction}, {id}{direction} LIMIT "
        ))
        .push_bind(limit.clamp(1, 200) as i64);
    Ok(query.build().fetch_all(pool).await?)
}
