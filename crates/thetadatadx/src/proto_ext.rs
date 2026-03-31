//! Extension helpers for converting between protobuf Price and tdx-encoding Price.
//!
//! The `Price` type lives in `tdx-encoding` (no proto dependency), so `from_proto`/`to_proto`
//! live here in `thetadatadx` where the proto types are available.

use crate::proto;
use tdx_encoding::types::price::Price;

/// Create a `Price` from a protobuf `Price` message.
pub fn price_from_proto(proto: &proto::Price) -> Price {
    Price::new(proto.value, proto.r#type)
}

/// Convert a `Price` to a protobuf `Price` message.
#[allow(dead_code)]
pub fn price_to_proto(price: &Price) -> proto::Price {
    proto::Price {
        value: price.value,
        r#type: price.price_type,
    }
}
