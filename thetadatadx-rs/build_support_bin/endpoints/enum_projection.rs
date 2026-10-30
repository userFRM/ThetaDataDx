//! The wire-string enums of the endpoint parameters, with every
//! per-language name the enum emitters need.

/// One enum of the endpoint parameters.
pub(super) struct EnumProjection {
    pub(super) name: &'static str,
    pub(super) variants: &'static [EnumVariantProjection],
}

/// One enum variant: its wire string and its Rust and Python names.
pub(super) struct EnumVariantProjection {
    pub(super) wire: &'static str,
    pub(super) rust: &'static str,
    pub(super) python: &'static str,
}

const fn variant(
    wire: &'static str,
    rust: &'static str,
    python: &'static str,
) -> EnumVariantProjection {
    EnumVariantProjection { wire, rust, python }
}

pub(super) const ENUMS: &[EnumProjection] = &[
    EnumProjection {
        name: "Right",
        variants: &[
            variant("call", "Call", "CALL"),
            variant("put", "Put", "PUT"),
            variant("both", "Both", "BOTH"),
        ],
    },
    EnumProjection {
        name: "Venue",
        variants: &[
            variant("nqb", "Nqb", "NQB"),
            variant("utp_cta", "UtpCta", "UTP_CTA"),
        ],
    },
    EnumProjection {
        name: "Interval",
        variants: &[
            variant("tick", "Tick", "TICK"),
            variant("10ms", "Ms10", "MS_10"),
            variant("100ms", "Ms100", "MS_100"),
            variant("500ms", "Ms500", "MS_500"),
            variant("1s", "S1", "S_1"),
            variant("5s", "S5", "S_5"),
            variant("10s", "S10", "S_10"),
            variant("15s", "S15", "S_15"),
            variant("30s", "S30", "S_30"),
            variant("1m", "M1", "M_1"),
            variant("5m", "M5", "M_5"),
            variant("10m", "M10", "M_10"),
            variant("15m", "M15", "M_15"),
            variant("30m", "M30", "M_30"),
            variant("1h", "H1", "H_1"),
        ],
    },
    EnumProjection {
        name: "RateType",
        variants: &[
            variant("sofr", "Sofr", "SOFR"),
            variant("treasury_m1", "TreasuryM1", "TREASURY_M1"),
            variant("treasury_m3", "TreasuryM3", "TREASURY_M3"),
            variant("treasury_m6", "TreasuryM6", "TREASURY_M6"),
            variant("treasury_y1", "TreasuryY1", "TREASURY_Y1"),
            variant("treasury_y2", "TreasuryY2", "TREASURY_Y2"),
            variant("treasury_y3", "TreasuryY3", "TREASURY_Y3"),
            variant("treasury_y5", "TreasuryY5", "TREASURY_Y5"),
            variant("treasury_y7", "TreasuryY7", "TREASURY_Y7"),
            variant("treasury_y10", "TreasuryY10", "TREASURY_Y10"),
            variant("treasury_y20", "TreasuryY20", "TREASURY_Y20"),
            variant("treasury_y30", "TreasuryY30", "TREASURY_Y30"),
        ],
    },
    EnumProjection {
        name: "RequestType",
        variants: &[
            variant("trade", "Trade", "TRADE"),
            variant("quote", "Quote", "QUOTE"),
            variant("eod", "Eod", "EOD"),
            variant("ohlc", "Ohlc", "OHLC"),
        ],
    },
    EnumProjection {
        name: "Version",
        variants: &[
            variant("latest", "Latest", "LATEST"),
            variant("1", "V1", "V1"),
        ],
    },
];
