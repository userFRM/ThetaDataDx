//! The coded fields.
//!
//! Python, TypeScript, Arrow and the local server write some values as names
//! that the C ABI carries as `int32_t` codes: a contract's security type, a
//! subscription result, a disconnect's reason and cause, a Greeks row's
//! implied-volatility source, a calendar day's type, the reconnect policy and
//! the enum settings of the config table. [`coded_fields`] lists each such
//! field once, every code with the name the bindings give it, read from the
//! core type the field carries. `generate_sdk_surfaces` renders from it the
//! C constants, the C++ name accessors and the C++ test cases, and the Python
//! and TypeScript binding tests hold their names to it.

use crate::config::ReconnectPolicy;
use crate::fpss::DisconnectCause;
use crate::tdbe::types::enums::CalendarStatus;
use crate::{ImpliedVolSource, RemoveReason, SecType, StreamResponseType};

/// The name of a code outside a field whose core type has no name for one.
const UNKNOWN: &str = "UNKNOWN";

/// One field the C ABI carries as an integer code.
#[doc(hidden)]
pub struct CodedField {
    /// The field's name; the C++ name accessor is `<field>_name`.
    pub field: &'static str,
    /// The stem of the C constants, `THETADATADX_<c_prefix>_<NAME>`.
    pub c_prefix: String,
    /// What the codes are and where the C ABI carries them.
    pub doc: &'static str,
    /// Every code, in declaration order, with its name.
    pub codes: Vec<(i32, &'static str)>,
    /// The name of a code the field does not define.
    pub unknown: &'static str,
}

impl CodedField {
    /// Names each code with `name`, which resolves any code: what it gives
    /// `i32::MIN`, which no field defines, is the field's unknown name.
    fn new(
        field: &'static str,
        c_prefix: String,
        doc: &'static str,
        codes: impl IntoIterator<Item = i32>,
        name: impl Fn(i32) -> &'static str,
    ) -> Self {
        Self {
            field,
            c_prefix,
            doc,
            codes: codes.into_iter().map(|code| (code, name(code))).collect(),
            unknown: name(i32::MIN),
        }
    }
}

/// Adds a field for every enum getter of the config table: Python and
/// TypeScript read the setting as the enum's name, C and C++ as its code.
macro_rules! config_fields {
    ($fields:ident $($dir:ident $sym:ident $sig:tt $target:tt { $($body:tt)* })*) => {
        $(config_fields!(@entry $fields $dir $sym $($body)*);)*
    };
    (@entry $fields:ident get $sym:ident
        default: $default:ident,
        values: $enum:ident { $($code:literal $variant:ident),* $(,)? },
        $($rest:tt)*) => {
        let field = &stringify!($sym)["thetadatadx_config_get_".len()..];
        $fields.push(CodedField::new(
            field,
            field.to_ascii_uppercase(),
            concat!("Codes `", stringify!($sym), "` writes and its setter takes."),
            [$($code),*],
            |code| match code {
                $($code => crate::$enum::$variant.as_str(),)*
                _ => UNKNOWN,
            },
        ));
    };
    (@entry $($other:tt)*) => {};
}

/// Every coded field.
#[doc(hidden)]
#[must_use]
pub fn coded_fields() -> Vec<CodedField> {
    let mut fields = vec![
        CodedField::new(
            "sec_type",
            "SEC_TYPE".into(),
            "Security type of a streaming contract, carried by `ThetaDataDxContract.sec_type`. `UNKNOWN` marks a data event that arrived before its contract was resolved.",
            SecType::ALL.iter().map(|&v| v as i32),
            |code| {
                SecType::from_code(code)
                    .unwrap_or(SecType::Unknown)
                    .as_str()
            },
        ),
        CodedField::new(
            "result",
            "SUBSCRIPTION_RESULT".into(),
            "Outcome of a stream subscription request, carried by `ThetaDataDxStreamReqResponse.result` and named as the local server's WebSocket publishes it.",
            StreamResponseType::ALL.iter().map(|&v| i32::from(v as u8)),
            StreamResponseType::name_for_code,
        ),
        CodedField::new(
            "reason",
            "DISCONNECT_REASON".into(),
            "Disconnect reason, carried by the `reason` of the disconnected, reconnecting and reconnects-exhausted events: the server's own code when it sent a disconnect message, and otherwise the code the client reports for the way the connection ended. The `cause` beside it tells the two apart.",
            RemoveReason::ALL.iter().map(|&v| i32::from(v as i16)),
            |code| {
                i16::try_from(code)
                    .map_or(RemoveReason::Unspecified, RemoveReason::from_code)
                    .as_str()
            },
        ),
        CodedField::new(
            "cause",
            "DISCONNECT_CAUSE".into(),
            "Where a disconnect came from, carried by the `cause` of the disconnected, reconnecting and reconnects-exhausted events. `ReadFailed` and `WriteFailed` name the I/O error kind in `cause_io_error_kind`, and `ReadTimeout` the deadline that expired in `cause_timeout_ms`.",
            (0..).take(DisconnectCause::NAMES.len()),
            DisconnectCause::name_for_code,
        ),
        CodedField::new(
            "implied_vol_source",
            "IMPLIED_VOL_SOURCE".into(),
            "Where a binomial Greeks row's `implied_volatility` came from, carried by its `implied_vol_source` when `has_implied_vol_source` is set.",
            ImpliedVolSource::ALL.iter().map(|&v| v as i32),
            |code| ImpliedVolSource::from_code(code).map_or(UNKNOWN, ImpliedVolSource::as_str),
        ),
        CodedField::new(
            "calendar_status",
            "CALENDAR_STATUS".into(),
            "Day type of a `ThetaDataDxCalendarDay.status`, in the vendor's own vocabulary.",
            CalendarStatus::ALL.iter().map(|&v| v as i32),
            |code| CalendarStatus::from_code(code).map_or(UNKNOWN, CalendarStatus::as_str),
        ),
        CodedField::new(
            "reconnect_policy",
            "RECONNECT_POLICY".into(),
            "Codes `thetadatadx_config_get_reconnect_policy` writes. `thetadatadx_config_set_reconnect_policy` takes `AUTO` and `MANUAL`; installing a reconnect callback selects `CUSTOM`.",
            (0..).take(ReconnectPolicy::NAMES.len()),
            |code| {
                usize::try_from(code)
                    .ok()
                    .and_then(|index| ReconnectPolicy::NAMES.get(index))
                    .copied()
                    .unwrap_or(UNKNOWN)
            },
        ),
    ];
    crate::__config! { config_fields { fields } }
    fields
}
