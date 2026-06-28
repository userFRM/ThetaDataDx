//! Trade and quote condition lookup tables for `ThetaData` market data.
//!
//! Source-of-truth lives in
//! `thetadatadx-rs/data/{trade,quote}_conditions.toml`. At build time,
//! `build_support/conditions.rs` reads the TOML and regenerates
//! `tables_generated.rs`, which is committed so consumers building from
//! crates.io don't need to re-run codegen.
//!
//! The public surface — [`TradeCondition`], [`QuoteCondition`],
//! [`TRADE_CONDITIONS`], [`QUOTE_CONDITIONS`], and the eight lookup
//! functions — wraps the generated tables with O(1) array-index lookups.

mod tables_generated;

pub use tables_generated::{QUOTE_CONDITIONS, TRADE_CONDITIONS};

// ───────────────────────────────────────────────────────────────────────
//  Trade conditions
// ───────────────────────────────────────────────────────────────────────

/// A trade condition code with its properties.
// Reason: 8 booleans match the exchange specification flags 1:1.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TradeCondition {
    /// Numeric trade condition code as carried on the wire.
    pub code: i32,
    /// Short human-readable name for the condition (e.g. `"Regular"`).
    pub name: &'static str,
    /// Long-form description of the condition's market meaning.
    pub description: &'static str,
    /// Whether the condition marks the trade as a cancellation.
    pub cancel: bool,
    /// Whether the trade is reported late relative to its execution.
    pub late_report: bool,
    /// Whether the trade was executed automatically by the matching engine.
    pub auto_executed: bool,
    /// Whether the trade qualifies as the session's opening report.
    pub open_report: bool,
    /// Whether the trade contributes to the session volume total.
    pub volume: bool,
    /// Whether the trade is eligible to update the session high.
    pub high: bool,
    /// Whether the trade is eligible to update the session low.
    pub low: bool,
    /// Whether the trade is eligible to update the last (most recent) price.
    pub last: bool,
}

/// Look up the human-readable name for a trade condition code.
///
/// Returns `"UNKNOWN"` for codes outside the known range.
#[inline]
#[must_use]
pub fn condition_name(code: i32) -> &'static str {
    usize::try_from(code)
        .ok()
        .filter(|&idx| idx < TRADE_CONDITIONS.len())
        .map_or("UNKNOWN", |idx| TRADE_CONDITIONS[idx].name)
}

/// Look up the description for a trade condition code.
///
/// Returns `""` for codes outside the known range.
/// O(1) array-index lookup.
#[inline]
#[must_use]
pub fn condition_description(code: i32) -> &'static str {
    usize::try_from(code)
        .ok()
        .filter(|&idx| idx < TRADE_CONDITIONS.len())
        .map_or("", |idx| TRADE_CONDITIONS[idx].description)
}

/// True if this trade condition code represents a cancellation.
#[inline]
#[must_use]
pub fn is_cancel(code: i32) -> bool {
    usize::try_from(code)
        .ok()
        .filter(|&idx| idx < TRADE_CONDITIONS.len())
        .is_some_and(|idx| TRADE_CONDITIONS[idx].cancel)
}

/// True if this trade condition updates volume.
#[inline]
#[must_use]
pub fn updates_volume(code: i32) -> bool {
    usize::try_from(code)
        .ok()
        .filter(|&idx| idx < TRADE_CONDITIONS.len())
        .is_some_and(|idx| TRADE_CONDITIONS[idx].volume)
}

// ───────────────────────────────────────────────────────────────────────
//  Quote conditions
// ───────────────────────────────────────────────────────────────────────

/// A quote condition code with its properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuoteCondition {
    /// Numeric quote condition code as carried on the wire.
    pub code: i32,
    /// Short human-readable name for the condition (e.g. `"Regular"`).
    pub name: &'static str,
    /// Long-form description of the condition's market meaning.
    pub description: &'static str,
    /// Whether the quote is firm (binding) rather than indicative.
    pub firm: bool,
    /// Whether the quote reflects a trading halt on the instrument.
    pub halted: bool,
}

/// Look up the human-readable name for a quote condition code.
///
/// Returns `"UNKNOWN"` for codes outside the known range.
#[inline]
#[must_use]
pub fn quote_condition_name(code: i32) -> &'static str {
    usize::try_from(code)
        .ok()
        .filter(|&idx| idx < QUOTE_CONDITIONS.len())
        .map_or("UNKNOWN", |idx| QUOTE_CONDITIONS[idx].name)
}

/// Look up the description for a quote condition code.
///
/// Returns `""` for codes outside the known range.
#[inline]
#[must_use]
pub fn quote_condition_description(code: i32) -> &'static str {
    usize::try_from(code)
        .ok()
        .filter(|&idx| idx < QUOTE_CONDITIONS.len())
        .map_or("", |idx| QUOTE_CONDITIONS[idx].description)
}

/// True if this quote condition is firm (binding).
#[inline]
#[must_use]
pub fn is_firm(code: i32) -> bool {
    usize::try_from(code)
        .ok()
        .filter(|&idx| idx < QUOTE_CONDITIONS.len())
        .is_some_and(|idx| QUOTE_CONDITIONS[idx].firm)
}

/// True if this quote condition indicates a trading halt.
#[inline]
#[must_use]
pub fn is_halted(code: i32) -> bool {
    usize::try_from(code)
        .ok()
        .filter(|&idx| idx < QUOTE_CONDITIONS.len())
        .is_some_and(|idx| QUOTE_CONDITIONS[idx].halted)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trade condition tests
    #[test]
    fn trade_condition_name_valid() {
        assert_eq!(condition_name(0), "REGULAR");
        assert_eq!(condition_name(40), "CANC");
        assert_eq!(condition_name(148), "EXTENDEDHOURSTRADE");
    }

    #[test]
    fn trade_condition_name_out_of_range() {
        assert_eq!(condition_name(-1), "UNKNOWN");
        assert_eq!(condition_name(149), "UNKNOWN");
        assert_eq!(condition_name(9999), "UNKNOWN");
    }

    #[test]
    fn trade_condition_description_valid() {
        assert_eq!(condition_description(0), "Regular Trade");
        assert!(condition_description(13).contains("Sets Consolidated Last"));
        assert!(condition_description(5).contains("update last if only trade"));
    }

    #[test]
    fn trade_condition_description_out_of_range() {
        assert_eq!(condition_description(-1), "");
        assert_eq!(condition_description(149), "");
    }

    #[test]
    fn trade_is_cancel() {
        assert!(!is_cancel(0));
        assert!(is_cancel(40));
        assert!(is_cancel(41));
        assert!(is_cancel(42));
        assert!(is_cancel(43));
        assert!(is_cancel(44));
        assert!(!is_cancel(45));
    }

    #[test]
    fn trade_updates_volume() {
        assert!(updates_volume(0));
        assert!(updates_volume(1));
        assert!(!updates_volume(40));
    }

    #[test]
    fn trade_array_codes_contiguous() {
        for (i, tc) in TRADE_CONDITIONS.iter().enumerate() {
            assert_eq!(
                tc.code as usize, i,
                "Trade condition at index {} has code {}",
                i, tc.code
            );
        }
    }

    #[test]
    fn all_149_trade_conditions_present() {
        assert_eq!(TRADE_CONDITIONS.len(), 149);
    }

    // Quote condition tests
    #[test]
    fn quote_condition_name_valid() {
        assert_eq!(quote_condition_name(0), "REGULAR");
        assert_eq!(quote_condition_name(17), "HALTED");
        assert_eq!(quote_condition_name(50), "NATIONAL_BBO");
        assert_eq!(quote_condition_name(74), "RETAIL_QTE");
    }

    #[test]
    fn quote_condition_name_out_of_range() {
        assert_eq!(quote_condition_name(-1), "UNKNOWN");
        assert_eq!(quote_condition_name(75), "UNKNOWN");
    }

    #[test]
    fn quote_condition_description_valid() {
        assert_eq!(quote_condition_description(0), "Regular two-sided quote");
        assert_eq!(quote_condition_description(17), "Trading halted");
        assert!(quote_condition_description(66).contains("Level 1"));
    }

    #[test]
    fn quote_condition_description_out_of_range() {
        assert_eq!(quote_condition_description(-1), "");
        assert_eq!(quote_condition_description(75), "");
    }

    #[test]
    fn quote_is_firm() {
        assert!(is_firm(0));
        assert!(!is_firm(17));
    }

    #[test]
    fn quote_is_halted() {
        assert!(!is_halted(0));
        assert!(is_halted(17));
        assert!(is_halted(18));
    }

    #[test]
    fn quote_array_codes_contiguous() {
        for (i, qc) in QUOTE_CONDITIONS.iter().enumerate() {
            assert_eq!(
                qc.code as usize, i,
                "Quote condition at index {} has code {}",
                i, qc.code
            );
        }
    }

    #[test]
    fn all_75_quote_conditions_present() {
        assert_eq!(QUOTE_CONDITIONS.len(), 75);
    }

    #[test]
    fn all_trade_descriptions_have_content_where_expected() {
        // Codes that must have non-empty descriptions (key market-critical ones)
        let must_have_desc = [0, 1, 2, 5, 13, 40, 95, 148];
        for &code in &must_have_desc {
            assert!(
                !condition_description(code).is_empty(),
                "Trade condition {} should have a description",
                code
            );
        }
    }

    #[test]
    fn all_quote_descriptions_have_content() {
        for (i, qc) in QUOTE_CONDITIONS.iter().enumerate() {
            assert!(
                !qc.description.is_empty(),
                "Quote condition {} ({}) should have a description",
                i,
                qc.name
            );
        }
    }

    /// Round-trip pinning test: 10 entries copied verbatim from the
    /// pre-codegen `conditions.rs` (commit `bf5f8bc`). Catches any
    /// drift between the TOML source-of-truth and the generated
    /// `TRADE_CONDITIONS` / `QUOTE_CONDITIONS` arrays.
    ///
    /// If you intentionally change a condition entry, update the TOML
    /// AND update the corresponding pin below in the same commit.
    #[test]
    fn condition_tables_pin() {
        // ----- Trade pins -----

        let t0 = TRADE_CONDITIONS[0];
        assert_eq!(t0.code, 0);
        assert_eq!(t0.name, "REGULAR");
        assert_eq!(t0.description, "Regular Trade");
        assert_eq!(
            (
                t0.cancel,
                t0.late_report,
                t0.auto_executed,
                t0.open_report,
                t0.volume,
                t0.high,
                t0.low,
                t0.last
            ),
            (false, false, false, false, true, true, true, true)
        );

        let t30 = TRADE_CONDITIONS[30];
        assert_eq!(t30.code, 30);
        assert_eq!(t30.name, "DISTRIBUTION");
        assert_eq!(
            t30.description,
            "Sale of a large block of stock in a way that price is not adversely affected."
        );
        assert_eq!(
            (
                t30.cancel,
                t30.late_report,
                t30.auto_executed,
                t30.open_report,
                t30.volume,
                t30.high,
                t30.low,
                t30.last
            ),
            (false, false, false, false, true, true, true, true)
        );

        let t40 = TRADE_CONDITIONS[40];
        assert_eq!(t40.code, 40);
        assert_eq!(t40.name, "CANC");
        assert!(t40.cancel);
        assert!(!t40.volume);
        assert!(t40
            .description
            .starts_with("Cancel a previously reported trade"));

        let t60 = TRADE_CONDITIONS[60];
        assert_eq!(t60.code, 60);
        assert_eq!(t60.name, "SPECIALSESSION");
        assert_eq!(
            (
                t60.cancel,
                t60.late_report,
                t60.auto_executed,
                t60.open_report,
                t60.volume,
                t60.high,
                t60.low,
                t60.last
            ),
            (false, false, false, false, true, false, false, false)
        );

        let t90 = TRADE_CONDITIONS[90];
        assert_eq!(t90.code, 90);
        assert_eq!(t90.name, "POSTFULL");
        assert_eq!(t90.description, "");
        assert_eq!(
            (
                t90.cancel,
                t90.late_report,
                t90.auto_executed,
                t90.open_report,
                t90.volume,
                t90.high,
                t90.low,
                t90.last
            ),
            (false, false, false, false, false, false, false, false)
        );

        let t120 = TRADE_CONDITIONS[120];
        assert_eq!(t120.code, 120);
        assert_eq!(t120.name, "RESERVED_81");
        assert_eq!(t120.description, "");

        let t148 = TRADE_CONDITIONS[148];
        assert_eq!(t148.code, 148);
        assert_eq!(t148.name, "EXTENDEDHOURSTRADE");
        assert!(t148.volume);
        assert!(!t148.last);

        // Code 61 was the post-rename `PRICEVOLUMEADJ` per the prior
        // refactor; pin it to defend the rename.
        let t61 = TRADE_CONDITIONS[61];
        assert_eq!(t61.code, 61);
        assert_eq!(t61.name, "PRICEVOLUMEADJ");

        // ----- Quote pins -----

        let q0 = QUOTE_CONDITIONS[0];
        assert_eq!(q0.code, 0);
        assert_eq!(q0.name, "REGULAR");
        assert_eq!(q0.description, "Regular two-sided quote");
        assert_eq!((q0.firm, q0.halted), (true, false));

        let q17 = QUOTE_CONDITIONS[17];
        assert_eq!(q17.code, 17);
        assert_eq!(q17.name, "HALTED");
        assert_eq!(q17.description, "Trading halted");
        assert_eq!((q17.firm, q17.halted), (false, true));

        let q50 = QUOTE_CONDITIONS[50];
        assert_eq!(q50.code, 50);
        assert_eq!(q50.name, "NATIONAL_BBO");
        assert_eq!(q50.description, "National best bid and offer");

        let q74 = QUOTE_CONDITIONS[74];
        assert_eq!(q74.code, 74);
        assert_eq!(q74.name, "RETAIL_QTE");
        assert_eq!(q74.description, "Retail interest on both sides");
        assert_eq!((q74.firm, q74.halted), (false, false));
    }
}
