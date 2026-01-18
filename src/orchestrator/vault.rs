//! The Vault: Secure Key Management
//! 
//! Manages the "Apprentice" keys (Hot Wallet) for autonomous low-value spending.
//! Keys are stored encrypted at rest using AES-GCM-256.

use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Nonce, Key
};
use rand::RngCore;
use std::path::PathBuf;
use std::fs;
use anyhow::{Result, Context, anyhow};
use tracing::{info, warn};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
struct VaultData {
    /// The apprentice's EVM private key (32 bytes hex)
    evm_key: String,
    /// The apprentice's Solana private key (64 bytes base58 or array)
    sol_key: String,
}

pub struct AgencyVault {
    file_path: PathBuf,
    // We keep the raw key in memory only while running.
    // In a higher security setting, we might use mlock/secrecy crate.
    data: Option<VaultData>,
}

impl AgencyVault {
    pub fn new() -> Self {
        Self {
            file_path: PathBuf::from("data/agency_vault.enc"),
            data: None,
        }
    }

    /// Unlock the vault using a password
    pub fn unlock(&mut self, password: &str) -> Result<()> {
        if !self.file_path.exists() {
            info!("🔐 Vault: No vault found. Generating new Apprentice keys...");
            self.generate_new(password)?;
            return Ok(());
        }

        info!("🔓 Vault: Attempting to unlock...");
        let encrypted_content = fs::read(&self.file_path)?;
        
        // Extract nonce (first 12 bytes)
        if encrypted_content.len() < 12 {
            return Err(anyhow!("Vault file corrupted (too short)"));
        }
        let (nonce_bytes, ciphertext) = encrypted_content.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);

        // Derive key from password (in prod use Argon2/PBKDF2, here simpler hash for proto)
        let key = Self::derive_key(password);
        let cipher = Aes256Gcm::new(&key);

        let plaintext = cipher.decrypt(nonce, ciphertext)
            .map_err(|_| anyhow!("Failed to unlock vault. Wrong password?"))?;
        
        let data: VaultData = serde_json::from_slice(&plaintext)?;
        self.data = Some(data);
        info!("🔓 Vault: Unlocked successfully. Apprentice active.");
        
        Ok(())
    }

    /// Generate new keys and save encrypted
    fn generate_new(&mut self, password: &str) -> Result<()> {
        // Generate EVM Key (32 bytes)
        let mut evm_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut evm_bytes);
        let evm_key = hex::encode(evm_bytes);

        // Generate Solana Key (32 bytes seed -> 64 bytes keypair)
        // For simplicity in this proto, we just gen 32 bytes seed
        let mut sol_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut sol_bytes);
        let sol_key = bs58::encode(sol_bytes).into_string();

        let data = VaultData { evm_key, sol_key };
        self.data = Some(data);
        self.save(password)
    }

    fn save(&self, password: &str) -> Result<()> {
        if let Some(data) = &self.data {
            let plaintext = serde_json::to_vec(data)?;
            let key = Self::derive_key(password);
            let cipher = Aes256Gcm::new(&key);
            let nonce = Aes256Gcm::generate_nonce(&mut OsRng); // 96-bits; unique per message
            
            let ciphertext = cipher.encrypt(&nonce, plaintext.as_ref())
                .map_err(|e| anyhow!("Encryption failed: {}", e))?;
            
            let mut file_content = nonce.to_vec();
            file_content.extend_from_slice(&ciphertext);
            
            fs::write(&self.file_path, file_content)?;
            info!("🔐 Vault: Apprentice keys saved to disk (Encrypted).");
        }
        Ok(())
    }

    /// Helper to derive a 32-byte key from a string password
    fn derive_key(password: &str) -> Key<Aes256Gcm> {
        // In PROD: Use argon2. Here: SHA256 for speed/demo
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(password.as_bytes());
        let result = hasher.finalize();
        *Key::<Aes256Gcm>::from_slice(&result)
    }

    pub fn get_evm_key(&self) -> Option<String> {
        self.data.as_ref().map(|d| d.evm_key.clone())
    }

    pub fn get_sol_key(&self) -> Option<String> {
        self.data.as_ref().map(|d| d.sol_key.clone())
    }
}
