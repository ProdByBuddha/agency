//! Cryptographic Sovereignty
//! 
//! Manages the Agency's Identity Key (Ed25519).
//! Allows the organism to sign messages, bounties, and transactions,
//! proving its identity to the Swarm without centralized authorities.

use ed25519_dalek::{Signer, Verifier, SigningKey, VerifyingKey, Signature};
use rand::rngs::OsRng;
use std::path::PathBuf;
use std::fs;
use anyhow::{Result, Context};
use tracing::{info, warn};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

pub struct SovereignIdentity {
    keypair: SigningKey,
    public_key: VerifyingKey,
    key_path: PathBuf,
}

impl SovereignIdentity {
    pub fn new() -> Result<Self> {
        let key_path = PathBuf::from("data/agency_identity.pem");
        
        let keypair = if key_path.exists() {
            info!("🔐 Sovereignty: Loading existing identity...");
            let pem = fs::read_to_string(&key_path)?;
            let bytes = BASE64.decode(pem.trim()).context("Failed to decode identity key")?;
            SigningKey::from_bytes(bytes.as_slice().try_into()?)
        } else {
            info!("🔐 Sovereignty: Generating NEW unique identity...");
            let mut csprng = OsRng;
            let key = SigningKey::generate(&mut csprng);
            
            // Persist the key
            let bytes = key.to_bytes();
            let pem = BASE64.encode(bytes);
            fs::write(&key_path, pem)?;
            
            key
        };

        let public_key = VerifyingKey::from(&keypair);
        info!("🔑 Agency Public ID: {}", hex::encode(public_key.as_bytes()));

        Ok(Self {
            keypair,
            public_key,
            key_path,
        })
    }

    /// Sign a message (bytes) to prove authorship
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.keypair.sign(message)
    }

    /// Get the public key as hex string
    pub fn public_id(&self) -> String {
        hex::encode(self.public_key.as_bytes())
    }

    /// Verify a signature from another agent
    pub fn verify(public_key_hex: &str, message: &[u8], signature_bytes: &[u8]) -> Result<bool> {
        let pk_bytes = hex::decode(public_key_hex)?;
        let pk = VerifyingKey::from_bytes(pk_bytes.as_slice().try_into()?)?;
        let sig = Signature::from_bytes(signature_bytes.try_into()?);
        
        Ok(pk.verify_strict(message, &sig).is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_lifecycle() {
        let identity = SovereignIdentity::new().expect("Failed to create identity");
        let pub_id = identity.public_id();
        assert_eq!(pub_id.len(), 64); // Ed25519 hex is 64 chars

        let message = b"I am sovereign";
        let sig = identity.sign(message);
        
        let valid = SovereignIdentity::verify(&pub_id, message, &sig.to_bytes()).expect("Verification failed");
        assert!(valid);
    }

    #[test]
    fn test_signature_rejection() {
        let identity = SovereignIdentity::new().expect("Failed to create identity");
        let pub_id = identity.public_id();
        let message = b"Real message";
        let sig = identity.sign(message);
        
        // Tamper with message
        let valid = SovereignIdentity::verify(&pub_id, b"Fake message", &sig.to_bytes()).unwrap();
        assert!(!valid, "Should reject invalid message");
    }
}
