//! The wire-string enums of the endpoint parameters.
//!
//! `__enums!` lists each enum once. The engine expands it into the Rust
//! enums, the Python and TypeScript bindings into their string enums, and
//! `generate_sdk_surfaces` into the values it holds each enum parameter's
//! "Accepted values" description to.

/// Every wire-string enum, in declaration order.
///
/// `__enums! { consumer { ctx.. } }` expands to
/// `consumer! { ctx.. Right { Call "call" CALL ["C" "c"], .. } .. }`: each
/// variant is its Rust name, its wire value, its Python name and the other
/// spellings the engine also parses as it.
#[doc(hidden)]
#[macro_export]
macro_rules! __enums {
    ($m:ident { $($ctx:tt)* }) => {
        $m! {
            $($ctx)*
            Right {
                Call "call" CALL ["C" "c"],
                Put "put" PUT ["P" "p"],
                Both "both" BOTH ["*"],
            }
            Venue {
                Nqb "nqb" NQB [],
                UtpCta "utp_cta" UTP_CTA [],
            }
            Interval {
                Tick "tick" TICK [],
                Ms10 "10ms" MS_10 [],
                Ms100 "100ms" MS_100 [],
                Ms500 "500ms" MS_500 [],
                S1 "1s" S_1 [],
                S5 "5s" S_5 [],
                S10 "10s" S_10 [],
                S15 "15s" S_15 [],
                S30 "30s" S_30 [],
                M1 "1m" M_1 [],
                M5 "5m" M_5 [],
                M10 "10m" M_10 [],
                M15 "15m" M_15 [],
                M30 "30m" M_30 [],
                H1 "1h" H_1 [],
            }
            RateType {
                Sofr "sofr" SOFR [],
                TreasuryM1 "treasury_m1" TREASURY_M1 [],
                TreasuryM3 "treasury_m3" TREASURY_M3 [],
                TreasuryM6 "treasury_m6" TREASURY_M6 [],
                TreasuryY1 "treasury_y1" TREASURY_Y1 [],
                TreasuryY2 "treasury_y2" TREASURY_Y2 [],
                TreasuryY3 "treasury_y3" TREASURY_Y3 [],
                TreasuryY5 "treasury_y5" TREASURY_Y5 [],
                TreasuryY7 "treasury_y7" TREASURY_Y7 [],
                TreasuryY10 "treasury_y10" TREASURY_Y10 [],
                TreasuryY20 "treasury_y20" TREASURY_Y20 [],
                TreasuryY30 "treasury_y30" TREASURY_Y30 [],
            }
            RequestType {
                Trade "trade" TRADE [],
                Quote "quote" QUOTE [],
                Eod "eod" EOD [],
                Ohlc "ohlc" OHLC [],
            }
            Version {
                Latest "latest" LATEST [],
                V1 "1" V1 [],
            }
        }
    };
}
