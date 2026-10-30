//! The tick row classes and their typed lists in the Python type stub.

use std::fmt::Write as _;

use super::table::{Tick, ticks_by_name};

pub(crate) const BEGIN_MARKER: &str = "# --- BEGIN GENERATED TICK CLASSES (tick table) ---";
pub(crate) const END_MARKER: &str = "# --- END GENERATED TICK CLASSES ---";

/// The methods every `<Tick>List` has, declared once: the stub's lists derive
/// from this base, which does not exist at runtime.
const TICK_LIST_BASE: &str = "
_Row = TypeVar(\"_Row\")


class _TickList(Generic[_Row]):
    \"\"\"Supports `len()`, indexing, slicing and iteration, and converts with
    `.to_list()`, `.to_arrow()`, `.to_pandas()` and `.to_polars()`.
    \"\"\"

    def __new__(cls, ticks: Sequence[_Row] = ...) -> Self: ...
    def __len__(self) -> int: ...
    def __bool__(self) -> bool: ...
    def __repr__(self) -> str: ...
    @overload
    def __getitem__(self, key: int) -> _Row: ...
    @overload
    def __getitem__(self, key: slice) -> Self: ...
    def __iter__(self) -> Iterator[_Row]: ...
    def to_list(self) -> List[_Row]:
        \"\"\"Return a plain Python `list` of the rows.\"\"\"
        ...

    def to_arrow(self) -> Any:
        \"\"\"Return a `pyarrow.Table` of the rows, carrying only the columns the response sent.\"\"\"
        ...

    def to_pandas(self) -> Any:
        \"\"\"Return a `pandas.DataFrame` via `pyarrow.Table.to_pandas()`. Requires pandas and pyarrow:
        `pip install thetadatadx-py[pandas]`.
        \"\"\"
        ...

    def to_polars(self) -> Any:
        \"\"\"Return a `polars.DataFrame` via `polars.from_arrow`. Requires polars and pyarrow:
        `pip install thetadatadx-py[polars]`.
        \"\"\"
        ...
";

/// The Python type of a column, the type its row attribute holds.
fn py_type(ty: &str) -> &'static str {
    match ty {
        "i32" | "i64" | "eod_num" | "eod_num64" | "eod_date" => "int",
        "f64" | "price" | "eod_price" => "float",
        "String" | "right" | "calendar_status" => "str",
        other => panic!("no Python type for tick column type '{other}'"),
    }
}

/// The attribute of a column: `lambda` is a Python keyword.
fn py_field(field: &str) -> String {
    if field == "lambda" {
        "lambda_".to_string()
    } else {
        field.to_string()
    }
}

/// A docstring at `indent`, closing on its own line when it spans several.
fn write_docstring(out: &mut String, indent: &str, lines: &[&str]) {
    match lines {
        [line] => writeln!(out, "{indent}\"\"\"{line}\"\"\"").unwrap(),
        [first, rest @ ..] => {
            writeln!(out, "{indent}\"\"\"{first}").unwrap();
            for line in rest {
                if line.is_empty() {
                    out.push('\n');
                } else {
                    writeln!(out, "{indent}{line}").unwrap();
                }
            }
            writeln!(out, "{indent}\"\"\"").unwrap();
        }
        [] => {}
    }
}

/// The region of `__init__.pyi` holding every tick row class and its
/// `<Tick>List`, by name, with the attributes, accessors and methods the
/// runtime classes expand from the same table.
pub(crate) fn render_python_tick_stubs() -> String {
    let mut out = format!(
        "{BEGIN_MARKER}\n\
         # Generated from the tick table; do not edit by hand. Run\n\
         # `cargo run -p xtask --bin generate_sdk_surfaces` to refresh.\n"
    );
    out.push_str(TICK_LIST_BASE);
    for tick in ticks_by_name() {
        render_row(&mut out, tick);
        let name = tick.name;
        write!(
            out,
            "\n\n@final\nclass {name}List(_TickList[{name}]):\n    \
             \"\"\"Typed list of `{name}` rows, returned by every market-data endpoint whose\n    \
             response is `{name}`.\n    \"\"\"\n"
        )
        .unwrap();
    }
    out.push_str("\n\n");
    out.push_str(END_MARKER);
    out
}

fn render_row(out: &mut String, tick: &Tick) {
    let name = tick.name;
    writeln!(out, "\n\n@final\nclass {name}:").unwrap();
    // The summary and the first paragraph: the later ones describe the decode.
    let doc: Vec<&str> = std::iter::once(tick.summary)
        .chain(
            tick.more
                .iter()
                .copied()
                .take_while(|line| !line.is_empty()),
        )
        .collect();
    write_docstring(out, "    ", &doc);

    // Every column is a keyword argument with the row's zero value as its
    // default, except the calendar status, which has none.
    out.push_str("\n    def __new__(\n        cls,\n        *,\n");
    let mut attributes = Vec::new();
    for column in tick.columns {
        let field = py_field(column.field);
        let base = py_type(column.ty);
        // A nullable column is `None` when the response did not carry it,
        // which the table's Rust documentation says through `columns()`.
        let (ty, doc) = if column.nullable {
            let first = column
                .doc
                .split_inclusive(". ")
                .next()
                .unwrap_or(column.doc);
            (
                format!("Optional[{base}]"),
                format!(
                    "{} `None` when the response carried no such column.",
                    first.trim_end()
                ),
            )
        } else {
            (base.to_string(), column.doc.to_string())
        };
        if column.ty == "calendar_status" {
            writeln!(out, "        {field}: {ty},").unwrap();
        } else {
            writeln!(out, "        {field}: {ty} = ...,").unwrap();
        }
        attributes.push((field, ty, doc));
    }
    if tick.contract_id {
        out.push_str(
            "        expiration: Optional[int] = ...,\n        \
             strike: Optional[float] = ...,\n        \
             right: Optional[str] = ...,\n",
        );
    }
    writeln!(out, "    ) -> {name}: ...\n").unwrap();

    for (field, ty, doc) in &attributes {
        writeln!(out, "    {field}: {ty}").unwrap();
        write_docstring(out, "    ", &[doc.as_str()]);
    }
    if tick.contract_id {
        out.push_str(
            "    expiration: Optional[int]\n    \
             \"\"\"Expiration of the row's contract as a YYYYMMDD integer on a wildcard option response; `None` when the response names a single contract.\"\"\"\n    \
             strike: Optional[float]\n    \
             \"\"\"Strike of the row's contract in dollars on a wildcard option response; `None` when the response names a single contract.\"\"\"\n    \
             right: Optional[str]\n    \
             \"\"\"Right of the row's contract on a wildcard option response, `\"C\"` or `\"P\"`; `None` when the response names a single contract.\"\"\"\n",
        );
    }
    for timestamp in tick.timestamps {
        writeln!(
            out,
            "\n    @property\n    def {}(self) -> Optional[int]:\n        \
             \"\"\"Unix epoch milliseconds (UTC, DST-aware) combining `{date}` with `{ms}` \
             (Eastern-Time milliseconds-of-day). `None` when `{date}` is absent (`0`).\"\"\"\n        ...",
            timestamp.name,
            date = timestamp.date,
            ms = timestamp.ms,
        )
        .unwrap();
    }
    for flag in tick.flags {
        writeln!(
            out,
            "\n    @property\n    def {}(self) -> bool:\n        \"\"\"{}\"\"\"\n        ...",
            flag.name, flag.doc
        )
        .unwrap();
    }
    out.push_str("\n    def __repr__(self) -> str: ...\n");
}
