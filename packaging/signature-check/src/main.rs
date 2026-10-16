//! Independent offline verification of the public artifacts consumed by Tauri.
//! This tool never accepts or reads a private signing key.
use base64::{engine::general_purpose::STANDARD, Engine};
use minisign_verify::{PublicKey, Signature};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{env, fs, path::Path};

fn document(path: &Path) -> Result<String, &'static str> {
    let value = fs::read_to_string(path).map_err(|_| "public_document_unreadable")?;
    let value = value.trim_start_matches('\u{feff}').trim();
    // Tauri .pub and .sig wrap the complete Minisign document in base64.
    // Plain Minisign public documents are accepted for interoperability.
    if value.starts_with("untrusted comment:") {
        Ok(value.to_owned())
    } else {
        let decoded = STANDARD
            .decode(value)
            .map_err(|_| "public_document_invalid_base64")?;
        String::from_utf8(decoded).map_err(|_| "public_document_invalid_utf8")
    }
}

fn verify(
    public_key_path: &Path,
    signature_path: &Path,
    artifact_path: &Path,
) -> Result<Value, &'static str> {
    let key = PublicKey::decode(&document(public_key_path)?).map_err(|_| "invalid_public_key")?;
    let signature =
        Signature::decode(&document(signature_path)?).map_err(|_| "invalid_signature_document")?;
    let mut bytes = fs::read(artifact_path).map_err(|_| "artifact_unreadable")?;
    if bytes.is_empty() {
        return Err("artifact_empty");
    }
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    // Match tauri-plugin-updater's verify(data, signature, allow_legacy=true).
    let signature_valid = key.verify(&bytes, &signature, true).is_ok();
    let offset = bytes.len() / 2;
    bytes[offset] ^= 1;
    let tampered_rejected = key.verify(&bytes, &signature, true).is_err();
    // No installer is executed and the original artifact is never changed.
    Ok(json!({
        "signature_valid": signature_valid,
        "tampered_rejected": tampered_rejected,
        "tamper": {"method": "in_memory_single_bit_flip", "byte_offset": offset},
        "sha256": sha256, "bytes": bytes.len(),
        "verifier": "minisign-verify 0.3.0",
        "private_key_read": false,
        "artifact_modified": false,
    }))
}

fn main() {
    let args: Vec<_> = env::args_os().collect();
    if args.len() != 4 {
        eprintln!("Usage: qingkan-signature-check PUBLIC_KEY_FILE SIGNATURE_FILE ARTIFACT");
        std::process::exit(2);
    }
    let result = verify(
        Path::new(&args[1]),
        Path::new(&args[2]),
        Path::new(&args[3]),
    );
    match result {
        Ok(value) => {
            let valid = value["signature_valid"] == true && value["tampered_rejected"] == true;
            println!("{value}");
            if !valid {
                std::process::exit(1);
            }
        }
        Err(code) => {
            println!("{}", json!({"signature_valid": false, "error": code}));
            std::process::exit(1);
        }
    }
}
