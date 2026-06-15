//! Contract identifier, OCC-21 parser, and wire serialization codec.
//!
//! ## Contract serialization
//!
//! Contracts are serialized as a compact binary format on the wire:
//!
//! - **Stock/Index**: `[total_size: u8] [root_len: u8] [root ASCII] [sec_type: u8]`
//! - **Option**:      `[total_size: u8] [root_len: u8] [root ASCII] [sec_type: u8]
//!                      [exp_date: i32 BE] [is_call: u8] [strike: i32 BE]`

use std::sync::Arc;

use crate::tdbe::types::enums::SecType;
use crate::tdbe::Right;

use crate::error::Error;

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

/// The expiration / strike / right of an option leg, passed by name to
/// [`Contract::option`].
///
/// All three are strings, so a positional `(expiration, strike, right)`
/// argument list lets a transposed pair compile silently. Taking them as
/// named struct fields makes the contract identity non-transposable: the
/// caller spells each field, so a swap is a visible mislabel rather than a
/// silent positional error.
///
/// ```
/// use thetadatadx::fpss::protocol::{Contract, OptionLeg};
///
/// let c = Contract::option(
///     "SPY",
///     OptionLeg { expiration: "20260417", strike: "550", right: "C" },
/// )?;
/// assert_eq!(c.expiration, Some(20_260_417));
/// # Ok::<(), thetadatadx::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OptionLeg<'a> {
    /// Expiration date as `YYYYMMDD` (dashes are stripped before parsing).
    pub expiration: &'a str,
    /// Strike price in dollars (e.g. `"550"` or `"550.50"`).
    pub strike: &'a str,
    /// Option right: `"C"` / `"CALL"` / `"P"` / `"PUT"` (case-insensitive).
    pub right: &'a str,
}

/// A contract identifier for FPSS subscriptions.
///
/// Matches the FPSS contract wire format:
/// - Stock/Index/Rate: root ticker + security type
/// - Option: root ticker + security type + expiration + call/put + strike
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Contract {
    /// Ticker symbol (ASCII, max ~6 chars in practice). Named `symbol` to
    /// match the v3 vendor surface; the wire codec still encodes it as the
    /// root field per the contract wire format.
    ///
    /// Stored as `Arc<str>` so every `Arc<Contract>` clone that flows
    /// through the hot-path event ring pays only an atomic refcount
    /// increment — no heap allocation per event.
    pub symbol: Arc<str>,
    /// Security type.
    pub sec_type: SecType,
    /// Expiration date as YYYYMMDD integer (options only).
    pub expiration: Option<i32>,
    /// True = call, false = put (options only).
    pub is_call: Option<bool>,
    /// Strike price in thousandths of a dollar (options only) — the
    /// FPSS wire codec's fixed-point integer encoding (a `$550.00`
    /// strike is `Some(550_000)`). Named with its unit so the dollar
    /// surface is unambiguous: read dollars via
    /// [`Self::strike_dollars`]; every non-Rust binding exposes
    /// `strike` in dollars only.
    pub strike_thousandths: Option<i32>,
}

impl Contract {
    /// Create a stock contract.
    ///
    /// A root with no security type defaults to STOCK on the wire.
    ///
    /// ```
    /// use thetadatadx::fpss::protocol::Contract;
    /// use thetadatadx::SecType;
    ///
    /// let c = Contract::stock("AAPL");
    /// assert_eq!(&*c.symbol, "AAPL");
    /// assert_eq!(c.sec_type, SecType::Stock);
    /// assert!(c.expiration.is_none());
    /// ```
    pub fn stock(symbol: &str) -> Self {
        Self {
            symbol: Arc::from(symbol),
            sec_type: SecType::Stock,
            expiration: None,
            is_call: None,
            strike_thousandths: None,
        }
    }

    /// Create an index contract.
    pub fn index(symbol: &str) -> Self {
        Self {
            symbol: Arc::from(symbol),
            sec_type: SecType::Index,
            expiration: None,
            is_call: None,
            strike_thousandths: None,
        }
    }

    /// Create a rate contract.
    pub fn rate(symbol: &str) -> Self {
        Self {
            symbol: Arc::from(symbol),
            sec_type: SecType::Rate,
            expiration: None,
            is_call: None,
            strike_thousandths: None,
        }
    }

    /// Create an option contract from string-formatted parameters.
    ///
    /// The expiration / strike / right travel as a named [`OptionLeg`] so
    /// the contract identity cannot be transposed: all three are strings,
    /// and a positional list would let a swapped pair compile silently.
    ///
    /// For callers that already hold wire-format integer triples (e.g.
    /// the in-process WS server parsing the JSON wire format), use
    /// [`Self::option_raw`] instead.
    ///
    /// ```
    /// use thetadatadx::fpss::protocol::{Contract, OptionLeg};
    ///
    /// let c = Contract::option(
    ///     "SPY",
    ///     OptionLeg { expiration: "20260417", strike: "550", right: "C" },
    /// )?;
    /// assert_eq!(&*c.symbol, "SPY");
    /// assert_eq!(c.expiration, Some(20_260_417));
    /// assert_eq!(c.is_call, Some(true));
    /// assert_eq!(c.strike_thousandths, Some(550_000));
    /// # Ok::<(), thetadatadx::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] on validation failure: unparseable
    /// expiration, non-strict right value, unparseable strike, or
    /// strike outside `i32` range after `*1000` scaling.
    pub fn option(symbol: &str, leg: OptionLeg<'_>) -> Result<Self, Error> {
        let OptionLeg {
            expiration,
            strike,
            right,
        } = leg;
        let exp: i32 = expiration.replace('-', "").parse().map_err(|e| {
            Error::config_invalid(
                "contract.expiration",
                format!("invalid expiration date {expiration:?}: {e}"),
            )
        })?;
        // Reject impossible expirations (00000000, 20260230,
        // 19990431, …) on every public option-builder input. Uses the
        // same canonical Gregorian validator the MDDS validator calls
        // (`crate::tdbe::time::is_valid_yyyymmdd`) so the two surfaces agree
        // on what counts as a real calendar date.
        if !crate::tdbe::time::is_valid_yyyymmdd(exp) {
            return Err(Error::config_invalid(
                "contract.expiration",
                format!(
                    "invalid expiration date {expiration:?}: not a valid Gregorian date (YYYYMMDD with year 1900-2100, valid month, day-of-month including 4/100/400 leap rule)"
                ),
            ));
        }
        let is_call = crate::tdbe::right::parse_right_strict(right)?
            .as_is_call()
            .ok_or_else(|| {
                Error::config_internal("parse_right_strict returned Both despite strict mode")
            })?;
        let strike_dollars: f64 = strike.parse().map_err(|e| {
            Error::config_invalid(
                "contract.strike_thousandths",
                format!("invalid strike price {strike:?}: {e}"),
            )
        })?;
        let strike_scaled = (strike_dollars * 1000.0).round();
        if !strike_scaled.is_finite()
            || strike_scaled < f64::from(i32::MIN)
            || strike_scaled > f64::from(i32::MAX)
        {
            return Err(Error::config_invalid(
                "contract.strike_thousandths",
                format!("strike {strike_dollars} out of i32 range after *1000 scaling"),
            ));
        }
        // Reason: bounds checked above.
        #[allow(clippy::cast_possible_truncation)]
        let strike_raw = strike_scaled as i32;
        Ok(Self::option_raw(symbol, exp, is_call, strike_raw))
    }

    /// Create an option contract from already-validated wire-format integers.
    ///
    /// Used by the in-process WS server which parses the wire format
    /// directly. Infallible: every input range is enforced at the type
    /// level.
    ///
    /// # Arguments
    /// - `symbol`: Underlying ticker
    /// - `expiration`: `YYYYMMDD` integer
    /// - `is_call`: `true` for call, `false` for put
    /// - `strike_raw`: Strike in thousandths of a dollar (i32)
    pub fn option_raw(symbol: &str, expiration: i32, is_call: bool, strike_raw: i32) -> Self {
        Self {
            symbol: Arc::from(symbol),
            sec_type: SecType::Option,
            expiration: Some(expiration),
            is_call: Some(is_call),
            strike_thousandths: Some(strike_raw),
        }
    }

    /// Strike price in dollars as `f64`. Derived from the wire-level
    /// [`Self::strike_thousandths`] fixed-point integer (a `$5,400.00`
    /// option carries `strike_thousandths == Some(5_400_000)`); this
    /// accessor divides by `1000.0` so user code reads the dollar
    /// notation it writes when calling
    /// `Contract::option("SPX", OptionLeg { strike: "5400.00", .. })`.
    /// Returns `None` for non-option contracts.
    #[must_use]
    pub fn strike_dollars(&self) -> Option<f64> {
        self.strike_thousandths.map(|s| f64::from(s) / 1000.0)
    }

    /// Option side as the typed [`Right`] enum: `Some(Right::Call)` /
    /// `Some(Right::Put)` for options, `None` for non-option contracts.
    /// Derived from the wire-level `is_call` flag. This is the
    /// user-facing accessor; non-Rust bindings surface it as the
    /// language-idiomatic shape: string `"C"` / `"P"` in Python and
    /// TypeScript, `char` in C / C++. Event-carried contracts never
    /// return [`Right::Both`].
    #[must_use]
    pub fn right(&self) -> Option<Right> {
        self.is_call
            .map(|c| if c { Right::Call } else { Right::Put })
    }

    /// Build an unresolved-contract sentinel for a given wire contract id.
    ///
    /// The sentinel is emitted for tick events that arrive before the
    /// matching `ContractAssigned` control frame has been processed.
    /// `sec_type` is [`SecType::Unknown`] so downstream code can detect
    /// the sentinel via the enum variant rather than a symbol-prefix match.
    /// The `symbol` carries the decimal wire id under the `__pending:`
    /// prefix for diagnostic correlation.
    #[must_use]
    pub fn pending(contract_id: i32) -> Self {
        use crate::tdbe::types::enums::SecType;
        Self {
            symbol: Arc::from(
                format!(
                    "{}{contract_id}",
                    crate::fpss::UNRESOLVED_CONTRACT_SYMBOL_PREFIX
                )
                .as_str(),
            ),
            sec_type: SecType::Unknown,
            expiration: None,
            is_call: None,
            strike_thousandths: None,
        }
    }

    /// Synthetic marker used by `reconnect_streaming` to represent a failed
    /// full-type subscription inside [`crate::Error::PartialReconnect::failed`].
    ///
    /// Full-type subscriptions are not addressed by a real `Contract`, but the
    /// failure list keeps a homogeneous `(SubscriptionKind, Contract)` shape
    /// so callers can iterate the list with one match arm. `symbol` is empty
    /// and option fields are `None`, which mirrors the lack of per-contract
    /// addressability for a full-type subscription. Operators see the
    /// original `SecType` via the per-failure `tracing::warn!` line emitted
    /// at the call site.
    #[must_use]
    pub fn full_type_marker(sec_type: SecType) -> Self {
        Self {
            symbol: Arc::from(""),
            sec_type,
            expiration: None,
            is_call: None,
            strike_thousandths: None,
        }
    }

    /// OCC-21 option identifier length (6 root + 6 YYMMDD + 1 side + 8 strike).
    const OCC21_LEN: usize = 21;

    /// Maximum root length the wire codec accepts
    /// (`Contract::to_bytes` / `Contract::from_bytes`). The parser
    /// matches this upper bound so `from_str` / `to_bytes` / `from_bytes`
    /// round-trip symmetrically -- the wire format is the ground truth
    /// for what the terminal can see. The 16-byte ceiling admits upstream
    /// sources that ship longer roots (14+ char instrument identifiers
    /// observed on some non-equity feeds) without requiring parser-side
    /// special-casing.
    pub(crate) const MAX_ROOT_LEN: usize = 16;
    /// Validate that a candidate root ticker is 1..=`MAX_ROOT_LEN` ASCII
    /// uppercase letters optionally containing a single `.` (e.g.
    /// `"BRK.B"`). Multiple dots are rejected -- industry-practice
    /// single-dot compound tickers like `"BRK.A"` / `"BRK.B"` /
    /// `"RDS.A"` never carry more than one dot, and allowing `"A..B"`
    /// would let callers sneak past the shape check with unconventional
    /// symbols that fail downstream exchange lookups.
    ///
    /// The length ceiling matches `Contract::to_bytes`, which accepts
    /// roots up to 16 bytes, so `from_str` / `to_bytes` / `from_bytes`
    /// round-trip symmetrically. Returns an `Error::Config` with the
    /// offending input on failure.
    fn validate_root(input: &str, root: &str) -> Result<(), Error> {
        if root.is_empty() || root.len() > Self::MAX_ROOT_LEN {
            return Err(Error::config_invalid(
                "contract.symbol",
                format!(
                    "Contract::from_str: root must be 1..={} chars, got {} chars in {input:?}",
                    Self::MAX_ROOT_LEN,
                    root.len()
                ),
            ));
        }
        let mut dot_count = 0usize;
        for ch in root.chars() {
            if ch == '.' {
                dot_count += 1;
                if dot_count > 1 {
                    return Err(Error::config_invalid(
                        "contract.symbol",
                        format!(
                            "Contract::from_str: root must contain at most one '.', got {root:?} in {input:?}"
                        ),
                    ));
                }
            } else if !ch.is_ascii_uppercase() {
                return Err(Error::config_invalid(
                    "contract.symbol",
                    format!(
                        "Contract::from_str: root must be ASCII A-Z (or '.'), got {ch:?} in {input:?}"
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Parse an OCC-21 option identifier (`"AAPL  260417C00550000"`).
    ///
    /// Layout (ASCII, exactly 21 bytes):
    /// - bytes `0..6`: root, right-padded with spaces.
    /// - bytes `6..12`: `YYMMDD` expiration; the returned `exp_date` is
    ///   `20000000 + YYMMDD` (e.g. `260417` -> `20260417`).
    /// - byte `12`: `'C'` or `'P'`.
    /// - bytes `13..21`: zero-padded strike in thousandths of a dollar.
    ///
    /// # Scope
    ///
    /// This parser is scoped to OCC symbols in the 2000-2099 range. Any
    /// two-digit YY maps to `2000 + YY`, so `"AAPL  990101C00100000"`
    /// parses to `exp_date = 20990101` (2099-01-01). The OCC roster
    /// re-uses YY codes every century; callers transitioning past 2099
    /// must introduce an explicit century argument. The live FPSS feed
    /// will never surface a pre-2000 OCC symbol (the market-data channel
    /// only ships live contracts), so no roll-over heuristic is
    /// encoded.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] naming the specific failure (wrong length,
    /// empty root, non-uppercase root character, bad YYMMDD digits, right
    /// outside `C`/`P`, bad strike digits) with the full offending input
    /// included.
    fn parse_occ21(input: &str) -> Result<Self, Error> {
        if input.len() != Self::OCC21_LEN {
            return Err(Error::config_invalid(
                "contract.occ21",
                format!(
                    "Contract::from_str: OCC-21 format must be exactly {} chars, got {} in {input:?}",
                    Self::OCC21_LEN,
                    input.len()
                ),
            ));
        }
        if !input.is_ascii() {
            return Err(Error::config_invalid(
                "contract.occ21",
                format!("Contract::from_str: OCC-21 identifier must be ASCII, got {input:?}"),
            ));
        }
        let bytes = input.as_bytes();
        // Root: bytes 0..6, right-padded with spaces.
        let root_raw = &input[0..6];
        let root: &str = root_raw.trim_end_matches(' ');
        Self::validate_root(input, root)?;

        // YYMMDD -> integer, then centuried to YYYYMMDD.
        let yymmdd_raw = &input[6..12];
        let yymmdd: i32 = yymmdd_raw.parse().map_err(|e| {
            Error::config_invalid(
                "contract.expiration",
                format!(
                    "Contract::from_str: expiration YYMMDD not numeric ({yymmdd_raw:?}, {e}) in {input:?}"
                ),
            )
        })?;
        if !(0..1_000_000).contains(&yymmdd) {
            return Err(Error::config_invalid(
                "contract.expiration",
                format!(
                    "Contract::from_str: expiration YYMMDD out of range ({yymmdd}) in {input:?}"
                ),
            ));
        }
        // Scope: 2000-2099. See the doc-comment `# Scope` section.
        // Any two-digit YY is interpreted as `2000 + YY`, so `99`
        // maps to 2099 rather than 1999. The FPSS live feed only
        // ships contracts with future expirations, so pre-2000 OCC
        // symbols cannot reach this parser over the wire.
        let expiration: i32 = 20_000_000 + yymmdd;
        // Reject impossible OCC-21 expirations (e.g. `260230`
        // (Feb 30) or `260431` (Apr 31)) using the same canonical
        // Gregorian validator as MDDS + `Contract::option`, so a
        // shape-valid-but-impossible date never decodes to a contract.
        if !crate::tdbe::time::is_valid_yyyymmdd(expiration) {
            return Err(Error::config_invalid(
                "contract.expiration",
                format!(
                    "Contract::from_str: OCC-21 expiration ({yymmdd_raw}) is not a valid Gregorian date in {input:?}"
                ),
            ));
        }

        // Right byte.
        let right_byte = bytes[12];
        let is_call = match right_byte {
            b'C' => true,
            b'P' => false,
            other => {
                return Err(Error::config_invalid(
                    "contract.right",
                    format!(
                        "Contract::from_str: expected 'C' or 'P' at position 12, got {:?} in {input:?}",
                        other as char
                    ),
                ));
            }
        };

        // Strike: thousandths of a dollar, zero-padded integer of 8 digits.
        let strike_raw = &input[13..21];
        for ch in strike_raw.chars() {
            if !ch.is_ascii_digit() {
                return Err(Error::config_invalid(
                    "contract.strike_thousandths",
                    format!(
                        "Contract::from_str: strike must be 8 ASCII digits, got {strike_raw:?} in {input:?}"
                    ),
                ));
            }
        }
        let strike: i32 = strike_raw.parse().map_err(|e| {
            Error::config_invalid(
                "contract.strike_thousandths",
                format!(
                    "Contract::from_str: strike not numeric ({strike_raw:?}, {e}) in {input:?}"
                ),
            )
        })?;

        Ok(Self {
            symbol: Arc::from(root),
            sec_type: SecType::Option,
            expiration: Some(expiration),
            is_call: Some(is_call),
            strike_thousandths: Some(strike),
        })
    }

    /// Serialize to the wire format used in FPSS subscription messages.
    ///
    /// # Wire format
    ///
    /// Stock/Index/Rate:
    /// ```text
    /// [total_size: u8] [root_len: u8] [root ASCII bytes] [sec_type: u8]
    /// ```
    ///
    /// Option:
    /// ```text
    /// [total_size: u8] [root_len: u8] [root ASCII bytes] [sec_type: u8]
    /// [exp_date: i32 BE] [is_call: u8] [strike: i32 BE]
    /// ```
    ///
    /// `total_size` counts the entire buffer including itself:
    ///   - Stock: `3 + root.length()` = size(1) + `root_len(1)` + root(N) + `sec_type(1)`
    ///   - Option: `12 + root.length()` = size(1) + `root_len(1)` + root(N) + `sec_type(1)` + exp(4) + `is_call(1)` + strike(4)
    ///
    /// The decoder validates `len == size`, confirming the size byte counts
    /// itself.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] if the contract root exceeds 16 bytes,
    /// which is the maximum encoded root length the wire format accepts.
    /// All FPSS subscribe / unsubscribe paths surface this error to the
    /// caller; user input is never permitted to panic the encoder.
    pub fn try_to_bytes(&self) -> Result<Vec<u8>, Error> {
        self.validate()?;
        Ok(self.encode_unchecked())
    }

    /// Encode the contract on the wire, asserting the root length invariant.
    ///
    /// Use [`Contract::try_to_bytes`] for paths that accept user input. This
    /// helper exists so the encoder can be inlined into hot loops once the
    /// caller has already validated the contract.
    ///
    /// # Panics
    ///
    /// Debug builds panic via `debug_assert!` if [`Contract::validate`]
    /// fails. Release builds emit a wire-malformed encoding without
    /// panicking unless the root exceeds 255 bytes, at which point
    /// `encode_unchecked` panics on the `u8::try_from(root_bytes.len())`
    /// conversion. Callers must call [`Contract::validate`] first or
    /// originate the contract from a validated source.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        debug_assert!(
            self.validate().is_ok(),
            "Contract::to_bytes called with invalid root; use try_to_bytes for caller-supplied input"
        );
        self.encode_unchecked()
    }

    /// Validate that the contract can be encoded on the FPSS wire.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] if the root is empty or longer than 16
    /// bytes. The 16-byte limit is the wire format's maximum encoded root.
    pub fn validate(&self) -> Result<(), Error> {
        let len = self.symbol.len();
        if len == 0 {
            return Err(Error::config_missing("contract.symbol"));
        }
        if len > 16 {
            return Err(Error::config_invalid(
                "contract.symbol",
                format!(
                    "contract symbol too long: {len} bytes (wire format caps the encoded root at 16 bytes)"
                ),
            ));
        }
        Ok(())
    }

    fn encode_unchecked(&self) -> Vec<u8> {
        // Local names mirror the wire spec: the wire field is "root_len /
        // root", and keeping the byte-level codec named that way keeps this
        // file diffing cleanly against the binary protocol layout. The
        // struct binding is `symbol`.
        let root_bytes = self.symbol.as_bytes();
        let root_len = u8::try_from(root_bytes.len()).expect("validate() bounds root_len to <= 16");

        let is_option = self.sec_type == SecType::Option;

        // `3 + root.length()` for non-option, `12 + root.length()` for option.
        // The size byte counts itself: size(1) + root_len(1) + root(N) + sec_type(1) [+ option fields(9)]
        let total_size = if is_option {
            12 + root_bytes.len()
        } else {
            3 + root_bytes.len()
        };

        let mut buf = Vec::with_capacity(total_size);

        // total_size byte (includes itself per the wire format).
        // Max total_size = 12 + 16 = 28, safely fits u8.
        buf.push(u8::try_from(total_size).expect("total_size <= 28"));
        // root_len
        buf.push(root_len);
        // root ASCII
        buf.extend_from_slice(root_bytes);
        // sec_type
        buf.push(self.sec_type as u8);

        if is_option {
            // exp_date: i32 big-endian
            buf.extend_from_slice(&self.expiration.unwrap_or(0).to_be_bytes());
            // is_call: u8 (1 = call, 0 = put)
            buf.push(u8::from(self.is_call.unwrap_or(false)));
            // strike: i32 big-endian
            buf.extend_from_slice(&self.strike_thousandths.unwrap_or(0).to_be_bytes());
        }

        buf
    }

    /// Deserialize from the wire format.
    ///
    /// Input starts at the `total_size` byte (the first byte of the encoded
    /// contract).
    ///
    /// # Errors
    ///
    /// Returns an error on network, authentication, or parsing failure.
    pub fn from_bytes(data: &[u8]) -> Result<(Self, usize), ContractParseError> {
        if data.is_empty() {
            return Err(ContractParseError::TooShort);
        }

        // The size byte counts itself: the total buffer length equals the
        // size byte value. The decoder rejects `len != size`, where `len` is
        // the total span including the size byte.
        let total_size = data[0] as usize;
        if data.len() < total_size {
            return Err(ContractParseError::TooShort);
        }

        // Minimum: size(1) + root_len(1) + root(>=0) + sec_type(1) = 3
        if total_size < 3 {
            return Err(ContractParseError::InvalidSize(total_size));
        }

        let root_len = data[1] as usize;
        // Validate: size(1) + root_len(1) + root(N) + sec_type(1) <= total_size
        if total_size < 2 + root_len + 1 {
            return Err(ContractParseError::InvalidSize(total_size));
        }

        let root_start = 2;
        let root_end = root_start + root_len;
        let root: Arc<str> = Arc::from(
            std::str::from_utf8(&data[root_start..root_end])
                .map_err(|_| ContractParseError::InvalidUtf8)?,
        );

        let sec_type_byte = data[root_end];
        let sec_type = SecType::from_code(i32::from(sec_type_byte))
            .ok_or(ContractParseError::UnknownSecType(sec_type_byte))?;

        if sec_type == SecType::Option {
            // Need 9 more bytes after sec_type: exp_date(4) + is_call(1) + strike(4)
            let opt_start = root_end + 1;
            if data.len() < opt_start + 9 {
                return Err(ContractParseError::TooShort);
            }

            let exp_date = i32::from_be_bytes([
                data[opt_start],
                data[opt_start + 1],
                data[opt_start + 2],
                data[opt_start + 3],
            ]);
            let is_call = data[opt_start + 4] != 0;
            let strike = i32::from_be_bytes([
                data[opt_start + 5],
                data[opt_start + 6],
                data[opt_start + 7],
                data[opt_start + 8],
            ]);

            Ok((
                Contract {
                    symbol: root,
                    sec_type,
                    expiration: Some(exp_date),
                    is_call: Some(is_call),
                    strike_thousandths: Some(strike),
                },
                total_size,
            ))
        } else {
            Ok((
                Contract {
                    symbol: root,
                    sec_type,
                    expiration: None,
                    is_call: None,
                    strike_thousandths: None,
                },
                total_size,
            ))
        }
    }
}

impl std::fmt::Display for Contract {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.sec_type {
            SecType::Option => {
                let right = if self.is_call.unwrap_or(false) {
                    "C"
                } else {
                    "P"
                };
                // Render the strike in dollars, not the wire-level
                // `strike_thousandths` fixed-point integer, so every binding
                // (Rust / Python / TypeScript / C++) prints the same
                // human-readable identity. `f64` Display drops a trailing
                // `.0` for whole-dollar strikes (`550`) and keeps the needed
                // decimals for fractional ones (`552.5`).
                let strike = self.strike_dollars().unwrap_or(0.0);
                write!(
                    f,
                    "{} {} {} {} {}",
                    self.symbol,
                    self.sec_type.as_str(),
                    self.expiration.unwrap_or(0),
                    right,
                    strike,
                )
            }
            _ => write!(f, "{} {}", self.symbol, self.sec_type.as_str()),
        }
    }
}

impl std::str::FromStr for Contract {
    type Err = Error;

    /// Parse a [`Contract`] from a string.
    ///
    /// Two formats are accepted:
    ///
    /// 1. **Bare root** (stock contract). 1..=6 ASCII uppercase letters,
    ///    optionally including a single `.`:
    ///    ```
    ///    # use std::str::FromStr;
    ///    # use thetadatadx::fpss::protocol::Contract;
    ///    let c = "AAPL".parse::<Contract>().unwrap();
    ///    assert_eq!(&*c.symbol, "AAPL");
    ///    ```
    /// 2. **OCC-21 option identifier**. 21 ASCII characters:
    ///    `[root (6, space-padded)] [YYMMDD (6)] [C|P (1)] [strike (8, 1/1000$)]`.
    ///    Both 21-char (canonical) and 20-char (one space inside the
    ///    root-padding region) inputs are accepted. The 20-char form is
    ///    repaired by peeling off the fixed 15-char `YYMMDD+right+strike`
    ///    suffix and right-padding the leading root slice to 6 chars —
    ///    byte-for-byte equivalent to the 21-char form for the same
    ///    contract, matching the tolerant upstream feed that occasionally
    ///    ships the 20-char variant.
    ///    ```
    ///    # use std::str::FromStr;
    ///    # use thetadatadx::fpss::protocol::Contract;
    ///    let c = "SPY   260417C00550000".parse::<Contract>().unwrap();
    ///    assert_eq!(&*c.symbol, "SPY");
    ///    assert_eq!(c.expiration, Some(20_260_417));
    ///    assert_eq!(c.is_call, Some(true));
    ///    assert_eq!(c.strike_thousandths, Some(550_000));
    ///    ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] whose message names the specific failure
    /// (wrong length, non-ASCII, bad YYMMDD digits, right outside `C`/`P`,
    /// bad strike digits, empty root, non-uppercase root character) and
    /// includes the original input.
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        // Strategy: the two formats are unambiguous by length after trim.
        // `"AAPL"` trimmed is 4 chars; `"SPY   260417C00550000"` trimmed
        // is 21 chars. A middle-length string is garbage — we still
        // route through the bare-root validator so the error message
        // names the exact constraint that failed.
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(Error::config_invalid(
                "contract.input",
                format!("Contract::from_str: empty or whitespace-only input in {input:?}"),
            ));
        }
        if trimmed.len() == Self::OCC21_LEN {
            return Self::parse_occ21(trimmed);
        }
        // 20-char tolerance: some upstream sources drop a single space
        // INSIDE the root-padding region (e.g. `"SPY 260417C00550000"`
        // has "SPY" + one pad space + 15-char suffix = 20 chars). The
        // OCC-21 layout is [6-char root][15-char YYMMDD+right+strike],
        // so repair by peeling the trailing 15 chars as the fixed
        // suffix and right-padding the leading root slice to 6 chars.
        //
        // This is strictly a repair heuristic. The strict 21-char
        // parser runs on the repaired string, so all other constraints
        // (digits, right byte, strike width) still surface their exact
        // errors with the repaired input in the message.
        const OCC21_SUFFIX_LEN: usize = 15; // YYMMDD(6) + right(1) + strike(8)
        if trimmed.len() == Self::OCC21_LEN - 1 && trimmed.len() > OCC21_SUFFIX_LEN {
            let split = trimmed.len() - OCC21_SUFFIX_LEN;
            let root_slice = trimmed[..split].trim_end_matches(' ');
            let suffix = &trimmed[split..];
            if !root_slice.is_empty() && root_slice.len() <= 6 {
                // Rebuild with the root re-padded to 6 chars, then the
                // 15-char suffix — byte-for-byte equivalent to a
                // correctly-formatted OCC-21 string for the same
                // contract.
                let mut repaired = String::with_capacity(Self::OCC21_LEN);
                repaired.push_str(root_slice);
                for _ in root_slice.len()..6 {
                    repaired.push(' ');
                }
                repaired.push_str(suffix);
                if repaired.len() == Self::OCC21_LEN {
                    return Self::parse_occ21(&repaired);
                }
            }
        }

        // Bare root fallback.
        Self::validate_root(input, trimmed)?;
        Ok(Self::stock(trimmed))
    }
}

/// Errors that can occur when parsing a contract from bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContractParseError {
    /// The buffer ended before a full contract record could be read.
    TooShort,
    /// The leading `total_size` byte is below the structural minimum or
    /// inconsistent with the declared root length.
    InvalidSize(usize),
    /// The root field bytes are not valid UTF-8.
    InvalidUtf8,
    /// The security-type byte does not map to a known [`SecType`].
    UnknownSecType(u8),
}

impl std::fmt::Display for ContractParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort => write!(f, "contract data too short"),
            Self::InvalidSize(s) => write!(f, "invalid contract total_size: {s}"),
            Self::InvalidUtf8 => write!(f, "contract root is not valid UTF-8"),
            Self::UnknownSecType(c) => write!(f, "unknown sec_type code: {c}"),
        }
    }
}

impl std::error::Error for ContractParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_contract_roundtrip() {
        let c = Contract::stock("AAPL");
        let bytes = c.to_bytes();
        // 3 + root.length() = 3 + 4 = 7 total bytes, size byte = 7.
        assert_eq!(bytes.len(), 7);
        assert_eq!(bytes[0], 7); // total_size includes itself (3 + root.length()).

        let (parsed, consumed) = Contract::from_bytes(&bytes).unwrap();
        assert_eq!(consumed, 7);
        assert_eq!(parsed, c);
    }

    #[test]
    fn option_contract_roundtrip() {
        let c = Contract::option(
            "SPY",
            OptionLeg {
                expiration: "20261218",
                strike: "60",
                right: "C",
            },
        )
        .unwrap();
        let bytes = c.to_bytes();
        // 12 + root.length() = 12 + 3 = 15 total bytes, size byte = 15.
        assert_eq!(bytes.len(), 15);
        assert_eq!(bytes[0], 15); // total_size includes itself (12 + root.length()).

        let (parsed, consumed) = Contract::from_bytes(&bytes).unwrap();
        assert_eq!(consumed, 15);
        assert_eq!(parsed, c);
        assert_eq!(parsed.expiration, Some(20261218));
        assert_eq!(parsed.is_call, Some(true));
        assert_eq!(parsed.strike_thousandths, Some(60000));
    }

    #[test]
    fn index_contract_roundtrip() {
        let c = Contract::index("SPX");
        let bytes = c.to_bytes();
        let (parsed, _) = Contract::from_bytes(&bytes).unwrap();
        assert_eq!(&*parsed.symbol, "SPX");
        assert_eq!(parsed.sec_type, SecType::Index);
    }

    #[test]
    fn contract_from_bytes_too_short() {
        let err = Contract::from_bytes(&[]).unwrap_err();
        assert_eq!(err, ContractParseError::TooShort);
    }

    #[test]
    fn contract_from_bytes_invalid_size() {
        // total_size = 2, but minimum valid is 3 (size + root_len + sec_type with root_len=0)
        let err = Contract::from_bytes(&[2, 0]).unwrap_err();
        assert_eq!(err, ContractParseError::InvalidSize(2));
    }

    #[test]
    fn contract_display_stock() {
        assert_eq!(Contract::stock("AAPL").to_string(), "AAPL STOCK");
    }

    #[test]
    fn contract_display_option() {
        let c = Contract::option(
            "SPY",
            OptionLeg {
                expiration: "20261218",
                strike: "45",
                right: "P",
            },
        )
        .unwrap();
        assert_eq!(c.to_string(), "SPY OPTION 20261218 P 45");
    }

    #[test]
    fn contract_display_option_renders_strike_in_dollars() {
        // The rendered strike is dollars, not the wire-level
        // `strike_thousandths` integer, so the Rust `Display` matches the
        // C++ `operator<<` and the Python / TypeScript string surface.
        let whole = Contract::option(
            "SPY",
            OptionLeg {
                expiration: "20260620",
                strike: "550",
                right: "C",
            },
        )
        .unwrap();
        assert_eq!(whole.to_string(), "SPY OPTION 20260620 C 550");

        // Fractional strikes keep the needed decimals.
        let fractional = Contract::option(
            "SPY",
            OptionLeg {
                expiration: "20260620",
                strike: "552.5",
                right: "P",
            },
        )
        .unwrap();
        assert_eq!(fractional.to_string(), "SPY OPTION 20260620 P 552.5");
    }

    // -- Wire-format parity tests ----------------------------------------------
    // Byte-for-byte compatibility with the documented Contract serialization.

    #[test]
    fn wire_parity_stock_aapl() {
        // root="AAPL" (4 bytes), sec=STOCK; allocates 3 + 4 = 7 bytes.
        // Wire: [7, 4, 'A', 'A', 'P', 'L', sec_type_code]
        let c = Contract::stock("AAPL");
        let bytes = c.to_bytes();
        assert_eq!(bytes[0], 7); // size byte = 3 + root.length()
        assert_eq!(bytes[1], 4); // root_len
        assert_eq!(&bytes[2..6], b"AAPL");
        assert_eq!(bytes[6], SecType::Stock as u8);
        assert_eq!(bytes.len(), 7);
    }

    #[test]
    fn wire_parity_option_spy() {
        // root="SPY" (3 bytes), sec=OPTION, exp=20261218, isCall=true, strike=60000.
        // Allocates 12 + 3 = 15 bytes.
        // Wire: [15, 3, 'S','P','Y', sec_type, exp(4), is_call(1), strike(4)]
        let c = Contract::option(
            "SPY",
            OptionLeg {
                expiration: "20261218",
                strike: "60",
                right: "C",
            },
        )
        .unwrap();
        let bytes = c.to_bytes();
        assert_eq!(bytes[0], 15); // size byte = 12 + root.length()
        assert_eq!(bytes[1], 3); // root_len
        assert_eq!(&bytes[2..5], b"SPY");
        assert_eq!(bytes[5], SecType::Option as u8);
        // exp_date = 20261218 big-endian
        assert_eq!(&bytes[6..10], &20261218i32.to_be_bytes());
        assert_eq!(bytes[10], 1); // is_call = true
                                  // strike = 60000 big-endian
        assert_eq!(&bytes[11..15], &60000i32.to_be_bytes());
    }

    #[test]
    fn wire_parity_index_spx() {
        // root="SPX" (3 bytes), sec=INDEX; allocates 3 + 3 = 6 bytes.
        let c = Contract::index("SPX");
        let bytes = c.to_bytes();
        assert_eq!(bytes[0], 6);
        assert_eq!(bytes[1], 3);
        assert_eq!(&bytes[2..5], b"SPX");
        assert_eq!(bytes[5], SecType::Index as u8);
        assert_eq!(bytes.len(), 6);
    }

    #[test]
    fn option_rejects_invalid_strike() {
        // Garbage strike string -- must return Err, not panic.
        assert!(Contract::option(
            "SPY",
            OptionLeg {
                expiration: "20261218",
                strike: "not-a-number",
                right: "C"
            }
        )
        .is_err());
    }

    #[test]
    fn option_rejects_overflowing_strike() {
        // Strike * 1000 exceeds i32::MAX. Must return Err, not wrap silently.
        assert!(Contract::option(
            "SPY",
            OptionLeg {
                expiration: "20261218",
                strike: "3000000",
                right: "C"
            }
        )
        .is_err());
    }

    #[test]
    fn option_rejects_invalid_expiration() {
        // Non-numeric expiration -- must return Err, not panic.
        assert!(Contract::option(
            "SPY",
            OptionLeg {
                expiration: "not-a-date",
                strike: "60",
                right: "C"
            }
        )
        .is_err());
    }

    #[test]
    fn option_rejects_impossible_calendar_dates() {
        // Both `Contract::option` and OCC-21 parsing defer to the
        // canonical Gregorian validator, so shape-valid-but-impossible
        // dates are rejected.
        for bad in ["00000000", "20260230", "19990431", "21010101", "18991231"] {
            assert!(
                Contract::option(
                    "SPY",
                    OptionLeg {
                        expiration: bad,
                        strike: "60",
                        right: "C"
                    }
                )
                .is_err(),
                "expected impossible expiration {bad} to be rejected",
            );
        }
    }

    #[test]
    fn from_str_occ21_rejects_impossible_calendar_dates() {
        use std::str::FromStr;
        // OCC-21 with Feb 30 / Apr 31 — the encoded YYYYMMDD is invalid.
        for bad in [
            "SPY   260230C00550000", // Feb 30 2026
            "SPY   260431P00550000", // Apr 31 2026
        ] {
            let err = Contract::from_str(bad).unwrap_err().to_string();
            assert!(
                err.contains("not a valid Gregorian date"),
                "OCC-21 must reject impossible date in {bad}, got: {err}"
            );
        }
    }

    #[test]
    fn wire_parity_single_char_root() {
        // Edge case: root="A" (1 byte), sec=STOCK
        // Wire size: 3 + 1 = 4 bytes
        let c = Contract::stock("A");
        let bytes = c.to_bytes();
        assert_eq!(bytes[0], 4);
        assert_eq!(bytes[1], 1);
        assert_eq!(bytes[2], b'A');
        assert_eq!(bytes.len(), 4);
    }

    // -- FromStr tests ---------------------------------------------------------

    #[test]
    fn from_str_bare_root_stock() {
        use std::str::FromStr;
        let c = Contract::from_str("AAPL").unwrap();
        assert_eq!(&*c.symbol, "AAPL");
        assert_eq!(c.sec_type, SecType::Stock);
        assert!(c.expiration.is_none());
        assert!(c.is_call.is_none());
        assert!(c.strike_thousandths.is_none());
    }

    #[test]
    fn from_str_bare_root_short_ticker() {
        use std::str::FromStr;
        let c = Contract::from_str("A").unwrap();
        assert_eq!(&*c.symbol, "A");
        assert_eq!(c.sec_type, SecType::Stock);
    }

    #[test]
    fn from_str_bare_root_with_dot() {
        use std::str::FromStr;
        // BRK.A style tickers must parse as stock roots.
        let c = Contract::from_str("BRK.A").unwrap();
        assert_eq!(&*c.symbol, "BRK.A");
        assert_eq!(c.sec_type, SecType::Stock);
    }

    #[test]
    fn from_str_bare_root_trims_surrounding_whitespace() {
        use std::str::FromStr;
        let c = Contract::from_str("  SPY  ").unwrap();
        assert_eq!(&*c.symbol, "SPY");
    }

    #[test]
    fn from_str_occ21_call() {
        use std::str::FromStr;
        // SPY  (4 chars -> 6 chars padded) 26-04-17 Call 550.00.
        let c = Contract::from_str("SPY   260417C00550000").unwrap();
        assert_eq!(&*c.symbol, "SPY");
        assert_eq!(c.sec_type, SecType::Option);
        assert_eq!(c.expiration, Some(20_260_417));
        assert_eq!(c.is_call, Some(true));
        assert_eq!(c.strike_thousandths, Some(550_000));
    }

    #[test]
    fn from_str_occ21_put() {
        use std::str::FromStr;
        // QQQ 26-06-20 Put 350.00.
        let c = Contract::from_str("QQQ   260620P00350000").unwrap();
        assert_eq!(&*c.symbol, "QQQ");
        assert_eq!(c.is_call, Some(false));
        assert_eq!(c.expiration, Some(20_260_620));
        assert_eq!(c.strike_thousandths, Some(350_000));
    }

    #[test]
    fn from_str_occ21_aapl_documented_example() {
        use std::str::FromStr;
        // The exact example from the spec.
        let c = Contract::from_str("AAPL  260417C00550000").unwrap();
        assert_eq!(&*c.symbol, "AAPL");
        assert_eq!(c.expiration, Some(20_260_417));
        assert_eq!(c.is_call, Some(true));
        assert_eq!(c.strike_thousandths, Some(550_000));
    }

    #[test]
    fn from_str_occ21_six_char_root() {
        use std::str::FromStr;
        // Full six-char root: no spaces in the root field.
        let c = Contract::from_str("ABCDEF260417C00550000").unwrap();
        assert_eq!(&*c.symbol, "ABCDEF");
        assert_eq!(c.expiration, Some(20_260_417));
        assert_eq!(c.strike_thousandths, Some(550_000));
    }

    #[test]
    fn from_str_occ21_malformed_strike() {
        use std::str::FromStr;
        // Strike contains non-digit characters.
        let err = Contract::from_str("SPY   260417C0055000X")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("strike"),
            "expected error to name strike failure, got: {err}"
        );
        assert!(
            err.contains("SPY   260417C0055000X"),
            "expected error to include the offending input, got: {err}"
        );
    }

    #[test]
    fn from_str_occ21_wrong_right_byte() {
        use std::str::FromStr;
        // Byte 12 is 'X', not 'C' or 'P'.
        let err = Contract::from_str("SPY   260417X00550000")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("'C' or 'P'"),
            "expected error to name the right constraint, got: {err}"
        );
    }

    #[test]
    fn from_str_occ21_bad_expiration_digits() {
        use std::str::FromStr;
        let err = Contract::from_str("SPY   2X0417C00550000")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("expiration"),
            "expected error to name expiration failure, got: {err}"
        );
    }

    #[test]
    fn from_str_wrong_length_garbage() {
        use std::str::FromStr;
        // 18 chars -- outside both the widened 1..=16 bare-root range
        // and the 21-char OCC-21 range. Must surface a specific
        // Contract::from_str error (not a generic pass-through) that
        // includes the offending input.
        let err = Contract::from_str("APPLETREESARECUTEST")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("Contract::from_str"),
            "expected Contract::from_str-prefixed error, got: {err}"
        );
        assert!(
            err.contains("APPLETREESARECUTEST"),
            "expected error to include the offending input, got: {err}"
        );
    }

    #[test]
    fn from_str_empty_input() {
        use std::str::FromStr;
        let err = Contract::from_str("").unwrap_err().to_string();
        assert!(
            err.contains("empty"),
            "expected error to mention empty input, got: {err}"
        );
    }

    #[test]
    fn from_str_lowercase_root_rejected() {
        use std::str::FromStr;
        let err = Contract::from_str("aapl").unwrap_err().to_string();
        assert!(
            err.contains("ASCII A-Z"),
            "expected error to describe root charset, got: {err}"
        );
    }

    #[test]
    fn from_str_root_too_long() {
        use std::str::FromStr;
        // 17-char non-space root: exceeds the MAX_ROOT_LEN (16) cap
        // that matches `Contract::to_bytes()` / `Contract::from_bytes()`.
        let err = Contract::from_str("ABCDEFGHIJKLMNOPQ")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("Contract::from_str"),
            "expected Contract::from_str-prefixed error, got: {err}"
        );
        assert!(
            err.contains("1..=16"),
            "expected error to name the widened length bound, got: {err}"
        );
    }

    // -- Contract::from_str wire-codec parity ---------------------------------
    //
    // `to_bytes()` accepts roots up to 16 bytes (the wire format's
    // encoded-root cap); `from_bytes()` round-trips
    // whatever the wire delivers. The bare-root parser must accept the
    // same 1..=16 range so `from_str` / `to_bytes` / `from_bytes`
    // round-trip symmetrically.

    #[test]
    fn from_str_accepts_seven_char_root() {
        use std::str::FromStr;
        let c = Contract::from_str("ABCDEFG").expect("7-char root must parse");
        assert_eq!(&*c.symbol, "ABCDEFG");
        assert_eq!(c.sec_type, SecType::Stock);
    }

    #[test]
    fn from_str_accepts_root_at_max_length() {
        use std::str::FromStr;
        // 16 A's is the exact MAX_ROOT_LEN boundary.
        let sixteen = "AAAAAAAAAAAAAAAA";
        assert_eq!(sixteen.len(), 16);
        let c = Contract::from_str(sixteen).expect("16-char root must parse");
        assert_eq!(&*c.symbol, sixteen);
        assert_eq!(c.sec_type, SecType::Stock);
    }

    #[test]
    fn from_str_round_trip_wire_codec_for_widened_roots() {
        use std::str::FromStr;
        // Every root length 7..=16 must parse AND round-trip through
        // the wire codec byte-for-byte. A regression that
        // re-narrows the parser but leaves `to_bytes` unchanged would
        // fail this loop on the first call.
        for n in 7usize..=16 {
            let root: String = "A".repeat(n);
            let parsed = Contract::from_str(&root)
                .unwrap_or_else(|_| panic!("from_str must accept {n}-char root"));
            assert_eq!(&*parsed.symbol, root.as_str());
            let wire = parsed.to_bytes();
            let (decoded, consumed) = Contract::from_bytes(&wire)
                .unwrap_or_else(|_| panic!("from_bytes must decode {n}-char root"));
            assert_eq!(decoded, parsed, "round-trip must preserve the contract");
            assert_eq!(consumed, wire.len(), "consumed must equal wire length");
        }
    }

    #[test]
    fn from_str_rejects_root_over_max_length() {
        use std::str::FromStr;
        // 17 chars exceeds the MAX_ROOT_LEN cap.
        let seventeen = "ABCDEFGHIJKLMNOPQ";
        assert_eq!(seventeen.len(), 17);
        let err = Contract::from_str(seventeen).unwrap_err().to_string();
        assert!(
            err.contains("1..=16"),
            "expected error to name the 1..=16 bound, got: {err}"
        );
    }

    // -- OCC-21 20-char tolerance -------------------------------------------
    //
    // Upstream feeds occasionally ship OCC symbols one space short of the
    // canonical 21 bytes because the missing space sits INSIDE the root
    // padding region. The repair MUST peel the fixed 15-char suffix, then
    // re-pad the leading root slice to 6 chars so the strict 21-char
    // parser sees the exact same bytes it would have on a correctly
    // formatted input.

    #[test]
    fn from_str_occ21_20char_single_space_padded_root() {
        use std::str::FromStr;
        // 20-char form: "SPY" + two pad spaces (one short) + 15-char
        // suffix. Canonical OCC-21 has three pad spaces after SPY.
        let twenty = "SPY  260417C00550000";
        assert_eq!(twenty.len(), 20);
        let c20 = Contract::from_str(twenty).expect("20-char OCC-21 must repair");

        // 21-char canonical: "SPY" + 3 pad spaces + 15-char suffix.
        let twentyone = "SPY   260417C00550000";
        assert_eq!(twentyone.len(), 21);
        let c21 = Contract::from_str(twentyone).expect("21-char OCC-21 must parse");

        // Both strings MUST produce the same Contract. This pins the
        // repair behaviour: peeling the fixed 15-char suffix and
        // re-padding the root keeps the right-byte aligned, rather than
        // a trailing-space pad that would shift it into a digit slot.
        assert_eq!(c20, c21, "20-char and 21-char forms must parse identically");
        assert_eq!(&*c20.symbol, "SPY");
        assert_eq!(c20.expiration, Some(20_260_417));
        assert_eq!(c20.is_call, Some(true));
        assert_eq!(c20.strike_thousandths, Some(550_000));
    }

    #[test]
    fn from_str_occ21_20char_two_char_root() {
        use std::str::FromStr;
        // Root "T" (1 char) + 4 pad spaces (5-char root field) + suffix
        // = 20 chars. Canonical has 5 pad spaces after "T".
        let twenty = "T    260417C00150000";
        assert_eq!(twenty.len(), 20);
        let c = Contract::from_str(twenty).expect("20-char OCC-21 with short root must repair");
        assert_eq!(&*c.symbol, "T");
        assert_eq!(c.is_call, Some(true));
        assert_eq!(c.strike_thousandths, Some(150_000));
    }

    // -- Century scope (2000-2099) -------------------------------------------

    #[test]
    fn from_str_occ21_yy_99_maps_to_2099() {
        use std::str::FromStr;
        // YY=99 maps to 2099 under the documented 2000-2099 scope.
        // Any post-2099 expansion MUST add an explicit century argument;
        // the live FPSS feed ships only live contracts so a pre-2000
        // OCC symbol cannot reach this parser over the wire.
        let c = Contract::from_str("AAPL  990101C00100000").expect("YY=99 must parse");
        assert_eq!(
            c.expiration,
            Some(20_990_101),
            "YY=99 must map to 2099-01-01"
        );
        assert_eq!(&*c.symbol, "AAPL");
        assert_eq!(c.is_call, Some(true));
        assert_eq!(c.strike_thousandths, Some(100_000));
    }

    // -- validate_root: at most one dot --------------------------------------

    #[test]
    fn from_str_bare_root_rejects_multiple_dots() {
        use std::str::FromStr;
        // "A..B" has two dots and must be rejected — no real ticker
        // has more than one dot in its root.
        let err = Contract::from_str("A..B").unwrap_err().to_string();
        assert!(
            err.contains("at most one '.'"),
            "expected single-dot error, got: {err}"
        );
        assert!(
            err.contains("A..B"),
            "expected error to include the offending input, got: {err}"
        );
    }

    #[test]
    fn from_str_bare_root_allows_exactly_one_dot() {
        use std::str::FromStr;
        // Industry-practice single-dot compound tickers MUST still
        // parse — keeping BRK.A / BRK.B / RDS.A reachable.
        let c = Contract::from_str("BRK.B").expect("single-dot root must parse");
        assert_eq!(&*c.symbol, "BRK.B");
    }

    // ---------------------------------------------------------------------------
    // Property-based tests
    // ---------------------------------------------------------------------------
    //
    // Two round-trip invariants:
    //
    //   1. `Contract -> to_bytes -> from_bytes -> Contract` is the identity
    //      for any stock or option contract whose symbol matches the
    //      `validate_root` shape (1..=16 ASCII uppercase, optional single
    //      dot) and whose option fields lie in the canonical wire ranges
    //      (expiration `[20000101, 20991231]`, strike `[1, 100_000_000]`
    //      = $0.001..=$100_000 in i32 thousandths, right `'C' | 'P'`).
    //
    //   2. `OCC-21 string -> parse_occ21 -> Display -> OCC-21 string` is
    //      not the identity in this codebase because `Display` formats
    //      the verbose `"SYMBOL OPTION YYYYMMDD R STRIKE"` shape rather
    //      than the OCC-21 wire shape. The substantive round-trip is
    //      `OCC-21 -> Contract -> to_bytes -> from_bytes -> Contract`,
    //      which exercises both the OCC-21 parser and the wire codec on
    //      the same Contract value. Asserted as such below.

    use proptest::prelude::*;

    /// Strategy for a valid root symbol: 1..=6 ASCII uppercase letters
    /// (the OCC-21 root field is 6 bytes, so we cap there for the OCC-21
    /// arm). The wire codec accepts up to 16 bytes; the bytes round-trip
    /// arm uses the same strategy because every OCC-21-valid root is
    /// also wire-codec-valid, and capping at 6 keeps the two arms aligned.
    fn arbitrary_root() -> impl Strategy<Value = String> {
        proptest::collection::vec(b'A'..=b'Z', 1usize..=6)
            .prop_map(|bytes| String::from_utf8(bytes).expect("ASCII uppercase is valid UTF-8"))
    }

    /// Strategy for a valid YYYYMMDD expiration in the OCC-21 century scope.
    /// Uses a lazy day-count clamp per month so every emitted date is real.
    fn arbitrary_expiration() -> impl Strategy<Value = i32> {
        (2000i32..=2099, 1u32..=12).prop_flat_map(|(y, m)| {
            let dim = match m {
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31u32,
                4 | 6 | 9 | 11 => 30,
                2 => {
                    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
                    if leap {
                        29
                    } else {
                        28
                    }
                }
                _ => unreachable!(),
            };
            (Just(y), Just(m), 1u32..=dim).prop_map(|(y, m, d)| {
                // Reason: y, m, d are bounded above; product fits in i32.
                #[allow(clippy::cast_possible_wrap)]
                {
                    y * 10_000 + (m as i32) * 100 + (d as i32)
                }
            })
        })
    }

    /// Strategy for a stock-or-option Contract.
    fn arbitrary_contract() -> impl Strategy<Value = Contract> {
        prop_oneof![
            // Stock branch.
            arbitrary_root().prop_map(|root| Contract::stock(&root)),
            // Option branch: wire-format integer triple variant so the
            // strategy never has to round-trip strings -- the Contract
            // value itself is the unit under test.
            (
                arbitrary_root(),
                arbitrary_expiration(),
                any::<bool>(),
                // Strike: 8 zero-padded digits in OCC-21, so the
                // representable range is `[0, 99_999_999]` for the
                // OCC-21 round-trip. Cap at the OCC-21 ceiling
                // (99_999_999 = $99_999.999) and bottom at 1 to dodge
                // the edge case where `strike = 0` would parse but
                // collide with a malformed wire frame in callers that
                // sentinel on it.
                1i32..=99_999_999,
            )
                .prop_map(|(root, exp, is_call, strike)| {
                    Contract::option_raw(&root, exp, is_call, strike)
                }),
        ]
    }

    /// Build a canonical 21-char OCC-21 string from a strike-bearing
    /// contract. Mirrors the layout documented on `parse_occ21`.
    fn contract_to_occ21(c: &Contract) -> String {
        let exp = c.expiration.expect("option contract carries expiration");
        let strike = c
            .strike_thousandths
            .expect("option contract carries strike");
        let right = if c.is_call.expect("option contract carries right") {
            'C'
        } else {
            'P'
        };
        // YYYYMMDD -> YYMMDD (the parser maps YY -> 2000+YY).
        let yymmdd = exp - 20_000_000;
        // Pad the root to 6 chars with trailing spaces.
        let root_padded = format!("{:<6}", c.symbol);
        format!("{root_padded}{yymmdd:06}{right}{strike:08}")
    }

    proptest! {
        /// `Contract -> to_bytes -> from_bytes` is the identity for any
        /// well-formed stock or option contract.
        #[test]
        fn contract_bytes_roundtrip(c in arbitrary_contract()) {
            let bytes = c.to_bytes();
            let (parsed, consumed) = Contract::from_bytes(&bytes)
                .expect("encoder output must decode");
            prop_assert_eq!(consumed, bytes.len());
            prop_assert_eq!(parsed, c);
        }

        /// OCC-21 string parsing is byte-for-byte invertible: rebuilding
        /// the OCC-21 string from a parsed Contract reproduces the
        /// original input exactly.
        #[test]
        fn occ21_string_roundtrip(
            root in arbitrary_root(),
            exp in arbitrary_expiration(),
            is_call in any::<bool>(),
            strike in 1i32..=99_999_999,
        ) {
            let original = {
                let yymmdd = exp - 20_000_000;
                let right = if is_call { 'C' } else { 'P' };
                let root_padded = format!("{root:<6}");
                format!("{root_padded}{yymmdd:06}{right}{strike:08}")
            };
            prop_assert_eq!(original.len(), 21);

            let parsed: Contract = original.parse()
                .expect("well-formed OCC-21 string must parse");
            prop_assert_eq!(&*parsed.symbol, root.as_str());
            prop_assert_eq!(parsed.expiration, Some(exp));
            prop_assert_eq!(parsed.is_call, Some(is_call));
            prop_assert_eq!(parsed.strike_thousandths, Some(strike));

            let rebuilt = contract_to_occ21(&parsed);
            prop_assert_eq!(rebuilt, original);
        }

        /// Composite round-trip: `OCC-21 string -> Contract -> to_bytes
        /// -> from_bytes -> Contract` yields the same Contract that the
        /// OCC-21 parser produced. Pins the wire codec and the OCC-21
        /// parser against each other on the same value.
        #[test]
        fn occ21_then_bytes_roundtrip(
            root in arbitrary_root(),
            exp in arbitrary_expiration(),
            is_call in any::<bool>(),
            strike in 1i32..=99_999_999,
        ) {
            let occ21 = {
                let yymmdd = exp - 20_000_000;
                let right = if is_call { 'C' } else { 'P' };
                let root_padded = format!("{root:<6}");
                format!("{root_padded}{yymmdd:06}{right}{strike:08}")
            };
            let parsed: Contract = occ21.parse()
                .expect("well-formed OCC-21 string must parse");

            let bytes = parsed.to_bytes();
            let (decoded, _) = Contract::from_bytes(&bytes)
                .expect("encoder output must decode");
            prop_assert_eq!(decoded, parsed);
        }
    }
}
