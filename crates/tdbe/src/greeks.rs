//! Black-Scholes Greeks calculator, ported from `ThetaData`'s Java implementation.
//!
//! Parameters:
//! - `s`: Spot price (underlying)
//! - `x`: Strike price
//! - `v`: Volatility (sigma)
//! - `r`: Risk-free rate
//! - `q`: Dividend yield
//! - `t`: Time to expiration (years)
//! - `right`: option side as a permissive string -- `"C"`/`"P"`,
//!   `"call"`/`"put"`, `"CALL"`/`"PUT"` all accepted (see
//!   [`crate::right::parse_right`])
//!
//! The low-level per-Greek primitives (`value`, `delta`, `theta`, ...) still
//! take a raw `is_call: bool` because they are pure-math helpers; the
//! user-facing aggregates [`all_greeks`] and [`implied_volatility`] take
//! `right: &str` so every SDK-level `right` surface funnels through the
//! same parser.
//!
//! # Edge-case guards
//!
//! All public Greek functions guard against `t <= 0.0` or `v <= 0.0` with
//! early returns of 0.0 (or the mathematically correct limit). This prevents
//! NaN/Inf contamination when Black-Scholes degenerates.

// 1 / sqrt(2 * pi)
const ONE_ROOT2PI: f64 = 0.398_942_280_401_432_7;

const MAX_TRIES: usize = 128;

/// Standard normal PDF: phi(x)
fn f1(x: f64) -> f64 {
    ONE_ROOT2PI * (-0.5 * x * x).exp()
}

/// Clamp Inf/NaN to 0.
fn realize(x: f64) -> f64 {
    if x.is_infinite() || x.is_nan() {
        0.0
    } else {
        x
    }
}

/// Return true if t or v make Black-Scholes degenerate.
#[inline]
fn is_degenerate(v: f64, t: f64) -> bool {
    t <= 0.0 || v <= 0.0
}

/// Standard normal CDF approximation (Zelen & Severo, 1964).
///
/// Uses Horner's method for polynomial evaluation: 4 fused multiply-adds instead
/// of 5 separate multiplies + 5 additions + 4 intermediate power variables.
/// Same Abramowitz & Stegun coefficients, same max error (~1.5e-7), fewer ops.
///
/// For IV solver loops (128 bisection iterations, each calling `norm_cdf` ~4x),
/// this is the dominant cost — Horner form shaves ~20% off the polynomial eval.
fn norm_cdf(x: f64) -> f64 {
    // Coefficients from Abramowitz & Stegun, formula 26.2.17.
    const A: [f64; 5] = [
        0.319_381_530,
        -0.356_563_782,
        1.781_477_937,
        -1.821_255_978,
        1.330_274_429,
    ];
    const P: f64 = 0.231_641_9;

    if x >= 0.0 {
        let t = 1.0 / (1.0 + P * x);
        // Horner evaluation: t*(A0 + t*(A1 + t*(A2 + t*(A3 + t*A4))))
        let poly = t * (A[0] + t * (A[1] + t * (A[2] + t * (A[3] + t * A[4]))));
        1.0 - f1(x) * poly
    } else {
        // N(-x) = 1 - N(x), but evaluate directly to avoid subtraction cancellation.
        let ax = -x;
        let t = 1.0 / (1.0 + P * ax);
        let poly = t * (A[0] + t * (A[1] + t * (A[2] + t * (A[3] + t * A[4]))));
        f1(ax) * poly
    }
}

// Reason: s, x, v, r, q, t are standard Black-Scholes parameter names.
#[allow(clippy::many_single_char_names)]
#[must_use]
pub fn d1(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    ((s / x).ln() + t * (r - q + v * v / 2.0)) / (v * t.sqrt())
}

// Reason: s, x, v, r, q, t are standard Black-Scholes parameter names.
#[allow(clippy::many_single_char_names)]
#[must_use]
pub fn d2(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    d1(s, x, v, r, q, t) - v * t.sqrt()
}

/// Compute `d1` and `d2` together, sharing `t.sqrt()`.
///
/// Callers that need both values should use this helper; calling `d1()` and
/// then `d2()` separately recomputes `d1` inside `d2` and double-pays the
/// `sqrt`/`ln`/`exp` cost of the formula.
// Reason: s, x, v, r, q, t are standard Black-Scholes parameter names.
#[allow(clippy::many_single_char_names)]
#[inline]
fn d1_d2(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> (f64, f64) {
    if is_degenerate(v, t) {
        return (0.0, 0.0);
    }
    let v_sqrt_t = v * t.sqrt();
    let d1 = ((s / x).ln() + t * (r - q + v * v / 2.0)) / v_sqrt_t;
    (d1, d1 - v_sqrt_t)
}

fn e1_from_d1(d1_val: f64) -> f64 {
    (-d1_val.powi(2) / 2.0).exp()
}

/// Black-Scholes theoretical option value.
// Reason: s, x, v, r, q, t are standard Black-Scholes parameter names.
#[allow(clippy::many_single_char_names)]
#[must_use]
pub fn value(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64, is_call: bool) -> f64 {
    if is_degenerate(v, t) {
        // At expiry / zero vol, value is intrinsic value.
        let intrinsic = if is_call {
            (s * (-q * t.max(0.0)).exp() - x * (-r * t.max(0.0)).exp()).max(0.0)
        } else {
            (x * (-r * t.max(0.0)).exp() - s * (-q * t.max(0.0)).exp()).max(0.0)
        };
        return intrinsic;
    }
    let (d1_val, d2_val) = d1_d2(s, x, v, r, q, t);
    if is_call {
        s * (-q * t).exp() * norm_cdf(d1_val) - (-r * t).exp() * x * norm_cdf(d2_val)
    } else {
        (-r * t).exp() * x * norm_cdf(-d2_val) - s * (-q * t).exp() * norm_cdf(-d1_val)
    }
}

// Reason: s, x, v, r, q, t are standard Black-Scholes parameter names.
#[allow(clippy::many_single_char_names, clippy::similar_names)]
#[must_use]
pub fn delta(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64, is_call: bool) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d1_val = d1(s, x, v, r, q, t);
    if is_call {
        (-q * t).exp() * norm_cdf(d1_val)
    } else {
        (-q * t).exp() * (norm_cdf(d1_val) - 1.0)
    }
}

// Reason: s, x, v, r, q, t are standard Black-Scholes parameter names.
#[allow(clippy::many_single_char_names, clippy::similar_names)]
#[must_use]
pub fn theta(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64, is_call: bool) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let (d1_val, d2_val) = d1_d2(s, x, v, r, q, t);
    let term1 = -(-q * t).exp() * (s * f1(d1_val) * v) / (2.0 * t.sqrt());
    if is_call {
        (term1 - r * x * (-r * t).exp() * norm_cdf(d2_val)
            + q * s * (-q * t).exp() * norm_cdf(d1_val))
            / 365.0
    } else {
        (term1 + r * x * (-r * t).exp() * norm_cdf(-d2_val)
            - q * s * (-q * t).exp() * norm_cdf(-d1_val))
            / 365.0
    }
}

// Reason: s, x, v, r, q, t are standard Black-Scholes parameter names.
#[allow(clippy::many_single_char_names, clippy::similar_names)]
#[must_use]
pub fn vega(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d1_val = d1(s, x, v, r, q, t);
    s * (-q * t).exp() * t.sqrt() * ONE_ROOT2PI * e1_from_d1(d1_val)
}

// Reason: s, x, v, r, q, t are standard Black-Scholes parameter names.
#[allow(clippy::many_single_char_names, clippy::similar_names)]
#[must_use]
pub fn rho(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64, is_call: bool) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d2_val = d2(s, x, v, r, q, t);
    if is_call {
        x * t * (-r * t).exp() * norm_cdf(d2_val)
    } else {
        -x * t * (-r * t).exp() * norm_cdf(-d2_val)
    }
}

// Reason: s, x, v, r, q, t are standard Black-Scholes parameter names.
#[allow(clippy::many_single_char_names, clippy::similar_names)]
#[must_use]
pub fn epsilon(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64, is_call: bool) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d1_val = d1(s, x, v, r, q, t);
    if is_call {
        realize(-s * t * (-q * t).exp() * norm_cdf(d1_val))
    } else {
        realize(s * t * (-q * t).exp() * norm_cdf(-d1_val))
    }
}

// Reason: s, x, v, r, q, t are standard Black-Scholes parameter names.
#[allow(clippy::many_single_char_names, clippy::similar_names)]
#[must_use]
pub fn lambda(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64, is_call: bool) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    realize(delta(s, x, v, r, q, t, is_call) * s / value(s, x, v, r, q, t, is_call))
}

// Reason: s, x, v, r, q, t are standard Black-Scholes parameter names.
#[allow(clippy::many_single_char_names, clippy::similar_names)]
#[must_use]
pub fn gamma(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d1_val = d1(s, x, v, r, q, t);
    (-q * t).exp() / (s * v * t.sqrt()) * ONE_ROOT2PI * e1_from_d1(d1_val)
}

// Reason: s, x, v, r, q, t are standard Black-Scholes parameter names.
#[allow(clippy::many_single_char_names, clippy::similar_names)]
#[must_use]
pub fn vanna(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let (d1_val, d2_val) = d1_d2(s, x, v, r, q, t);
    -(-q * t).exp() * f1(d1_val) * d2_val / v
}

// Reason: s, x, v, r, q, t are standard Black-Scholes parameter names.
#[allow(clippy::many_single_char_names, clippy::similar_names)]
#[must_use]
pub fn charm(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64, is_call: bool) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let (d1_val, d2_val) = d1_d2(s, x, v, r, q, t);
    let p1 = (2.0 * (r - q) * t - d2_val * v * t.sqrt()) / (2.0 * t * v * t.sqrt());
    if is_call {
        q * (-q * t).exp() * norm_cdf(d1_val) - (-q * t).exp() * f1(d1_val) * p1
    } else {
        -q * (-q * t).exp() * norm_cdf(-d1_val) - (-q * t).exp() * f1(d1_val) * p1
    }
}

// Reason: s, x, v, r, q, t are standard Black-Scholes parameter names.
#[allow(clippy::many_single_char_names, clippy::similar_names)]
#[must_use]
pub fn vomma(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let (d1_val, d2_val) = d1_d2(s, x, v, r, q, t);
    vega(s, x, v, r, q, t) * (d1_val * d2_val / v)
}

// Reason: s, x, v, r, q, t are standard Black-Scholes parameter names.
#[allow(clippy::many_single_char_names, clippy::similar_names)]
#[must_use]
pub fn veta(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let (d1_val, d2_val) = d1_d2(s, x, v, r, q, t);
    -s * (-q * t).exp()
        * f1(d1_val)
        * t.sqrt()
        * (q + (r - q) * d1_val / (v * t.sqrt()) - (1.0 + d1_val * d2_val) / (2.0 * t))
}

// Reason: s, x, v, r, q, t are standard Black-Scholes parameter names.
#[allow(clippy::many_single_char_names, clippy::similar_names)]
#[must_use]
pub fn speed(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d1_val = d1(s, x, v, r, q, t);
    -(-q * t).exp() * f1(d1_val) / (s * s * v * t.sqrt()) * (d1_val / (v * t.sqrt()) + 1.0)
}

// Reason: s, x, v, r, q, t are standard Black-Scholes parameter names.
#[allow(clippy::many_single_char_names, clippy::similar_names)]
#[must_use]
pub fn zomma(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let (d1_val, d2_val) = d1_d2(s, x, v, r, q, t);
    (-q * t).exp() * f1(d1_val) * (d1_val * d2_val - 1.0) / (s * v * v * t.sqrt())
}

// Reason: s, x, v, r, q, t are standard Black-Scholes parameter names.
#[allow(clippy::many_single_char_names, clippy::similar_names)]
#[must_use]
pub fn color(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let (d1_val, d2_val) = d1_d2(s, x, v, r, q, t);
    -(-q * t).exp() * f1(d1_val) / (2.0 * s * t * v * t.sqrt())
        * (2.0 * q * t
            + 1.0
            + (2.0 * (r - q) * t - d2_val * v * t.sqrt()) / (v * t.sqrt()) * d1_val)
}

// Reason: s, x, v, r, q, t are standard Black-Scholes parameter names.
#[allow(clippy::many_single_char_names, clippy::similar_names)]
#[must_use]
pub fn ultima(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let (d1_val, d2_val) = d1_d2(s, x, v, r, q, t);
    let out = -vega(s, x, v, r, q, t) / (v * v)
        * (d1_val * d2_val * (1.0 - d1_val * d2_val) + d1_val * d1_val + d2_val * d2_val);
    out.clamp(-100.0, 100.0)
}

// Reason: s, x, v, r, q, t are standard Black-Scholes parameter names.
#[allow(clippy::many_single_char_names, clippy::similar_names)]
#[must_use]
pub fn dual_delta(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64, is_call: bool) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d2_val = d2(s, x, v, r, q, t);
    if is_call {
        -(-r * t).exp() * norm_cdf(d2_val)
    } else {
        (-r * t).exp() * norm_cdf(-d2_val)
    }
}

// Reason: s, x, v, r, q, t are standard Black-Scholes parameter names.
#[allow(clippy::many_single_char_names, clippy::similar_names)]
#[must_use]
pub fn dual_gamma(s: f64, x: f64, v: f64, r: f64, q: f64, t: f64) -> f64 {
    if is_degenerate(v, t) {
        return 0.0;
    }
    let d2_val = d2(s, x, v, r, q, t);
    (-r * t).exp() * f1(d2_val) / (x * v * t.sqrt())
}

/// Implied volatility solver using bisection. Returns `(iv, error)` on
/// success.
///
/// `right` accepts `"C"`/`"P"`/`"call"`/`"put"` case-insensitively (see
/// [`crate::right::parse_right_strict`]).
///
/// # Errors
///
/// Returns [`crate::Error::Config`] if `right` is not one of the accepted
/// forms or resolves to `both`/`*`. Strict-parse failures from
/// [`crate::right::parse_right_strict`] surface here directly.
// Reason: s, x, r, q, t are standard Black-Scholes parameter names.
#[allow(clippy::many_single_char_names)]
pub fn implied_volatility(
    s: f64,
    x: f64,
    r: f64,
    q: f64,
    t: f64,
    option_price: f64,
    right: &str,
) -> Result<(f64, f64), crate::Error> {
    let is_call = crate::right::parse_right_strict(right)?
        .as_is_call()
        .ok_or_else(|| {
            crate::Error::Config(format!(
                "option right '{right}' resolves to 'both' but a single side is required"
            ))
        })?;
    if t <= 0.0 || option_price <= 0.0 {
        return Ok((0.0, 0.0));
    }
    let mut out = [0.0f64; 2];
    iv_bisection(s, x, r, q, t, option_price, is_call, &mut out);
    Ok((out[0], out[1]))
}

// Reason: s, x, r, q, t, o are standard Black-Scholes/IV solver parameter names.
#[allow(
    clippy::too_many_arguments,
    clippy::many_single_char_names,
    clippy::similar_names
)]
fn iv_bisection(s: f64, x: f64, r: f64, q: f64, t: f64, o: f64, is_call: bool, out: &mut [f64; 2]) {
    // Check intrinsic value boundary
    if value(s, x, 0.0, r, q, t, is_call) > o {
        out[0] = 0.0;
        out[1] = ((value(s, x, 0.0, r, q, t, is_call) - o) / o).clamp(-100.0, 100.0);
        return;
    }

    let mut guess = 0.5;
    let mut start = 0.0;
    let mut end = guess;
    let mut changer = 0.2;

    // Find upper bound
    for _ in 0..32 {
        end += changer;
        if value(s, x, end, r, q, t, is_call) > o {
            break;
        }
        changer *= 2.0;
    }
    for _ in 0..MAX_TRIES {
        let v = value(s, x, guess, r, q, t, is_call);
        if (v - o).abs() < 0.001 {
            out[0] = guess;
            out[1] = ((v - o) / o).clamp(-100.0, 100.0);
            return;
        }
        if v > o {
            end = guess;
            guess -= (end - start) / 2.0;
        } else {
            start = guess;
            guess += (end - start) / 2.0;
        }
    }

    let v = value(s, x, guess, r, q, t, is_call);
    out[0] = guess;
    out[1] = ((v - o) / o).clamp(-100.0, 100.0);
}

/// All Greeks computed in a single struct.
#[derive(Debug, Clone, Copy)]
pub struct GreeksResult {
    pub value: f64,
    pub delta: f64,
    pub gamma: f64,
    pub theta: f64,
    pub vega: f64,
    pub rho: f64,
    pub iv: f64,
    pub iv_error: f64,
    // Second order
    pub vanna: f64,
    pub charm: f64,
    pub vomma: f64,
    pub veta: f64,
    /// `vera` (a.k.a. DvegaDr) — sensitivity of vega to the risk-free
    /// rate. Computed as the textbook `-K * exp(-r*T) * T * sqrt(T) * phi(d2)`
    /// where `phi` is the standard-normal PDF. The vendor's exact
    /// definition is not published; if it ever turns out to differ in
    /// sign or scale, callers can rescale locally — the field is
    /// internally consistent and pinned to the formula by a unit test.
    pub vera: f64,
    // Third order
    pub speed: f64,
    pub zomma: f64,
    pub color: f64,
    pub ultima: f64,
    // Auxiliary
    pub d1: f64,
    pub d2: f64,
    pub dual_delta: f64,
    pub dual_gamma: f64,
    pub epsilon: f64,
    pub lambda: f64,
}

/// Compute all 23 Greeks at once with maximally shared intermediates.
///
/// Precomputes `d1`, `d2`, and all shared sub-expressions (exponentials,
/// CDF values, products) once, then evaluates Greeks in dependency tiers:
///
/// **Tier 0 — Shared intermediates** (8 precomputed values):
///   `sqrt_t`, `v_sqrt_t`, `d1`, `d2`, `exp(-qt)`, `exp(-rt)`, `N(d1)`, `N(d2)`,
///   `f1(d1)`, `f1(d2)`, `e1`, `d1*d2`
///
/// **Tier 1 — First-order Greeks** (value, delta, gamma, theta, vega, rho, epsilon):
///   All share `exp_neg_qt`, `nd1/nd2`, `f1_d1`, `e1_val`.
///
/// **Tier 2 — Second-order Greeks** (vanna, charm, vomma, veta):
///   Depend on Tier 1 intermediates + `d1_d2` product.
///
/// **Tier 3 — Third-order Greeks** (speed, zomma, color, ultima):
///   Depend on Tier 1/2 intermediates.
///
/// **Auxiliary** (lambda, `dual_delta`, `dual_gamma)`:
///   Depend on Tier 1 values.
///
/// This avoids ~20 redundant `d1`/`d2` recalculations and ~40 redundant
/// `exp()`/`norm_cdf()` calls compared to calling each Greek individually.
///
/// `right` accepts `"C"`/`"P"`/`"call"`/`"put"` case-insensitively (see
/// [`crate::right::parse_right_strict`]).
///
/// # Errors
///
/// Returns [`crate::Error::Config`] if `right` is not one of the accepted
/// forms or resolves to `both`/`*`. Strict-parse failures from
/// [`crate::right::parse_right_strict`] surface here directly.
// Reason: s, x, r, q, t are standard Black-Scholes parameter names.
// Reason: 23-Greek computation cannot be meaningfully split without duplicating intermediates.
#[allow(
    clippy::many_single_char_names,
    clippy::similar_names,
    clippy::too_many_lines
)]
pub fn all_greeks(
    s: f64,
    x: f64,
    r: f64,
    q: f64,
    t: f64,
    option_price: f64,
    right: &str,
) -> Result<GreeksResult, crate::Error> {
    let is_call = crate::right::parse_right_strict(right)?
        .as_is_call()
        .ok_or_else(|| {
            crate::Error::Config(format!(
                "option right '{right}' resolves to 'both' but a single side is required"
            ))
        })?;

    // Inline the IV solver to keep the `right` parse at this layer (avoids
    // a second parse that `implied_volatility(&str)` would otherwise do).
    let (iv_val, iv_err) = if t <= 0.0 || option_price <= 0.0 {
        (0.0, 0.0)
    } else {
        let mut out = [0.0f64; 2];
        iv_bisection(s, x, r, q, t, option_price, is_call, &mut out);
        (out[0], out[1])
    };
    let v = iv_val;

    // Guard: if vol or time is degenerate, return all zeros (except value = intrinsic).
    if is_degenerate(v, t) {
        return Ok(GreeksResult {
            value: value(s, x, v, r, q, t, is_call),
            delta: 0.0,
            gamma: 0.0,
            theta: 0.0,
            vega: 0.0,
            rho: 0.0,
            iv: iv_val,
            iv_error: iv_err,
            vanna: 0.0,
            charm: 0.0,
            vomma: 0.0,
            veta: 0.0,
            vera: 0.0,
            speed: 0.0,
            zomma: 0.0,
            color: 0.0,
            ultima: 0.0,
            d1: 0.0,
            d2: 0.0,
            dual_delta: 0.0,
            dual_gamma: 0.0,
            epsilon: 0.0,
            lambda: 0.0,
        });
    }

    // -- Tier 0: Shared intermediates -----------------------------------------
    let sqrt_t = t.sqrt();
    let v_sqrt_t = v * sqrt_t; // used 8+ times below
    let d1_val = ((s / x).ln() + t * (r - q + v * v / 2.0)) / v_sqrt_t;
    let d2_val = d1_val - v_sqrt_t;
    let e1_val = (-d1_val * d1_val / 2.0).exp(); // == e1_from_d1(d1_val)
    let f1_d1 = ONE_ROOT2PI * e1_val; // == f1(d1_val)
    let f1_d2 = f1(d2_val); // needed for dual_gamma
    let exp_neg_qt = (-q * t).exp();
    let exp_neg_rt = (-r * t).exp();
    let nd1 = norm_cdf(d1_val);
    let nd2 = norm_cdf(d2_val);
    let n_neg_d1 = norm_cdf(-d1_val);
    let n_neg_d2 = norm_cdf(-d2_val);
    let d1_d2 = d1_val * d2_val; // used by vomma, veta, zomma, color, ultima
    let r_minus_q = r - q;

    // Common sub-expression: exp_neg_qt * f1_d1 (used by vanna, charm, veta, speed, zomma, color)
    let eqt_f1d1 = exp_neg_qt * f1_d1;
    // Common sub-expression: 1 / (s * v * sqrt_t) (used by gamma, speed)
    let inv_s_v_sqrt_t = 1.0 / (s * v_sqrt_t);

    // -- Tier 1: First-order Greeks (value, delta, gamma, theta, vega, rho, epsilon) --
    let value_val = if is_call {
        s * exp_neg_qt * nd1 - exp_neg_rt * x * nd2
    } else {
        exp_neg_rt * x * n_neg_d2 - s * exp_neg_qt * n_neg_d1
    };

    let delta_val = if is_call {
        exp_neg_qt * nd1
    } else {
        exp_neg_qt * (nd1 - 1.0)
    };

    let gamma_val = exp_neg_qt * inv_s_v_sqrt_t * ONE_ROOT2PI * e1_val;

    let theta_term1 = -eqt_f1d1 * s * v / (2.0 * sqrt_t);
    let theta_val = if is_call {
        (theta_term1 - r * x * exp_neg_rt * nd2 + q * s * exp_neg_qt * nd1) / 365.0
    } else {
        (theta_term1 + r * x * exp_neg_rt * n_neg_d2 - q * s * exp_neg_qt * n_neg_d1) / 365.0
    };

    let vega_val = s * exp_neg_qt * sqrt_t * ONE_ROOT2PI * e1_val;

    let rho_val = if is_call {
        x * t * exp_neg_rt * nd2
    } else {
        -x * t * exp_neg_rt * n_neg_d2
    };

    let epsilon_val = if is_call {
        realize(-s * t * exp_neg_qt * nd1)
    } else {
        realize(s * t * exp_neg_qt * n_neg_d1)
    };

    // Lambda depends on value + delta (still first-order conceptually)
    let lambda_val = if value_val.abs() > f64::EPSILON {
        realize(delta_val * s / value_val)
    } else {
        0.0
    };

    // -- Tier 2: Second-order Greeks (vanna, charm, vomma, veta) --------------
    let vanna_val = -eqt_f1d1 * d2_val / v;

    let charm_p1 = (2.0 * r_minus_q * t - d2_val * v_sqrt_t) / (2.0 * t * v_sqrt_t);
    let charm_val = if is_call {
        q * exp_neg_qt * nd1 - eqt_f1d1 * charm_p1
    } else {
        -q * exp_neg_qt * n_neg_d1 - eqt_f1d1 * charm_p1
    };

    let vomma_val = vega_val * (d1_d2 / v);

    let veta_val =
        -s * eqt_f1d1 * sqrt_t * (q + r_minus_q * d1_val / v_sqrt_t - (1.0 + d1_d2) / (2.0 * t));

    // vera (DvegaDr): cross-sensitivity of vega to the risk-free rate.
    // Textbook form: vera = -K * exp(-r*T) * T * sqrt(T) * phi(d2),
    // with phi the standard-normal PDF (= ONE_ROOT2PI * exp(-d2^2 / 2)).
    let vera_val = -x * exp_neg_rt * t * sqrt_t * f1_d2;

    // -- Tier 3: Third-order Greeks (speed, zomma, color, ultima) -------------
    let speed_val = -eqt_f1d1 * inv_s_v_sqrt_t / s * (d1_val / v_sqrt_t + 1.0);

    let zomma_val = eqt_f1d1 * (d1_d2 - 1.0) / (s * v * v_sqrt_t);

    let color_val = -eqt_f1d1 / (2.0 * s * t * v_sqrt_t)
        * (2.0 * q * t + 1.0 + (2.0 * r_minus_q * t - d2_val * v_sqrt_t) / v_sqrt_t * d1_val);

    let ultima_raw =
        -vega_val / (v * v) * (d1_d2 * (1.0 - d1_d2) + d1_val * d1_val + d2_val * d2_val);
    let ultima_val = ultima_raw.clamp(-100.0, 100.0);

    // -- Auxiliary: Dual Greeks ------------------------------------------------
    let dual_delta_val = if is_call {
        -exp_neg_rt * nd2
    } else {
        exp_neg_rt * n_neg_d2
    };

    let dual_gamma_val = exp_neg_rt * f1_d2 / (x * v_sqrt_t);

    Ok(GreeksResult {
        value: value_val,
        delta: delta_val,
        gamma: gamma_val,
        theta: theta_val,
        vega: vega_val,
        rho: rho_val,
        iv: iv_val,
        iv_error: iv_err,
        vanna: vanna_val,
        charm: charm_val,
        vomma: vomma_val,
        veta: veta_val,
        vera: vera_val,
        speed: speed_val,
        zomma: zomma_val,
        color: color_val,
        ultima: ultima_val,
        d1: d1_val,
        d2: d2_val,
        dual_delta: dual_delta_val,
        dual_gamma: dual_gamma_val,
        epsilon: epsilon_val,
        lambda: lambda_val,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Assert that a value is finite (not NaN, not Inf).
    fn assert_finite(val: f64, label: &str) {
        assert!(val.is_finite(), "{label} must be finite, got {val}");
    }

    #[test]
    fn test_call_value() {
        // SPY ~$450, strike $450, vol 20%, r=5%, q=1.5%, 30 days
        let v = value(450.0, 450.0, 0.20, 0.05, 0.015, 30.0 / 365.0, true);
        assert!(v > 5.0 && v < 15.0, "ATM call value: {v}");
    }

    #[test]
    fn test_put_call_parity() {
        let s = 100.0;
        let x = 100.0;
        let v = 0.25;
        let r = 0.05;
        let q = 0.02;
        let t = 0.5;

        let call = value(s, x, v, r, q, t, true);
        let put = value(s, x, v, r, q, t, false);
        let parity = s * (-q * t).exp() - x * (-r * t).exp();
        assert!(
            (call - put - parity).abs() < 1e-10,
            "Put-call parity violated: call={call}, put={put}, parity={parity}"
        );
    }

    #[test]
    fn test_iv_roundtrip() {
        let s = 150.0;
        let x = 155.0;
        let r = 0.05;
        let q = 0.015;
        let t = 45.0 / 365.0;
        let true_vol = 0.22;

        let price = value(s, x, true_vol, r, q, t, true);
        let (iv, err) = implied_volatility(s, x, r, q, t, price, "C").expect("valid right");
        assert!(
            (iv - true_vol).abs() < 0.005,
            "IV roundtrip: expected {true_vol}, got {iv}, err={err}"
        );
    }

    #[test]
    fn greeks_api_accepts_permissive_right() {
        let s = 100.0;
        let x = 100.0;
        let r = 0.05;
        let q = 0.01;
        let t = 30.0 / 365.0;
        let price = value(s, x, 0.2, r, q, t, true);

        // Every accepted `right` form must agree with the call-side result.
        let call_ref = all_greeks(s, x, r, q, t, price, "C").expect("valid right");
        for form in ["call", "CALL", "Call", "c"] {
            let g = all_greeks(s, x, r, q, t, price, form).expect("valid right");
            assert!((g.delta - call_ref.delta).abs() < 1e-12, "form={form}");
        }

        let put_price = value(s, x, 0.2, r, q, t, false);
        let put_ref = all_greeks(s, x, r, q, t, put_price, "P").expect("valid right");
        for form in ["put", "PUT", "Put", "p"] {
            let g = all_greeks(s, x, r, q, t, put_price, form).expect("valid right");
            assert!((g.delta - put_ref.delta).abs() < 1e-12, "form={form}");
        }

        // Same for `implied_volatility`.
        let (iv_c, _) = implied_volatility(s, x, r, q, t, price, "call").expect("valid right");
        let (iv_short, _) = implied_volatility(s, x, r, q, t, price, "C").expect("valid right");
        assert!((iv_c - iv_short).abs() < 1e-12);
    }

    #[test]
    fn all_greeks_errors_on_garbage_right() {
        let err = all_greeks(100.0, 100.0, 0.05, 0.01, 0.25, 5.0, "xyz").unwrap_err();
        assert!(matches!(err, crate::Error::Config(_)));
        assert!(err.to_string().contains("invalid option right"));
    }

    #[test]
    fn all_greeks_errors_on_both() {
        let err = all_greeks(100.0, 100.0, 0.05, 0.01, 0.25, 5.0, "both").unwrap_err();
        assert!(matches!(err, crate::Error::Config(_)));
        assert!(err.to_string().contains("resolves to 'both'"));
    }

    #[test]
    fn implied_volatility_errors_on_garbage_right() {
        let err = implied_volatility(100.0, 100.0, 0.05, 0.01, 0.25, 5.0, "xyz").unwrap_err();
        assert!(matches!(err, crate::Error::Config(_)));
        assert!(err.to_string().contains("invalid option right"));
    }

    // -- Edge-case tests (Fix #10 + Fix #16) --

    #[test]
    fn edge_t_zero_returns_finite() {
        let s = 100.0;
        let x = 100.0;
        let v = 0.20;
        let r = 0.05;
        let q = 0.01;
        let t = 0.0;

        // All public Greeks must return finite values.
        assert_finite(d1(s, x, v, r, q, t), "d1(t=0)");
        assert_finite(d2(s, x, v, r, q, t), "d2(t=0)");
        assert_finite(value(s, x, v, r, q, t, true), "value(t=0, call)");
        assert_finite(value(s, x, v, r, q, t, false), "value(t=0, put)");
        assert_finite(delta(s, x, v, r, q, t, true), "delta(t=0)");
        assert_finite(theta(s, x, v, r, q, t, true), "theta(t=0)");
        assert_finite(vega(s, x, v, r, q, t), "vega(t=0)");
        assert_finite(rho(s, x, v, r, q, t, true), "rho(t=0)");
        assert_finite(gamma(s, x, v, r, q, t), "gamma(t=0)");
        assert_finite(vanna(s, x, v, r, q, t), "vanna(t=0)");
        assert_finite(charm(s, x, v, r, q, t, true), "charm(t=0)");
        assert_finite(vomma(s, x, v, r, q, t), "vomma(t=0)");
        assert_finite(veta(s, x, v, r, q, t), "veta(t=0)");
        assert_finite(speed(s, x, v, r, q, t), "speed(t=0)");
        assert_finite(zomma(s, x, v, r, q, t), "zomma(t=0)");
        assert_finite(color(s, x, v, r, q, t), "color(t=0)");
        assert_finite(ultima(s, x, v, r, q, t), "ultima(t=0)");
        assert_finite(dual_delta(s, x, v, r, q, t, true), "dual_delta(t=0)");
        assert_finite(dual_gamma(s, x, v, r, q, t), "dual_gamma(t=0)");
        assert_finite(epsilon(s, x, v, r, q, t, true), "epsilon(t=0)");
        assert_finite(lambda(s, x, v, r, q, t, true), "lambda(t=0)");
    }

    #[test]
    fn edge_v_zero_returns_finite() {
        let s = 100.0;
        let x = 100.0;
        let v = 0.0;
        let r = 0.05;
        let q = 0.01;
        let t = 0.5;

        assert_finite(d1(s, x, v, r, q, t), "d1(v=0)");
        assert_finite(d2(s, x, v, r, q, t), "d2(v=0)");
        assert_finite(value(s, x, v, r, q, t, true), "value(v=0, call)");
        assert_finite(value(s, x, v, r, q, t, false), "value(v=0, put)");
        assert_finite(delta(s, x, v, r, q, t, true), "delta(v=0)");
        assert_finite(theta(s, x, v, r, q, t, true), "theta(v=0)");
        assert_finite(gamma(s, x, v, r, q, t), "gamma(v=0)");
        assert_finite(vega(s, x, v, r, q, t), "vega(v=0)");
    }

    #[test]
    fn edge_option_price_zero_returns_finite() {
        let s = 100.0;
        let x = 100.0;
        let r = 0.05;
        let q = 0.01;
        let t = 0.5;

        let (iv, err) = implied_volatility(s, x, r, q, t, 0.0, "C").expect("valid right");
        assert_finite(iv, "iv(option_price=0)");
        assert_finite(err, "iv_err(option_price=0)");
        assert_eq!(iv, 0.0);

        let g = all_greeks(s, x, r, q, t, 0.0, "C").expect("valid right");
        assert_finite(g.value, "all_greeks(option_price=0).value");
        assert_finite(g.delta, "all_greeks(option_price=0).delta");
        assert_finite(g.gamma, "all_greeks(option_price=0).gamma");
        assert_finite(g.theta, "all_greeks(option_price=0).theta");
    }

    #[test]
    fn edge_atm_at_expiry_returns_finite() {
        // s == x (ATM) and t == 0 (at expiry).
        let s = 100.0;
        let x = 100.0;
        let r = 0.05;
        let q = 0.01;
        let t = 0.0;

        let g = all_greeks(s, x, r, q, t, 5.0, "C").expect("valid right");
        assert_finite(g.value, "all_greeks(ATM, t=0).value");
        assert_finite(g.delta, "all_greeks(ATM, t=0).delta");
        assert_finite(g.gamma, "all_greeks(ATM, t=0).gamma");
        assert_finite(g.theta, "all_greeks(ATM, t=0).theta");
        assert_finite(g.vega, "all_greeks(ATM, t=0).vega");
        assert_finite(g.rho, "all_greeks(ATM, t=0).rho");
        assert_finite(g.iv, "all_greeks(ATM, t=0).iv");
        assert_finite(g.iv_error, "all_greeks(ATM, t=0).iv_error");
        assert_finite(g.vanna, "all_greeks(ATM, t=0).vanna");
        assert_finite(g.charm, "all_greeks(ATM, t=0).charm");
        assert_finite(g.vera, "all_greeks(ATM, t=0).vera");
        assert_finite(g.d1, "all_greeks(ATM, t=0).d1");
        assert_finite(g.d2, "all_greeks(ATM, t=0).d2");
    }

    /// `vera = -K * exp(-r*T) * T * sqrt(T) * phi(d2)`.
    ///
    /// Pin the field to the textbook DvegaDr formula. Expected value
    /// computed by hand with `phi(d2) = ONE_ROOT2PI * exp(-d2*d2/2)`.
    #[test]
    fn vera_matches_textbook_dvega_dr() {
        let s = 100.0;
        let x = 100.0;
        let r = 0.05;
        let q = 0.00;
        let t = 1.0;
        // True vol used to build a self-consistent option price; the IV
        // solver inside all_greeks then recovers (approximately) the same
        // vol, so we compute the expected vera using the recovered iv.
        let true_vol = 0.20;
        let price = value(s, x, true_vol, r, q, t, true);

        let g = all_greeks(s, x, r, q, t, price, "C").expect("valid right");
        let v_recovered = g.iv;

        // Recompute d2 with the iv that all_greeks settled on — this
        // mirrors the helper's own intermediates so any solver wobble
        // does not destabilise the comparison.
        let d2_val = d2(s, x, v_recovered, r, q, t);
        let phi_d2 = (-d2_val * d2_val / 2.0).exp() / (2.0 * std::f64::consts::PI).sqrt();
        let expected_vera = -x * (-r * t).exp() * t * t.sqrt() * phi_d2;

        assert!(
            (g.vera - expected_vera).abs() < 1e-10,
            "vera mismatch: got {got}, expected {expected_vera}",
            got = g.vera
        );
        // Sanity check sign and order of magnitude (call-on-stock,
        // short-rate scenario): expected ~ -37.52 by independent
        // longhand calculation: 100 * exp(-0.05) * 1 * 1 * phi(d2)
        // where the IV solver lands close to but not exactly on the
        // 0.20 vol used to build the option price.
        assert!(g.vera < 0.0, "vera should be negative for this scenario");
        assert!(
            g.vera > -40.0 && g.vera < -35.0,
            "vera order-of-magnitude check: {got}",
            got = g.vera
        );
    }

    #[test]
    fn all_greeks_precomputed_matches_individual() {
        // Verify that the precomputed all_greeks produces the same results
        // as calling each individual function.
        let s = 150.0;
        let x = 155.0;
        let r = 0.05;
        let q = 0.015;
        let t = 45.0 / 365.0;
        let price = value(s, x, 0.22, r, q, t, true);

        let g = all_greeks(s, x, r, q, t, price, "C").expect("valid right");
        let v = g.iv;

        let eps = 1e-10;
        assert!(
            (g.value - value(s, x, v, r, q, t, true)).abs() < eps,
            "value mismatch"
        );
        assert!(
            (g.delta - delta(s, x, v, r, q, t, true)).abs() < eps,
            "delta mismatch"
        );
        assert!(
            (g.gamma - gamma(s, x, v, r, q, t)).abs() < eps,
            "gamma mismatch"
        );
        assert!(
            (g.theta - theta(s, x, v, r, q, t, true)).abs() < eps,
            "theta mismatch"
        );
        assert!(
            (g.vega - vega(s, x, v, r, q, t)).abs() < eps,
            "vega mismatch"
        );
        assert!(
            (g.rho - rho(s, x, v, r, q, t, true)).abs() < eps,
            "rho mismatch"
        );
        assert!((g.d1 - d1(s, x, v, r, q, t)).abs() < eps, "d1 mismatch");
        assert!((g.d2 - d2(s, x, v, r, q, t)).abs() < eps, "d2 mismatch");
    }
}
