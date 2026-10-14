//! Fails when `contract/` does not match the Rust types, or when the types
//! changed without a `CONTRACT_VERSION` bump.

use std::path::PathBuf;

use genzo_model::typescript::check_checked_in;

#[test]
fn checked_in_contract_matches_types() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../contract");
    let ts = std::fs::read_to_string(dir.join("genzo.ts")).unwrap_or_default();
    let lock = std::fs::read_to_string(dir.join("contract.lock")).unwrap_or_default();
    if let Err(drift) = check_checked_in(&ts, &lock) {
        panic!("{drift}");
    }
}
