//! The epoch accessors a tick type carries, for the TypeScript renderer.

use super::schema::TickTypeDef;

/// `(accessor_name, ms_field, date_field)` triples: the epoch accessors of a
/// tick type, the same set the core structs carry. Empty when the type has
/// no `date` column.
///
/// A prefixed time of day takes the date of its own prefix when the type
/// carries one. The Greeks ticks hold two moments: the row's own
/// (`date` + `ms_of_day`) and the underlying price's
/// (`underlying_date` + `underlying_ms_of_day`), and the vendor sends them on
/// different days whenever the option quote and the underlying price come
/// from different sessions. Pairing a prefixed time of day with the row's
/// `date` would hand back an instant a day out on exactly those rows.
pub(super) fn timestamp_accessor_fields(def: &TickTypeDef) -> Vec<(String, String, String)> {
    if !def.columns.iter().any(|c| c.field == "date") {
        return Vec::new();
    }
    let has_column = |name: &str| def.columns.iter().any(|c| c.field == name);
    let mut out = Vec::new();
    for column in &def.columns {
        if column.field == "ms_of_day" {
            out.push((
                "timestamp_ms".to_string(),
                column.field.clone(),
                "date".to_string(),
            ));
        } else if let Some(prefix) = column.field.strip_suffix("_ms_of_day") {
            let prefixed_date = format!("{prefix}_date");
            let date_field = if has_column(&prefixed_date) {
                prefixed_date
            } else {
                "date".to_string()
            };
            out.push((
                format!("{prefix}_timestamp_ms"),
                column.field.clone(),
                date_field,
            ));
        }
    }
    out
}
