# Endpoint Schema (`endpoint_schema.toml`)

The file `crates/thetadatadx/endpoint_schema.toml` is the canonical schema for
ThetaData `DataTable` response layouts and the generated parser functions that
decode them into typed ticks.

## What it is

A TOML file where each `[types.TypeName]` table describes:
- The expected `DataTable` columns for a tick layout
- The parser function that converts a protobuf `DataTable` into a `Vec<TypeName>`

`build.rs` reads this file at compile time and generates one Rust source file into `$OUT_DIR`:
- `decode_generated.rs` - all `parse_*` functions

The parser module is included into the crate via `include!()`:
- `src/decode.rs` includes `decode_generated.rs` alongside the hand-written helper functions

The public tick structs themselves live in `crates/tdbe/src/types/tick.rs` and
must stay aligned with the schema.

## Column types

| Type         | Rust field type | Reader function                    | Default |
|:-------------|:----------------|:-----------------------------------|:--------|
| `i32`        | `i32`           | `row_number(row, i)`               | `0`     |
| `i64`        | `i64`           | `row_number_i64(row, i)`           | `0`     |
| `f64`        | `f64`           | `row_float(row, i)`                | `0.0`   |
| `String`     | `String`        | `row_text(row, i)`                 | `""`    |
| `price`      | `i32`           | `row_price_value(row, i)` or `row_number(row, i)` depending on whether the column carries Price-typed cells | `0` |
| `price_value`| `i32`           | Always `row_price_value(row, i)`   | `0`     |
| `eod_num`    | `i32`           | Inline helper that accepts both `Number` and `Price` cell types | `0` |

## Schema options

### Per-type options

| Key                    | Type       | Description |
|:-----------------------|:-----------|:------------|
| `doc`                  | `string`   | Doc comment on the generated struct |
| `copy`                 | `bool`     | Derive `Copy` (false for types with `String` fields) |
| `align`                | `int?`     | If set, adds `#[repr(C, align(N))]` |
| `parser`               | `string`   | Name of the generated parse function |
| `required`             | `[string]` | Headers that must exist or the parser returns `vec![]` |
| `eod_style`            | `bool`     | Use `eod_num` helper that handles both Price and Number cells |
| `contract_id`          | `bool`     | Inject `expiration`/`strike`/`right` fields (populated on wildcard queries) |

### Per-column options

| Key            | Type      | Description |
|:---------------|:----------|:------------|
| `name`         | `string`  | The DataTable header name to look up |
| `field`        | `string`  | The Rust struct field name |
| `type`         | `string`  | One of the column types above |

## How to add a new endpoint/column

1. Add a new `[types.YourNewTick]` table to `endpoint_schema.toml`
2. Define all columns with their header names, field names, and types
3. Set `parser = "parse_your_new_ticks"`
4. Set `required`, `copy`, `align`, etc. as needed
5. Run `cargo build` - the parser is generated automatically
6. Add or update the corresponding tick struct in `crates/tdbe/src/types/tick.rs`
7. If the tick needs helper methods, add them in `tdbe`
8. Wire the new layout into `endpoint_surface.toml` / `external.proto` as needed

To add a column to an existing type, just add a new entry to that type's `columns` array.

## What build.rs generates

For each type in the schema:

**Parser** (`decode_generated.rs`):
```rust
pub fn parse_greeks_ticks(table: &crate::proto::DataTable) -> Vec<GreeksTick> {
    // header lookup
    // required-header guards
    // row iteration with correct reader functions
}
```

## When ThetaData updates their proto

If ThetaData adds new fields to an existing endpoint's DataTable:

1. Add the new column(s) to the corresponding type in `endpoint_schema.toml`
2. Run `cargo build` and `cargo test`
3. The new field automatically appears in the struct and is parsed from the DataTable

If ThetaData adds a completely new endpoint:

1. Update `proto/external.proto`
2. Add the endpoint entry to `endpoint_surface.toml`
3. Add the tick type to `endpoint_schema.toml`
4. Add or update the corresponding tick struct in `crates/tdbe/src/types/tick.rs`
