//! Human-readable display helpers for tick fields.
//!
//! All lookups are `const`-compatible: static arrays with index or match-based
//! access. No `HashMap`, no `LazyLock`, no allocations on the lookup path.
//!
//! # Time / Date formatting
//!
//! ```
//! # use tdbe::display;
//! assert_eq!(display::time_str(34_200_000), "09:30:00.000");
//! assert_eq!(display::date_str(20_240_315), "2024-03-15");
//! ```
//!
//! # Exchange lookup (78 exchanges, code 0..=77)
//!
//! ```
//! # use tdbe::display;
//! assert_eq!(display::exchange_name(1), "NasdaqExchange");
//! assert_eq!(display::exchange_symbol(3), "NYSE");
//! ```
//!
//! # Trade / quote condition names
//!
//! ```
//! # use tdbe::display;
//! assert_eq!(display::condition_name(0), "REGULAR");
//! assert_eq!(display::quote_condition_name(17), "HALTED");
//! ```

use core::fmt::Write;

// ═══════════════════════════════════════════════════════════════════════════════
//  Time helpers
// ═══════════════════════════════════════════════════════════════════════════════

/// Decompose milliseconds-of-day into `(hour, minute, second, millis)`.
#[inline]
pub fn time_hms(ms_of_day: i32) -> (u8, u8, u8, u16) {
    let ms = ms_of_day as u32;
    let total_secs = ms / 1000;
    let millis = (ms % 1000) as u16;
    let h = (total_secs / 3600) as u8;
    let m = ((total_secs % 3600) / 60) as u8;
    let s = (total_secs % 60) as u8;
    (h, m, s, millis)
}

/// Format milliseconds-of-day as `"HH:MM:SS.mmm"`.
pub fn time_str(ms_of_day: i32) -> String {
    let (h, m, s, ms) = time_hms(ms_of_day);
    let mut buf = String::with_capacity(12);
    let _ = write!(buf, "{h:02}:{m:02}:{s:02}.{ms:03}");
    buf
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Date helpers
// ═══════════════════════════════════════════════════════════════════════════════

/// Decompose a `YYYYMMDD` integer date into `(year, month, day)`.
#[inline]
pub fn date_ymd(date: i32) -> (i32, u8, u8) {
    let y = date / 10_000;
    let m = ((date % 10_000) / 100) as u8;
    let d = (date % 100) as u8;
    (y, m, d)
}

/// Format a `YYYYMMDD` integer date as `"YYYY-MM-DD"`.
pub fn date_str(date: i32) -> String {
    let (y, m, d) = date_ymd(date);
    let mut buf = String::with_capacity(10);
    let _ = write!(buf, "{y:04}-{m:02}-{d:02}");
    buf
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Exchange codes  (0..=77, direct index)
// ═══════════════════════════════════════════════════════════════════════════════

/// `(name, symbol)` for each exchange code, indexed by code.
const EXCHANGES: [(&str, &str); 78] = [
    ("NanexComp", "COMP"),                              // 0
    ("NasdaqExchange", "NQEX"),                         // 1
    ("NasdaqAlternativeDisplayFacility", "NQAD"),       // 2
    ("NewYorkStockExchange", "NYSE"),                   // 3
    ("AmericanStockExchange", "AMEX"),                  // 4
    ("ChicagoBoardOptionsExchange", "CBOE"),            // 5
    ("InternationalSecuritiesExchange", "ISEX"),        // 6
    ("NYSEARCA(Pacific)", "PACF"),                      // 7
    ("NationalStockExchange(Cincinnati)", "CINC"),      // 8
    ("PhiladelphiaStockExchange", "PHIL"),              // 9
    ("OptionsPricingReportingAuthority", "OPRA"),       // 10
    ("BostonStock/OptionsExchange", "BOST"),            // 11
    ("NasdaqGlobal+SelectMarket(NMS)", "NQNM"),         // 12
    ("NasdaqCapitalMarket(SmallCap)", "NQSC"),          // 13
    ("NasdaqBulletinBoard", "NQBB"),                    // 14
    ("NasdaqOTC", "NQPK"),                              // 15
    ("NasdaqIndexes(GIDS)", "NQIX"),                    // 16
    ("ChicagoStockExchange", "CHIC"),                   // 17
    ("TorontoStockExchange", "TSE"),                    // 18
    ("CanadianVentureExchange", "CDNX"),                // 19
    ("ChicagoMercantileExchange", "CME"),               // 20
    ("NewYorkBoardofTrade", "NYBT"),                    // 21
    ("ISEMercury", "MRCY"),                             // 22
    ("COMEX(divisionofNYMEX)", "COMX"),                 // 23
    ("ChicagoBoardofTrade", "CBOT"),                    // 24
    ("NewYorkMercantileExchange", "NYMX"),              // 25
    ("KansasCityBoardofTrade", "KCBT"),                 // 26
    ("MinneapolisGrainExchange", "MGEX"),               // 27
    ("NYSE/ARCABonds", "NYBO"),                         // 28
    ("NasdaqBasic", "NQBS"),                            // 29
    ("DowJonesIndices", "DOWJ"),                        // 30
    ("ISEGemini", "GEMI"),                              // 31
    ("SingaporeInternationalMonetaryExchange", "SIMX"), // 32
    ("LondonStockExchange", "FTSE"),                    // 33
    ("Eurex", "EURX"),                                  // 34
    ("ImpliedPrice", "IMPL"),                           // 35
    ("DataTransmissionNetwork", "DTN"),                 // 36
    ("LondonMetalsExchangeMatchedTrades", "LMT"),       // 37
    ("LondonMetalsExchange", "LME"),                    // 38
    ("IntercontinentalExchange(IPE)", "IPEX"),          // 39
    ("NasdaqMutualFunds(MFDS)", "NQMF"),                // 40
    ("COMEXClearport", "fcec"),                         // 41
    ("CBOEC2OptionExchange", "C2"),                     // 42
    ("MiamiExchange", "MIAX"),                          // 43
    ("NYMEXClearport", "CLRP"),                         // 44
    ("Barclays", "BARK"),                               // 45
    ("MiamiEmeraldOptionsExchange", "EMLD"),            // 46
    ("NASDAQBoston", "NQBX"),                           // 47
    ("HotSpotEurexUS", "HOTS"),                         // 48
    ("EurexUS", "EUUS"),                                // 49
    ("EurexEU", "EUEU"),                                // 50
    ("EuronextCommodities", "ENCM"),                    // 51
    ("EuronextIndexDerivatives", "ENID"),               // 52
    ("EuronextInterestRates", "ENIR"),                  // 53
    ("CBOEFuturesExchange", "CFE"),                     // 54
    ("PhiladelphiaBoardofTrade", "PBOT"),               // 55
    ("FCME", "CMEFloor"),                               // 56
    ("FINRA/NASDAQTradeReportingFacility", "NQNX"),     // 57
    ("BSETradeReportingFacility", "BTRF"),              // 58
    ("NYSETradeReportingFacility", "NTRF"),             // 59
    ("BATSTrading", "BATS"),                            // 60
    ("CBOTFloor", "FCBT"),                              // 61
    ("PinkSheets", "PINK"),                             // 62
    ("BATSYExchange", "BATY"),                          // 63
    ("DirectEdgeA", "EDGE"),                            // 64
    ("DirectEdgeX", "EDGX"),                            // 65
    ("RussellIndexes", "RUSL"),                         // 66
    ("CMEIndexes", "CMEX"),                             // 67
    ("InvestorsExchange", "IEX"),                       // 68
    ("MiamiPearlOptionsExchange", "PERL"),              // 69
    ("LondonStockExchange", "LSE"),                     // 70
    ("NYSEGlobalIndexFeed", "GIF"),                     // 71
    ("TSXIndexes", "TSIX"),                             // 72
    ("MembersExchange", "MEMX"),                        // 73
    ("CBOECGI", "CGI"),                                 // 74
    ("LongTermStockExchange", "LTSE"),                  // 75
    ("MIAXSapphire", "SPHR"),                           // 76
    ("24XNationalExchange", "24X"),                     // 77
];

/// Exchange full name by code. Returns `"UNKNOWN"` for out-of-range codes.
#[inline]
pub fn exchange_name(code: i32) -> &'static str {
    match EXCHANGES.get(code as usize) {
        Some((name, _)) => name,
        None => "UNKNOWN",
    }
}

/// Exchange symbol by code. Returns `"UNKNOWN"` for out-of-range codes.
#[inline]
pub fn exchange_symbol(code: i32) -> &'static str {
    match EXCHANGES.get(code as usize) {
        Some((_, sym)) => sym,
        None => "UNKNOWN",
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Trade condition codes  (0..=148, match-based)
// ═══════════════════════════════════════════════════════════════════════════════

/// Semantic flags for a trade condition.
pub struct TradeConditionInfo {
    pub name: &'static str,
    pub description: &'static str,
    pub cancel: bool,
    pub late_report: bool,
    pub volume: bool,
    pub high: bool,
    pub low: bool,
    pub last: bool,
}

/// Trade condition name by code. Returns `"UNKNOWN"` for unmapped codes.
pub fn condition_name(code: i32) -> &'static str {
    match condition_info(code) {
        Some(info) => info.name,
        None => "UNKNOWN",
    }
}

/// Full trade condition info by code.
pub fn condition_info(code: i32) -> Option<&'static TradeConditionInfo> {
    // Using a const array indexed by code for O(1) lookup.
    // All 149 conditions (0..=148) are contiguous.
    const TABLE: [TradeConditionInfo; 149] = [
        TradeConditionInfo {
            name: "REGULAR",
            description: "Regular Trade",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "FORMT",
            description: "Form T. Before and After Regular Hours.",
            cancel: false,
            late_report: false,
            volume: true,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "OUTOFSEQ",
            description: "Report was sent Out Of Sequence.",
            cancel: false,
            late_report: true,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "AVGPRC",
            description: "Average Price for a trade. NYSE/AMEX stocks.",
            cancel: false,
            late_report: false,
            volume: true,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "AVGPRC_NASDAQ",
            description: "Average Price. Nasdaq stocks.",
            cancel: false,
            late_report: false,
            volume: true,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "OPENREPORTLATE",
            description: "NYSE/AMEX. Market opened Late.",
            cancel: false,
            late_report: true,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "OPENREPORTOUTOFSEQ",
            description: "Report IS out of sequence. Market was open.",
            cancel: false,
            late_report: true,
            volume: true,
            high: true,
            low: true,
            last: false,
        },
        TradeConditionInfo {
            name: "OPENREPORTINSEQ",
            description: "Opening report. This is the first price.",
            cancel: false,
            late_report: true,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "PRIORREFERENCEPRICE",
            description: "Trade references price established earlier.",
            cancel: false,
            late_report: true,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "NEXTDAYSALE",
            description: "NYSE/AMEX: Next Day Clearing.",
            cancel: false,
            late_report: false,
            volume: true,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "BUNCHED",
            description: "Aggregate of 2 or more Regular trades.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "CASHSALE",
            description: "Delivery of securities and payment on the same day.",
            cancel: false,
            late_report: false,
            volume: true,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "SELLER",
            description: "Stock can be delivered up to 60 days later.",
            cancel: false,
            late_report: false,
            volume: true,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "SOLDLAST",
            description: "Late Reporting.",
            cancel: false,
            late_report: true,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "RULE127",
            description: "NYSE only. Rule 127 block trade.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "BUNCHEDSOLD",
            description: "Several trades bunched, report is late.",
            cancel: false,
            late_report: true,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "NONBOARDLOT",
            description: "Size of trade is less than a board lot (oddlot).",
            cancel: false,
            late_report: false,
            volume: true,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "POSIT",
            description: "POSIT Canada mid-point matching.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: false,
        },
        TradeConditionInfo {
            name: "AUTOEXECUTION",
            description: "Transaction executed electronically.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "HALT",
            description: "Temporary halt in trading.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "DELAYED",
            description: "Indicates a delayed opening.",
            cancel: false,
            late_report: false,
            volume: true,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "REOPEN",
            description: "Reopening of a previously halted contract.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "ACQUISITION",
            description: "Exchange Acquisition.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "CASHMARKET",
            description: "Cash only Market.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "NEXTDAYMARKET",
            description: "Next Day Only Market.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "BURSTBASKET",
            description: "Specialist basket execution.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "OPENDETAIL",
            description: "Opening/Reopening Trade Detail.",
            cancel: false,
            late_report: true,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "INTRADETAIL",
            description: "Detail trade of a previous trade.",
            cancel: false,
            late_report: true,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "BASKETONCLOSE",
            description: "Paired basket order on close.",
            cancel: false,
            late_report: true,
            volume: true,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "RULE155",
            description: "AMEX only rule 155.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "DISTRIBUTION",
            description: "Sale of a large block of stock.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "SPLIT",
            description: "Execution in 2 markets.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "REGULARSETTLE",
            description: "Regular settlement.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "CUSTOMBASKETCROSS",
            description: "Custom basket cross.",
            cancel: false,
            late_report: false,
            volume: true,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "ADJTERMS",
            description: "Terms adjusted for stock split/dividend.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "SPREAD",
            description: "Spread between 2 options.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "STRADDLE",
            description: "Straddle between 2 options.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "BUYWRITE",
            description: "Option part of a covered call.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "COMBO",
            description: "A buy and a sell in 2+ options.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "STPD",
            description: "Traded at agreed price following non-stopped trade.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "CANC",
            description: "Cancel a previously reported trade.",
            cancel: true,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "CANCLAST",
            description: "Cancel the most recent qualifying last trade.",
            cancel: true,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "CANCOPEN",
            description: "Cancel the opening trade report.",
            cancel: true,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "CANCONLY",
            description: "Cancel the only trade report.",
            cancel: true,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "CANCSTPD",
            description: "Cancel the STPD trade report.",
            cancel: true,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "MATCHCROSS",
            description: "Cross Trade from crossing session.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "FASTMARKET",
            description: "Unusually hectic market conditions.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "NOMINAL",
            description: "Nominal price for margin/risk evaluation.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "CABINET",
            description: "Deep out-of-the-money option trade.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "BLANKPRICE",
            description: "Sent by exchange to blank out a price.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "NOTSPECIFIED",
            description: "Unspecified condition.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "MCOFFICIALCLOSE",
            description: "Market Center official closing value.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "SPECIALTERMS",
            description: "All trades settled in non-regular manner.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "CONTINGENTORDER",
            description: "Contingent order on offsetting security.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "INTERNALCROSS",
            description: "Cross between two client accounts.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "STOPPEDREGULAR",
            description: "Stopped Stock Regular Trade.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "STOPPEDSOLDLAST",
            description: "Stopped Stock SoldLast Trade.",
            cancel: false,
            late_report: false,
            volume: false,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "STOPPEDOUTOFSEQ",
            description: "Stopped Stock Out of Sequence.",
            cancel: false,
            late_report: true,
            volume: false,
            high: true,
            low: true,
            last: false,
        },
        TradeConditionInfo {
            name: "BASIS",
            description: "Basket/index participation unit transaction.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "VWAP",
            description: "Volume Weighted Average Price.",
            cancel: false,
            late_report: false,
            volume: true,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "SPECIALSESSION",
            description: "Special Trading Session at last sale price.",
            cancel: false,
            late_report: false,
            volume: true,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "NANEXADMIN",
            description: "Volume and price corrections.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "OPENREPORT",
            description: "Opening trade report.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: false,
        },
        TradeConditionInfo {
            name: "MARKETONCLOSE",
            description: "Market Center closing value.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "SETTLEPRICE",
            description: "Settlement Price.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "OUTOFSEQPREMKT",
            description: "Out of sequence pre/post market trade.",
            cancel: false,
            late_report: true,
            volume: true,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "MCOFFICIALOPEN",
            description: "Market Center official opening value.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "FUTURESSPREAD",
            description: "Futures spread execution.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "OPENRANGE",
            description: "Opening range high/low.",
            cancel: false,
            late_report: false,
            volume: false,
            high: true,
            low: true,
            last: false,
        },
        TradeConditionInfo {
            name: "CLOSERANGE",
            description: "Closing range high/low.",
            cancel: false,
            late_report: false,
            volume: false,
            high: true,
            low: true,
            last: false,
        },
        TradeConditionInfo {
            name: "NOMINALCABINET",
            description: "Nominal Cabinet.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "CHANGINGTRANS",
            description: "Changing Transaction.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "CHANGINGTRANSCAB",
            description: "Changing Cabinet Transaction.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "NOMINALUPDATE",
            description: "Nominal price update.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "PITSETTLEMENT",
            description: "Pit session settlement price.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "BLOCKTRADE",
            description: "Large block trade (typically 10,000+ shares).",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "EXGFORPHYSICAL",
            description: "Exchange Future for Physical.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "VOLUMEADJUSTMENT",
            description: "Cumulative volume adjustment.",
            cancel: false,
            late_report: false,
            volume: true,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "VOLATILITYTRADE",
            description: "Volatility trade.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "YELLOWFLAG",
            description: "Exchange may be experiencing technical difficulties.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "FLOORPRICE",
            description: "Floor Bid/Ask on LME.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "OFFICIALPRICE",
            description: "Official bid/ask price on LME.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "UNOFFICIALPRICE",
            description: "Unofficial bid/ask price on LME.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "MIDBIDASKPRICE",
            description: "Mid bid-ask price on LME.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "ENDSESSIONHIGH",
            description: "End of Session High Price.",
            cancel: false,
            late_report: false,
            volume: false,
            high: true,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "ENDSESSIONLOW",
            description: "End of Session Low Price.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: true,
            last: false,
        },
        TradeConditionInfo {
            name: "BACKWARDATION",
            description: "Immediate delivery price > future delivery.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "CONTANGO",
            description: "Future delivery price > immediate delivery.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "HOLIDAY",
            description: "Holiday.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "PREOPENING",
            description: "Pre-opening period (7:00-9:30 AM).",
            cancel: false,
            late_report: false,
            volume: true,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "POSTFULL",
            description: "Post Full.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "POSTRESTRICTED",
            description: "Post Restricted.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "CLOSINGAUCTION",
            description: "Closing Auction.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "BATCH",
            description: "Batch.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "TRADING",
            description: "Trading.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "INTERMARKETSWEEP",
            description: "Intermarket Sweep Order Execution.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "DERIVATIVE",
            description: "Derivatively priced.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "REOPENING",
            description: "Market center re-opening prints.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "CLOSING",
            description: "Market center closing prints.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "CAPELECTION",
            description: "Odd Lot Trade (formerly Cap Election).",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: false,
        },
        TradeConditionInfo {
            name: "SPOTSETTLEMENT",
            description: "Spot Settlement.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "BASISHIGH",
            description: "Basis High.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: false,
        },
        TradeConditionInfo {
            name: "BASISLOW",
            description: "Basis Low.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: false,
        },
        TradeConditionInfo {
            name: "YIELD",
            description: "Yield (Cantor Treasuries).",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "PRICEVARIATION",
            description: "Price Variation.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "CONTINGENTTRADEFORMERLYSTOCKOPTION",
            description: "Contingent trade execution.",
            cancel: false,
            late_report: false,
            volume: true,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "STOPPEDIM",
            description: "Stopped at non-trade-through price.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: false,
        },
        TradeConditionInfo {
            name: "BENCHMARK",
            description: "Benchmark.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "TRADETHRUEXEMPT",
            description: "Trade Through Exempt.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: true,
        },
        TradeConditionInfo {
            name: "IMPLIED",
            description: "Spread trade leg price.",
            cancel: false,
            late_report: false,
            volume: true,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "OTC",
            description: "Over The Counter.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "MKTSUPERVISION",
            description: "Market Supervision.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "RESERVED_77",
            description: "",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "RESERVED_91",
            description: "",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "CONTINGENTUTP",
            description: "Contingent UTP.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "ODDLOT",
            description: "Trade with size between 1-99.",
            cancel: false,
            late_report: false,
            volume: true,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "RESERVED_89",
            description: "",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "CORRECTEDCSLAST",
            description: "Corrected consolidated last.",
            cancel: false,
            late_report: false,
            volume: false,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "OPRAEXTHOURS",
            description: "OPRA extended trading hours session.",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "RESERVED_78",
            description: "",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "RESERVED_81",
            description: "",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "RESERVED_84",
            description: "",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "RESERVED_878",
            description: "",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "RESERVED_90",
            description: "",
            cancel: false,
            late_report: false,
            volume: false,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "QUALIFIEDCONTINGENTTRADE",
            description: "Qualified Contingent Trade.",
            cancel: false,
            late_report: false,
            volume: true,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "SINGLELEGAUCTIONNONISO",
            description: "Single leg auction (non-ISO).",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "SINGLELEGAUCTIONISO",
            description: "Single leg auction (ISO).",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "SINGLELEGCROSSNONISO",
            description: "Single leg cross (non-ISO).",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "SINGLELEGCROSSISO",
            description: "Single leg cross (ISO).",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "SINGLELEGFLOORTRADE",
            description: "Single leg floor trade.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "MULTILEGAUTOELECTRONICTRADE",
            description: "Multi-leg auto electronic trade.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "MULTILEGAUCTION",
            description: "Multi-leg auction.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "MULTILEGCROSS",
            description: "Multi-leg cross.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "MULTILEGFLOORTRADE",
            description: "Multi-leg floor trade.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "MULTILEGAUTOELECTRADEAGAINSTSINGLELEG",
            description: "Multi-leg auto trade vs single leg.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "STOCKOPTIONSAUCTION",
            description: "Stock/options auction.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "MULTILEGAUCTIONAGAINSTSINGLELEG",
            description: "Multi-leg auction vs single leg.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "MULTILEGFLOORTRADEAGAINSTSINGLELEG",
            description: "Multi-leg floor trade vs single leg.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "STOCKOPTIONSAUTOELECTRADE",
            description: "Stock/options auto electronic trade.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "STOCKOPTIONSCROSS",
            description: "Stock/options cross.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "STOCKOPTIONSFLOORTRADE",
            description: "Stock/options floor trade.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "STOCKOPTIONSAUTOELECTRADEAGAINSTSINGLELEG",
            description: "Stock/options auto trade vs single leg.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "STOCKOPTIONSAUCTIONAGAINSTSINGLELEG",
            description: "Stock/options auction vs single leg.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "STOCKOPTIONSFLOORTRADEAGAINSTSINGLELEG",
            description: "Stock/options floor trade vs single leg.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "MULTILEGFLOORTRADEOFPROPRIETARYPRODUCTS",
            description: "Multi-leg proprietary products floor trade.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "BIDAGGRESSOR",
            description: "Aggressor on the buy side.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "ASKAGGRESSOR",
            description: "Aggressor on the sell side.",
            cancel: false,
            late_report: false,
            volume: true,
            high: true,
            low: true,
            last: true,
        },
        TradeConditionInfo {
            name: "MULTILATERALCOMPRESSIONTRADEOFPROPRIETARYDATAPRODUCTS",
            description: "Multilateral compression trade.",
            cancel: false,
            late_report: false,
            volume: true,
            high: false,
            low: false,
            last: false,
        },
        TradeConditionInfo {
            name: "EXTENDEDHOURSTRADE",
            description: "Trade executed outside regular market hours.",
            cancel: false,
            late_report: false,
            volume: true,
            high: false,
            low: false,
            last: false,
        },
    ];

    if code >= 0 && (code as usize) < TABLE.len() {
        Some(&TABLE[code as usize])
    } else {
        None
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Quote condition codes  (0..=74, direct index)
// ═══════════════════════════════════════════════════════════════════════════════

/// Quote condition names, indexed by code.
const QUOTE_CONDITIONS: [&str; 75] = [
    "REGULAR",              // 0
    "BID_ASK_AUTO_EXEC",    // 1
    "ROTATION",             // 2
    "SPECIALIST_ASK",       // 3
    "SPECIALIST_BID",       // 4
    "LOCKED",               // 5
    "FAST_MARKET",          // 6
    "SPECIALIST_BID_ASK",   // 7
    "ONE_SIDE",             // 8
    "OPENING_QUOTE",        // 9
    "CLOSING_QUOTE",        // 10
    "MARKET_MAKER_CLOSED",  // 11
    "DEPTH_ON_ASK",         // 12
    "DEPTH_ON_BID",         // 13
    "DEPTH_ON_BID_ASK",     // 14
    "TIER_3",               // 15
    "CROSSED",              // 16
    "HALTED",               // 17
    "OPERATIONAL_HALT",     // 18
    "NEWS_OUT",             // 19
    "NEWS_PENDING",         // 20
    "NON_FIRM",             // 21
    "DUE_TO_RELATED",       // 22
    "RESUME",               // 23
    "NO_MARKET_MAKERS",     // 24
    "ORDER_IMBALANCE",      // 25
    "ORDER_INFLUX",         // 26
    "INDICATED",            // 27
    "PRE_OPEN",             // 28
    "IN_VIEW_OF_COMMON",    // 29
    "RELATED_NEWS_PENDING", // 30
    "RELATED_NEWS_OUT",     // 31
    "ADDITIONAL_INFO",      // 32
    "RELATED_ADD_INFO",     // 33
    "NO_OPEN_RESUME",       // 34
    "DELETED",              // 35
    "REGULATORY_HALT",      // 36
    "SEC_SUSPENSION",       // 37
    "NON_COMLIANCE",        // 38
    "FILINGS_NOT_CURRENT",  // 39
    "CATS_HALTED",          // 40
    "CATS",                 // 41
    "EX_DIV_OR_SPLIT",      // 42
    "UNASSIGNED",           // 43
    "INSIDE_OPEN",          // 44
    "INSIDE_CLOSED",        // 45
    "OFFER_WANTED",         // 46
    "BID_WANTED",           // 47
    "CASH",                 // 48
    "INACTIVE",             // 49
    "NATIONAL_BBO",         // 50
    "NOMINAL",              // 51
    "CABINET",              // 52
    "NOMINAL_CABINET",      // 53
    "BLANK_PRICE",          // 54
    "SLOW_BID_ASK",         // 55
    "SLOW_LIST",            // 56
    "SLOW_BID",             // 57
    "SLOW_ASK",             // 58
    "BID_OFFER_WANTED",     // 59
    "SUBPENNY",             // 60
    "NON_BBO",              // 61
    "SPECIAL_OPEN",         // 62
    "BENCHMARK",            // 63
    "IMPLIED",              // 64
    "EXCHANGE_BEST",        // 65
    "MKT_WIDE_HALT_1",      // 66
    "MKT_WIDE_HALT_2",      // 67
    "MKT_WIDE_HALT_3",      // 68
    "ON_DEMAND_AUCTION",    // 69
    "NON_FIRM_BID",         // 70
    "NON_FIRM_ASK",         // 71
    "RETAIL_BID",           // 72
    "RETAIL_ASK",           // 73
    "RETAIL_QTE",           // 74
];

/// Quote condition name by code. Returns `"UNKNOWN"` for unmapped codes.
#[inline]
pub fn quote_condition_name(code: i32) -> &'static str {
    match QUOTE_CONDITIONS.get(code as usize) {
        Some(name) => name,
        None => "UNKNOWN",
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Option right helper
// ═══════════════════════════════════════════════════════════════════════════════

/// Decode the `right` field: ASCII 67 = `"C"`, 80 = `"P"`, else `""`.
#[inline]
pub fn right_str(code: i32) -> &'static str {
    match code {
        67 => "C",
        80 => "P",
        _ => "",
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Request type constants
// ═══════════════════════════════════════════════════════════════════════════════

pub const REQUEST_TYPE_TRADE: &str = "trade";
pub const REQUEST_TYPE_QUOTE: &str = "quote";

// ═══════════════════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── Time helpers ──

    #[test]
    fn test_time_hms_market_open() {
        assert_eq!(time_hms(34_200_000), (9, 30, 0, 0));
    }

    #[test]
    fn test_time_str_market_open() {
        assert_eq!(time_str(34_200_000), "09:30:00.000");
    }

    #[test]
    fn test_time_str_with_millis() {
        assert_eq!(time_str(34_200_123), "09:30:00.123");
    }

    #[test]
    fn test_time_str_midnight() {
        assert_eq!(time_str(0), "00:00:00.000");
    }

    #[test]
    fn test_time_str_end_of_day() {
        // 23:59:59.999
        assert_eq!(time_str(86_399_999), "23:59:59.999");
    }

    // ── Date helpers ──

    #[test]
    fn test_date_ymd() {
        assert_eq!(date_ymd(20_240_315), (2024, 3, 15));
    }

    #[test]
    fn test_date_str() {
        assert_eq!(date_str(20_240_315), "2024-03-15");
    }

    #[test]
    fn test_date_str_jan_first() {
        assert_eq!(date_str(20_250_101), "2025-01-01");
    }

    // ── Exchange lookups ──

    #[test]
    fn test_exchange_name_nasdaq() {
        assert_eq!(exchange_name(1), "NasdaqExchange");
    }

    #[test]
    fn test_exchange_symbol_nyse() {
        assert_eq!(exchange_symbol(3), "NYSE");
    }

    #[test]
    fn test_exchange_iex() {
        assert_eq!(exchange_name(68), "InvestorsExchange");
        assert_eq!(exchange_symbol(68), "IEX");
    }

    #[test]
    fn test_exchange_out_of_range() {
        assert_eq!(exchange_name(999), "UNKNOWN");
        assert_eq!(exchange_symbol(-1), "UNKNOWN");
    }

    #[test]
    fn test_exchange_last_entry() {
        assert_eq!(exchange_name(77), "24XNationalExchange");
        assert_eq!(exchange_symbol(77), "24X");
    }

    // ── Trade condition lookups ──

    #[test]
    fn test_condition_regular() {
        assert_eq!(condition_name(0), "REGULAR");
        let info = condition_info(0).unwrap();
        assert!(!info.cancel);
        assert!(info.volume);
        assert!(info.last);
    }

    #[test]
    fn test_condition_canc() {
        assert_eq!(condition_name(40), "CANC");
        let info = condition_info(40).unwrap();
        assert!(info.cancel);
        assert!(!info.volume);
    }

    #[test]
    fn test_condition_intermarket_sweep() {
        assert_eq!(condition_name(95), "INTERMARKETSWEEP");
    }

    #[test]
    fn test_condition_last_entry() {
        assert_eq!(condition_name(148), "EXTENDEDHOURSTRADE");
    }

    #[test]
    fn test_condition_out_of_range() {
        assert_eq!(condition_name(999), "UNKNOWN");
        assert!(condition_info(999).is_none());
        assert_eq!(condition_name(-1), "UNKNOWN");
    }

    // ── Quote condition lookups ──

    #[test]
    fn test_quote_condition_regular() {
        assert_eq!(quote_condition_name(0), "REGULAR");
    }

    #[test]
    fn test_quote_condition_halted() {
        assert_eq!(quote_condition_name(17), "HALTED");
    }

    #[test]
    fn test_quote_condition_last_entry() {
        assert_eq!(quote_condition_name(74), "RETAIL_QTE");
    }

    #[test]
    fn test_quote_condition_out_of_range() {
        assert_eq!(quote_condition_name(999), "UNKNOWN");
    }

    // ── Right helper ──

    #[test]
    fn test_right_str_call() {
        assert_eq!(right_str(67), "C");
    }

    #[test]
    fn test_right_str_put() {
        assert_eq!(right_str(80), "P");
    }

    #[test]
    fn test_right_str_neither() {
        assert_eq!(right_str(0), "");
    }
}
