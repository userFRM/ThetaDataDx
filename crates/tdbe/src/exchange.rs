//! Exchange code lookup tables for `ThetaData` market data.
//!
//! Maps numeric exchange codes to human-readable names and MIC symbols.

/// An exchange with its numeric code, name, and symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exchange {
    pub code: i32,
    pub name: &'static str,
    pub symbol: &'static str,
}

/// All known exchange codes (0..77).
pub const EXCHANGES: [Exchange; 78] = [
    Exchange {
        code: 0,
        name: "NanexComp",
        symbol: "COMP",
    },
    Exchange {
        code: 1,
        name: "NasdaqExchange",
        symbol: "NQEX",
    },
    Exchange {
        code: 2,
        name: "NasdaqAlternativeDisplayFacility",
        symbol: "NQAD",
    },
    Exchange {
        code: 3,
        name: "NewYorkStockExchange",
        symbol: "NYSE",
    },
    Exchange {
        code: 4,
        name: "AmericanStockExchange",
        symbol: "AMEX",
    },
    Exchange {
        code: 5,
        name: "ChicagoBoardOptionsExchange",
        symbol: "CBOE",
    },
    Exchange {
        code: 6,
        name: "InternationalSecuritiesExchange",
        symbol: "ISEX",
    },
    Exchange {
        code: 7,
        name: "NYSEARCA(Pacific)",
        symbol: "PACF",
    },
    Exchange {
        code: 8,
        name: "NationalStockExchange(Cincinnati)",
        symbol: "CINC",
    },
    Exchange {
        code: 9,
        name: "PhiladelphiaStockExchange",
        symbol: "PHIL",
    },
    Exchange {
        code: 10,
        name: "OptionsPricingReportingAuthority",
        symbol: "OPRA",
    },
    Exchange {
        code: 11,
        name: "BostonStock/OptionsExchange",
        symbol: "BOST",
    },
    Exchange {
        code: 12,
        name: "NasdaqGlobal+SelectMarket(NMS)",
        symbol: "NQNM",
    },
    Exchange {
        code: 13,
        name: "NasdaqCapitalMarket(SmallCap)",
        symbol: "NQSC",
    },
    Exchange {
        code: 14,
        name: "NasdaqBulletinBoard",
        symbol: "NQBB",
    },
    Exchange {
        code: 15,
        name: "NasdaqOTC",
        symbol: "NQPK",
    },
    Exchange {
        code: 16,
        name: "NasdaqIndexes(GIDS)",
        symbol: "NQIX",
    },
    Exchange {
        code: 17,
        name: "ChicagoStockExchange",
        symbol: "CHIC",
    },
    Exchange {
        code: 18,
        name: "TorontoStockExchange",
        symbol: "TSE",
    },
    Exchange {
        code: 19,
        name: "CanadianVentureExchange",
        symbol: "CDNX",
    },
    Exchange {
        code: 20,
        name: "ChicageMercantileExchange",
        symbol: "CME",
    },
    Exchange {
        code: 21,
        name: "NewYorkBoardofTrade",
        symbol: "NYBT",
    },
    Exchange {
        code: 22,
        name: "ISEMercury",
        symbol: "MRCY",
    },
    Exchange {
        code: 23,
        name: "COMEX(divisionofNYMEX)",
        symbol: "COMX",
    },
    Exchange {
        code: 24,
        name: "ChicagoBoardofTrade",
        symbol: "CBOT",
    },
    Exchange {
        code: 25,
        name: "NewYorkMercantileExchange",
        symbol: "NYMX",
    },
    Exchange {
        code: 26,
        name: "KansasCityBoardofTrade",
        symbol: "KCBT",
    },
    Exchange {
        code: 27,
        name: "MinneapolisGrainExchange",
        symbol: "MGEX",
    },
    Exchange {
        code: 28,
        name: "NYSE/ARCABonds",
        symbol: "NYBO",
    },
    Exchange {
        code: 29,
        name: "NasdaqBasic",
        symbol: "NQBS",
    },
    Exchange {
        code: 30,
        name: "DowJonesIndices",
        symbol: "DOWJ",
    },
    Exchange {
        code: 31,
        name: "ISEGemini",
        symbol: "GEMI",
    },
    Exchange {
        code: 32,
        name: "SingaporeInternationalMonetaryExchange",
        symbol: "SIMX",
    },
    Exchange {
        code: 33,
        name: "LondonStockExchange",
        symbol: "FTSE",
    },
    Exchange {
        code: 34,
        name: "Eurex",
        symbol: "EURX",
    },
    Exchange {
        code: 35,
        name: "ImpliedPrice",
        symbol: "IMPL",
    },
    Exchange {
        code: 36,
        name: "DataTransmissionNetwork",
        symbol: "DTN",
    },
    Exchange {
        code: 37,
        name: "LondonMetalsExchangeMatchedTrades",
        symbol: "LMT",
    },
    Exchange {
        code: 38,
        name: "LondonMetalsExchange",
        symbol: "LME",
    },
    Exchange {
        code: 39,
        name: "IntercontinentalExchange(IPE)",
        symbol: "IPEX",
    },
    Exchange {
        code: 40,
        name: "NasdaqMutualFunds(MFDS)",
        symbol: "NQMF",
    },
    Exchange {
        code: 41,
        name: "COMEXClearport",
        symbol: "fcec",
    },
    Exchange {
        code: 42,
        name: "CBOEC2OptionExchange",
        symbol: "C2",
    },
    Exchange {
        code: 43,
        name: "MiamiExchange",
        symbol: "MIAX",
    },
    Exchange {
        code: 44,
        name: "NYMEXClearport",
        symbol: "CLRP",
    },
    Exchange {
        code: 45,
        name: "Barclays",
        symbol: "BARK",
    },
    Exchange {
        code: 46,
        name: "MiamiEmeraldOptionsExchange",
        symbol: "EMLD",
    },
    Exchange {
        code: 47,
        name: "NASDAQBoston",
        symbol: "NQBX",
    },
    Exchange {
        code: 48,
        name: "HotSpotEurexUS",
        symbol: "HOTS",
    },
    Exchange {
        code: 49,
        name: "EurexUS",
        symbol: "EUUS",
    },
    Exchange {
        code: 50,
        name: "EurexEU",
        symbol: "EUEU",
    },
    Exchange {
        code: 51,
        name: "EuronextCommodities",
        symbol: "ENCM",
    },
    Exchange {
        code: 52,
        name: "EuronextIndexDerivatives",
        symbol: "ENID",
    },
    Exchange {
        code: 53,
        name: "EuronextInterestRates",
        symbol: "ENIR",
    },
    Exchange {
        code: 54,
        name: "CBOEFuturesExchange",
        symbol: "CFE",
    },
    Exchange {
        code: 55,
        name: "PhiladelphiaBoardofTrade",
        symbol: "PBOT",
    },
    Exchange {
        code: 56,
        name: "FCME",
        symbol: "CMEFloor",
    },
    Exchange {
        code: 57,
        name: "FINRA/NASDAQTradeReportingFacility",
        symbol: "NQNX",
    },
    Exchange {
        code: 58,
        name: "BSETradeReportingFacility",
        symbol: "BTRF",
    },
    Exchange {
        code: 59,
        name: "NYSETradeReportingFacility",
        symbol: "NTRF",
    },
    Exchange {
        code: 60,
        name: "BATSTrading",
        symbol: "BATS",
    },
    Exchange {
        code: 61,
        name: "CBOTFloor",
        symbol: "FCBT",
    },
    Exchange {
        code: 62,
        name: "PinkSheets",
        symbol: "PINK",
    },
    Exchange {
        code: 63,
        name: "BATSYExchange",
        symbol: "BATY",
    },
    Exchange {
        code: 64,
        name: "DirectEdgeA",
        symbol: "EDGE",
    },
    Exchange {
        code: 65,
        name: "DirectEdgeX",
        symbol: "EDGX",
    },
    Exchange {
        code: 66,
        name: "RussellIndexes",
        symbol: "RUSL",
    },
    Exchange {
        code: 67,
        name: "CMEIndexes",
        symbol: "CMEX",
    },
    Exchange {
        code: 68,
        name: "InvestorsExchange",
        symbol: "IEX",
    },
    Exchange {
        code: 69,
        name: "MiamiPearlOptionsExchange",
        symbol: "PERL",
    },
    Exchange {
        code: 70,
        name: "LondonStockExchange",
        symbol: "LSE",
    },
    Exchange {
        code: 71,
        name: "NYSEGlobalIndexFeed",
        symbol: "GIF",
    },
    Exchange {
        code: 72,
        name: "TSXIndexes",
        symbol: "TSIX",
    },
    Exchange {
        code: 73,
        name: "MembersExchange",
        symbol: "MEMX",
    },
    Exchange {
        code: 74,
        name: "CBOECGI",
        symbol: "CGI",
    },
    Exchange {
        code: 75,
        name: "LongTermStockExchange",
        symbol: "LTSE",
    },
    Exchange {
        code: 76,
        name: "MIAXSapphire",
        symbol: "SPHR",
    },
    Exchange {
        code: 77,
        name: "24XNationalExchange",
        symbol: "24X",
    },
];

/// Look up the human-readable name for an exchange code.
///
/// Returns `"UNKNOWN"` for codes outside the known range.
#[inline]
#[must_use]
// reason: Exchange codes are non-negative i32 wire values bounded by the
// EXCHANGES array length (~78). Cast to usize is safe for valid codes.
#[allow(clippy::cast_sign_loss)]
pub fn exchange_name(code: i32) -> &'static str {
    if code >= 0 && (code as usize) < EXCHANGES.len() {
        EXCHANGES[code as usize].name
    } else {
        "UNKNOWN"
    }
}

/// Look up the symbol (MIC-like identifier) for an exchange code.
///
/// Returns `"UNKNOWN"` for codes outside the known range.
#[inline]
#[must_use]
// reason: Exchange codes are non-negative i32 wire values bounded by the
// EXCHANGES array length (~78). Cast to usize is safe for valid codes.
#[allow(clippy::cast_sign_loss)]
pub fn exchange_symbol(code: i32) -> &'static str {
    if code >= 0 && (code as usize) < EXCHANGES.len() {
        EXCHANGES[code as usize].symbol
    } else {
        "UNKNOWN"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exchange_name_valid() {
        assert_eq!(exchange_name(0), "NanexComp");
        assert_eq!(exchange_name(3), "NewYorkStockExchange");
        assert_eq!(exchange_name(68), "InvestorsExchange");
        assert_eq!(exchange_name(77), "24XNationalExchange");
    }

    #[test]
    fn exchange_name_out_of_range() {
        assert_eq!(exchange_name(-1), "UNKNOWN");
        assert_eq!(exchange_name(78), "UNKNOWN");
        assert_eq!(exchange_name(9999), "UNKNOWN");
    }

    #[test]
    fn exchange_symbol_valid() {
        assert_eq!(exchange_symbol(3), "NYSE");
        assert_eq!(exchange_symbol(5), "CBOE");
        assert_eq!(exchange_symbol(68), "IEX");
    }

    #[test]
    fn exchange_symbol_out_of_range() {
        assert_eq!(exchange_symbol(-1), "UNKNOWN");
        assert_eq!(exchange_symbol(78), "UNKNOWN");
    }

    #[test]
    fn array_codes_are_contiguous() {
        for (i, ex) in EXCHANGES.iter().enumerate() {
            assert_eq!(
                ex.code as usize, i,
                "Exchange at index {i} has code {}",
                ex.code
            );
        }
    }
}
