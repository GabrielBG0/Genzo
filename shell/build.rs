// Bakes the contract version the bundled frontend was generated against into
// the binary, so the shell can refuse to start on a mismatch.
fn main() {
    let lock = "../contract/contract.lock";
    println!("cargo:rerun-if-changed={lock}");
    let text = std::fs::read_to_string(lock).expect("contract/contract.lock is missing");
    let json: serde_json::Value = serde_json::from_str(&text).expect("contract.lock is not JSON");
    let version = json["version"]
        .as_str()
        .expect("contract.lock has no version");
    println!("cargo:rustc-env=GENZO_APP_CONTRACT_VERSION={version}");
    tauri_build::build();
}
