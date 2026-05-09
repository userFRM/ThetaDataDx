//! Contract resolution for FPSS data events.

use std::sync::Arc;

use thetadatadx::fpss::protocol::Contract;
use thetadatadx::fpss::{FpssData, FpssEvent};

/// Return the parsed contract attached to a data event, if any.
///
/// Every `FpssData::*` variant now carries `contract: Arc<Contract>`
/// directly — the I/O thread populates it from its internal contract
/// cache at decode time. The WebSocket bridge no longer maintains its
/// own `contract_id -> Contract` map; cloning the `Arc` is a refcount
/// bump, not a heap allocation on the contract symbol.
pub(super) fn lookup_event_contract(event: &FpssEvent) -> Option<Arc<Contract>> {
    match event {
        FpssEvent::Data(FpssData::Quote { contract, .. })
        | FpssEvent::Data(FpssData::Trade { contract, .. })
        | FpssEvent::Data(FpssData::OpenInterest { contract, .. })
        | FpssEvent::Data(FpssData::Ohlcvc { contract, .. }) => Some(Arc::clone(contract)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_quote(contract: Arc<Contract>) -> FpssEvent {
        FpssEvent::Data(FpssData::Quote {
            contract,
            ms_of_day: 0,
            bid_size: 0,
            bid_exchange: 0,
            bid: 0.0,
            bid_condition: 0,
            ask_size: 0,
            ask_exchange: 0,
            ask: 0.0,
            ask_condition: 0,
            date: 0,
            received_at_ns: 0,
        })
    }

    /// Resolution must alias the same `Contract` heap allocation the
    /// event carries — a different pointer would mean we regressed to a
    /// per-event `Contract::clone`.
    #[test]
    fn event_contract_lookup_aliases_event_arc() {
        let contract = Arc::new(Contract::stock("AAPL"));
        let original_ptr = Arc::as_ptr(&contract);
        let event = make_quote(Arc::clone(&contract));
        let resolved = lookup_event_contract(&event).expect("resolution must succeed");
        assert_eq!(
            Arc::as_ptr(&resolved),
            original_ptr,
            "resolved Arc must alias the Contract heap cell carried by the event"
        );
    }
}
