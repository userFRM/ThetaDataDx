# Endpoint Schema (`endpoint_schema.toml`)

The file `crates/thetadatadx/endpoint_schema.toml` is the **single source of truth** for all tick type struct definitions and their DataTable parsers.

## What it is

A TOML file where each `[types.TypeName]` table describes:
- The Rust struct (fields, types, derives, repr/alignment)
- The parser function that converts a protobuf `DataTable` into a `Vec<TypeName>`

`build.rs` reads this file at compile time and generates two Rust source files into `$OUT_DIR`:
- `tick_generated.rs` -- all tick struct definitions
- `decode_generated.rs` -- all `parse_*` functions

These are included into the crate via `include!()`:
- `src/types/tick.rs` includes `tick_generated.rs` and adds hand-written `impl` blocks
- `src/decode.rs` includes `decode_generated.rs` alongside the hand-written helper functions

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
| `price_typed_columns`  | `[string]` | Columns that may carry `Price`-typed cells (vs plain `Number`) |
| `eod_style`            | `bool`     | Use `eod_num` helper that handles both Price and Number cells |

### Per-column options

| Key            | Type      | Description |
|:---------------|:----------|:------------|
| `name`         | `string`  | The DataTable header name to look up |
| `field`        | `string`  | The Rust struct field name |
| `type`         | `string`  | One of the column types above |
| `price_source` | `string?` | For `price_type` fields: which price column to extract the type from |

## How to add a new endpoint/column

1. Add a new `[types.YourNewTick]` table to `endpoint_schema.toml`
2. Define all columns with their header names, field names, and types
3. Set `parser = "parse_your_new_ticks"`
4. Set `required`, `copy`, `align`, etc. as needed
5. Run `cargo build` -- the struct and parser are generated automatically
6. If the tick needs helper methods (like `get_price()`), add an `impl YourNewTick` block in `src/types/tick.rs` after the `include!()` line
7. Wire up the new parser in `src/direct.rs` where needed

To add a column to an existing type, just add a new entry to that type's `columns` array.

## What build.rs generates

For each type in the schema:

**Struct** (`tick_generated.rs`):
```rust
/// Doc comment from schema
#[derive(Debug, Clone, Copy)]  // Copy if copy = true
#[repr(C, align(64))]          // if align is set
pub struct GreeksTick {
    pub ms_of_day: i32,
    pub implied_volatility: f64,
    // ...
}
```

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

1. Add the proto definitions to `proto/v3_endpoints.proto`
2. Add the tick type to `endpoint_schema.toml`
3. Wire up the RPC in `src/direct.rs`
4. Add any needed `impl` blocks in `src/types/tick.rs`
