//! The endpoint table, expanded into plain records for the writer.
//!
//! `thetadatadx::__endpoints!`, `__stream_endpoints!` and `__request_options!`
//! call back into the macros below, which turn each entry into a
//! [`GeneratedEndpoint`] and each resolved parameter into a
//! [`GeneratedParam`]. The renderers read nothing else about an endpoint.

use std::collections::BTreeSet;

use heck::ToUpperCamelCase;

use super::enum_projection::{ENUMS, EnumVariantProjection};

/// One endpoint of the table.
pub(super) struct GeneratedEndpoint {
    pub(super) name: &'static str,
    /// The first documentation line, which is also the registry description.
    pub(super) description: &'static str,
    /// Every documentation line, the description first.
    pub(super) doc: &'static [&'static str],
    pub(super) category: &'static str,
    pub(super) subcategory: &'static str,
    pub(super) rest_path: &'static str,
    /// `list`, `buffered`, `chunked` or `stream`.
    pub(super) shape: &'static str,
    /// The returned collection: `StringList`, or the tick type plus `s`.
    pub(super) return_type: &'static str,
    pub(super) list_column: Option<&'static str>,
    pub(super) params: &'static [GeneratedParam],
}

/// One parameter of an endpoint, in declaration order.
#[derive(Clone, Copy)]
pub(super) struct GeneratedParam {
    /// The wire and registry name.
    pub(super) name: &'static str,
    pub(super) description: &'static str,
    pub(super) param_type: &'static str,
    /// A positional method argument when set, a builder setter otherwise.
    pub(super) required: bool,
    /// The documented default, without quotes.
    pub(super) default: Option<&'static str>,
}

macro_rules! param_records {
    ($({ $p:ident $a:ident $b:ident $t:ident [$($v:tt)?] $d:literal })*) => {
        &[$(GeneratedParam {
            name: stringify!($p),
            description: $d,
            param_type: stringify!($t),
            required: param_records!(@required $b),
            default: param_records!(@default $t $($v)?),
        }),*]
    };
    (@required req) => { true };
    (@required opt) => { false };
    (@default $t:ident) => { None };
    (@default Bool $v:tt) => { Some(stringify!($v)) };
    (@default Int $v:tt) => { Some(stringify!($v)) };
    (@default Float $v:tt) => { Some(stringify!($v)) };
    (@default $t:ident $v:literal) => { Some($v) };
}

macro_rules! endpoint_records {
    ($out:ident $( $(#[$attr:meta])* $shape:ident $name:ident {
        group: $cat:ident / $sub:ident,
        rest: $rest:literal,
        returns: $ret:ident $(($col:literal))?,
        params: [$($key:tt)*],
        doc: [$($doc:literal),* $(,)?],
    } )*) => {
        $( thetadatadx::__resolve! { [$($key)*] => endpoint_record {
            $out $shape $name $cat $sub $rest $ret $(($col))? [$($doc)*]
        } } )*
    };
}

macro_rules! endpoint_record {
    ($out:ident $shape:ident $name:ident $cat:ident $sub:ident $rest:literal $ret:ident $(($col:literal))?
        [$description:literal $($doc:literal)*] params { all [$($all:tt)*] $($resolved:tt)* }) => {
        $out.push(GeneratedEndpoint {
            name: stringify!($name),
            description: $description,
            doc: &[$description $(, $doc)*],
            category: stringify!($cat),
            subcategory: stringify!($sub),
            rest_path: $rest,
            shape: stringify!($shape),
            return_type: endpoint_record!(@returns $ret $(($col))?),
            list_column: endpoint_record!(@column $($col)?),
            params: param_records!($($all)*),
        });
    };
    (@returns StringList($col:literal)) => { "StringList" };
    (@returns $ret:ident) => { concat!(stringify!($ret), "s") };
    (@column $col:literal) => { Some($col) };
    (@column) => { None };
}

macro_rules! request_option_records {
    (params { all [$($all:tt)*] $($resolved:tt)* }) => { param_records!($($all)*) };
}

/// The fields of the C ABI options struct, in ABI order.
pub(super) const REQUEST_OPTIONS: &[GeneratedParam] =
    thetadatadx::__request_options! { request_option_records {} };

/// Every endpoint, in registry order, the raw server streams last.
pub(super) fn endpoints() -> Vec<GeneratedEndpoint> {
    let mut out = Vec::new();
    thetadatadx::__endpoints! { endpoint_records { out } }
    thetadatadx::__stream_endpoints! { endpoint_records { out } }
    out
}

/// Every RPC `proto/mdds.proto` declares must have its endpoint in the table.
///
/// The compiler holds the other direction: an entry whose RPC is missing does
/// not build. Comparing the two sets also fails when the proto stops parsing.
pub(super) fn check_proto_coverage(
    endpoints: &[GeneratedEndpoint],
) -> Result<(), Box<dyn std::error::Error>> {
    let proto = std::fs::read_to_string("proto/mdds.proto")?;
    let rpcs: BTreeSet<&str> = proto
        .lines()
        .filter_map(|line| line.trim().strip_prefix("rpc Get"))
        .filter_map(|rest| rest.split([' ', '(']).next())
        .collect();
    let names: BTreeSet<String> = endpoints
        .iter()
        .filter(|e| e.shape != "stream")
        .map(|e| e.name.to_upper_camel_case())
        .collect();
    let missing: Vec<&str> = rpcs
        .iter()
        .copied()
        .filter(|rpc| !names.contains(*rpc))
        .collect();
    if !missing.is_empty() || rpcs.len() != names.len() {
        return Err(format!(
            "proto/mdds.proto declares {} RPCs and the endpoint table {} endpoints; \
             RPCs with no endpoint in src/surface/endpoints.rs: [{}]",
            rpcs.len(),
            names.len(),
            missing.join(", ")
        )
        .into());
    }
    Ok(())
}

/// What the compiler cannot hold the table to. The engine builds an entry of
/// an unknown shape, or a list entry that returns ticks, as a tick endpoint,
/// while the surfaces rendered here would describe it as written. A string
/// default is sent as written, so it must be a value its type accepts. An
/// enum parameter's description lists the enum's values by hand, for the
/// bindings that expand the table without this generator, so it must list
/// exactly those values. The enums themselves must each be declared once,
/// with at least one variant and distinct wire values, Rust names and Python
/// names.
pub(super) fn check_entries(
    endpoints: &[GeneratedEndpoint],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut errors = BTreeSet::new();
    let mut enum_names = BTreeSet::new();
    for spec in ENUMS {
        let distinct = |name: fn(&EnumVariantProjection) -> &'static str| {
            spec.variants
                .iter()
                .map(name)
                .collect::<BTreeSet<_>>()
                .len()
                == spec.variants.len()
        };
        if !enum_names.insert(spec.name)
            || spec.variants.is_empty()
            || !distinct(|v| v.wire)
            || !distinct(|v| v.rust)
            || !distinct(|v| v.python)
        {
            errors.insert(format!(
                "enum {} must be declared once, with distinct wire values, Rust names and Python names",
                spec.name
            ));
        }
    }
    for endpoint in endpoints {
        let (name, shape) = (endpoint.name, endpoint.shape);
        if !matches!(shape, "list" | "buffered" | "chunked" | "stream") {
            errors.insert(format!("{name} has no shape '{shape}'"));
        } else if (shape == "list") != endpoint.list_column.is_some() {
            errors.insert(format!(
                "{name} is {shape} and returns {}: a list, and only a list, returns StringList",
                endpoint.return_type
            ));
        }
    }
    for param in endpoints.iter().flat_map(|e| e.params) {
        let name = param.name;
        let digits = |n: usize| {
            param
                .default
                .is_none_or(|d| d.len() == n && d.bytes().all(|b| b.is_ascii_digit()))
        };
        if (param.param_type == "Date" && !digits(8)) || (param.param_type == "Year" && !digits(4))
        {
            errors.insert(format!(
                "'{name}' has a default its type {} does not accept",
                param.param_type
            ));
        }
        let Some(spec) = ENUMS.iter().find(|e| e.name == param.param_type) else {
            continue;
        };
        if let Some(default) = param.default
            && !spec.variants.iter().any(|v| v.wire == default)
        {
            errors.insert(format!(
                "'{name}' defaults to '{default}', not a value of {}",
                spec.name
            ));
        }
        let mut values: Vec<String> = spec
            .variants
            .iter()
            .map(|v| format!("`{}`", v.wire))
            .collect();
        // `Right` also parses `*`, as `both`.
        if spec.name == "Right" {
            values.push("`*`".into());
        }
        let listed = format!("Accepted values: {}.", values.join(", "));
        if !param.description.ends_with(&listed) {
            errors.insert(format!(
                "'{name}' description does not end with \"{listed}\""
            ));
        }
    }
    if errors.is_empty() {
        return Ok(());
    }
    let errors: Vec<String> = errors.into_iter().collect();
    Err(format!("endpoint table:\n  {}", errors.join("\n  ")).into())
}
