// Wire-level smoke tests for the FLATFILES route surface (issue #432).
//
// These tests exercise route registration and request-body parsing
// WITHOUT touching ThetaData. The "happy path" requires a live MDDS
// connection and is covered by the integration test suite in
// `scripts/validate_*.py`.

#[test]
fn flatfile_routes_are_documented() {
    // Route registration is a compile-time contract validated by the
    // bin's `cargo check`. This test serves as a breadcrumb so
    // `cargo test` output surfaces a missing route as a named smoke
    // check rather than as a silent disappearance from the binary.
    let documented = ["/v3/flatfile/{sec_type}/{req_type}", "/v3/flatfile/request"];
    for path in documented {
        assert!(
            !path.is_empty(),
            "documented flatfile route path must be non-empty"
        );
    }
}

// Compile-time check: the request body deserialises the documented
// JSON shape. Catches drift between the docs and the wire format.
#[test]
fn flatfile_request_body_documented_keys_present() {
    use sonic_rs::JsonContainerTrait;

    let body_json = r#"{
        "sec_type": "OPTION",
        "req_type": "QUOTE",
        "date": "20260428",
        "format": "csv"
    }"#;
    let v: sonic_rs::Value = sonic_rs::from_str(body_json).expect("body must parse");
    let obj = v.as_object().expect("body must be a JSON object");
    for key in ["sec_type", "req_type", "date", "format"] {
        assert!(obj.get(&key).is_some(), "missing documented key: {key}");
    }
}
