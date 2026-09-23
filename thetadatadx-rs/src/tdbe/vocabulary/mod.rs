//! The vendor's own vocabulary for the things its protocol addresses.
//!
//! These are names, not wire values: the columns a response can carry, the
//! request families the service exposes, the arguments those requests take,
//! the output formats, the account tiers, and the calendar query kinds. Most
//! already appear on this SDK's published reference pages; the tables collect
//! them in one place so a caller can check a name against the vendor's
//! vocabulary rather than against this SDK's spelling of it.
//!
//! The SDK does not send most of them. It addresses the typed gRPC surface,
//! where the request family is the method and its arguments are the message
//! fields, so there is no place to put a request-family name on the wire.
//! They are carried as reference vocabulary.
//!
//! Five of this SDK's column names resolve onto four of the vendor's; see
//! [`vendor_column_name`].
//!
//! The tables cover every enum the vendor's protocol defines, including the
//! ones this SDK models as typed enums of its own. `SEC_TYPE` carries
//! `IGNORE`, which this SDK spells `Unknown` on its own surface because that
//! is what it means to a caller holding one; the vendor's spelling is here.

mod tables_generated;

pub use tables_generated::{
    ACCOUNT_TYPE, CALENDAR_TYPE, DATA_TYPE, RATE_TYPE, REMOVE_REASON, REQ_ARG, REQ_TYPE,
    RESULTS_FORMAT, SEC_TYPE, STREAM_MSG_TYPE, STREAM_RESPONSE_TYPE,
};

/// The vendor's name for a column this SDK spells differently.
///
/// Every other column on every tick carries the vendor's own name, so this
/// answers `None` for all of them, and for a name that is not a column at all.
/// It exists so a caller joining this SDK's output against the vendor's column
/// vocabulary can resolve the five that were renamed here.
///
/// ```
/// use thetadatadx::utils::vocabulary::vendor_column_name;
///
/// assert_eq!(vendor_column_name("implied_volatility"), Some("IMPLIED_VOL"));
/// assert_eq!(vendor_column_name("underlying_ms_of_day"), Some("MS_OF_DAY2"));
/// assert_eq!(vendor_column_name("bid"), None);
/// ```
#[must_use]
pub fn vendor_column_name(sdk_column: &str) -> Option<&'static str> {
    match sdk_column {
        "implied_volatility" => Some("IMPLIED_VOL"),
        "ask_implied_volatility" => Some("ASK_IMPLIED_VOL"),
        "bid_implied_volatility" => Some("BID_IMPLIED_VOL"),
        // One vendor column, carried under two names depending on which tick
        // holds it: the greeks ticks pair it with the underlying, the
        // trade-quote tick pairs it with the quote.
        "underlying_ms_of_day" | "quote_ms_of_day" => Some("MS_OF_DAY2"),
        _ => None,
    }
}

/// Whether `name` is in the vendor's column vocabulary.
#[must_use]
pub fn is_vendor_column(name: &str) -> bool {
    DATA_TYPE.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The renamed columns must resolve to a name the vendor's own column
    /// vocabulary contains. A mapping to a name that is not in `DATA_TYPE`
    /// would send a caller looking for a column the vendor does not have.
    #[test]
    fn every_renamed_column_resolves_into_the_vendor_vocabulary() {
        for sdk in [
            "implied_volatility",
            "ask_implied_volatility",
            "bid_implied_volatility",
            "underlying_ms_of_day",
            "quote_ms_of_day",
        ] {
            let vendor = vendor_column_name(sdk).expect("a renamed column maps");
            assert!(
                is_vendor_column(vendor),
                "{sdk} maps to {vendor}, which is not in the vendor column vocabulary"
            );
        }
    }

    /// A column this SDK spells the vendor's way must not claim a rename.
    #[test]
    fn a_column_carrying_the_vendor_name_reports_no_rename() {
        for same in ["bid", "ask", "ms_of_day", "date", "condition", "exchange"] {
            assert_eq!(vendor_column_name(same), None, "{same}");
        }
    }
}
