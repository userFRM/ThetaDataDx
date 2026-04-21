//! Contract-ID <-> subscriber bookkeeping for FPSS events.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use thetadatadx::fpss::protocol::Contract;
use thetadatadx::fpss::{FpssData, FpssEvent};

/// Peek the contract for an event's `contract_id`, if any, while briefly
/// holding the shared contract-map lock. Returns an `Arc<Contract>` so the
/// lock can be released before the (O(fields)) JSON serialization runs —
/// cloning the `Arc` is a refcount bump, not a heap allocation on the
/// `Contract::root: String`.
pub(super) fn lookup_event_contract(
    event: &FpssEvent,
    contract_map: &Mutex<HashMap<i32, Arc<Contract>>>,
) -> Option<Arc<Contract>> {
    let cid = match event {
        FpssEvent::Data(FpssData::Quote { contract_id, .. })
        | FpssEvent::Data(FpssData::Trade { contract_id, .. })
        | FpssEvent::Data(FpssData::OpenInterest { contract_id, .. })
        | FpssEvent::Data(FpssData::Ohlcvc { contract_id, .. }) => *contract_id,
        _ => return None,
    };
    let map = contract_map.lock().unwrap_or_else(|e| e.into_inner());
    map.get(&cid).cloned()
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_quote(contract_id: i32) -> FpssEvent {
        FpssEvent::Data(FpssData::Quote {
            contract_id,
            contract: Arc::new(Contract::stock("")),
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

    // -----------------------------------------------------------------------
    //  Hot-path: Arc<Contract> clone is a refcount bump, not a heap alloc
    // -----------------------------------------------------------------------

    #[test]
    fn arc_contract_clone_is_refcount_bump_not_string_alloc() {
        // Prove the structural claim: the map stores `Arc<Contract>`, so
        // `lookup_event_contract` returns an `Arc` that shares the SAME
        // heap allocation as the map entry. Before the fix, the lookup
        // returned a freshly-cloned `Contract` (new `String` heap alloc
        // per event). This test pins the invariant: same backing pointer.
        let map: Arc<Mutex<HashMap<i32, Arc<Contract>>>> = Arc::new(Mutex::new(HashMap::new()));
        let contract = Arc::new(Contract::stock("AAPL"));
        let original_ptr = Arc::as_ptr(&contract);
        map.lock().unwrap().insert(7, Arc::clone(&contract));

        let event = make_quote(7);
        let peeked = lookup_event_contract(&event, &map).expect("peek must succeed");
        assert_eq!(
            Arc::as_ptr(&peeked),
            original_ptr,
            "lookup must return an Arc pointing at the SAME Contract heap cell — \
             a different pointer means we regressed to per-event Contract::clone"
        );
    }
}
