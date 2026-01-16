//! Wallet Tool (Multi-Chain)
//! 
//! Allows agents to interact with their multi-chain Economic Metabolism.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use crate::agent::{AgentResult, AgentError};
use crate::tools::{Tool, ToolOutput};
use crate::orchestrator::metabolism::{EconomicMetabolism, TransactionCategory, Network};

pub struct WalletTool {
    metabolism: Arc<EconomicMetabolism>,
}

impl WalletTool {
    pub fn new(metabolism: Arc<EconomicMetabolism>) -> Self {
        Self { metabolism }
    }
}

#[async_trait]
impl Tool for WalletTool {
    fn name(&self) -> String {
        "agency_wallet".to_string()
    }

    fn description(&self) -> String {
        "Access the Agency's multi-chain economic ledger. Supports Bitcoin, Ethereum, and Solana.".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["check_balance", "record_expense"],
                    "description": "The action to perform."
                },
                "network": {
                    "type": "string",
                    "enum": ["bitcoin", "ethereum", "solana", "base", "worldchain"],
                    "default": "bitcoin",
                    "description": "The blockchain network."
                },
                "amount": {
                    "type": "string",
                    "description": "Amount to spend."
                },
                "reason": {
                    "type": "string",
                    "description": "Reason for the expense."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value) -> AgentResult<ToolOutput> {
        let action = params["action"].as_str().unwrap_or("check_balance");
        let network_str = params["network"].as_str().unwrap_or("bitcoin");
        
        let network = match network_str {
            "bitcoin" => Network::Bitcoin,
            "ethereum" => Network::Ethereum,
            "solana" => Network::Solana,
            "base" => Network::Base,
            "worldchain" => Network::Worldchain,
            _ => return Ok(ToolOutput::failure(format!("Unsupported network: {}", network_str))),
        };

        match action {
            "check_balance" => {
                match self.metabolism.get_balance(network.clone()).await {
                    Ok(balance) => Ok(ToolOutput::success(
                        json!({ "network": network_str, "balance": balance }),
                        format!("Current {:?} Balance: {}", network, balance)
                    )),
                    Err(e) => Ok(ToolOutput::failure(format!("Wallet error: {}", e))),
                }
            },
            "record_expense" => {
                let amount = params["amount"].as_str().ok_or_else(|| AgentError::Validation("Missing amount".to_string()))?;
                let reason = params["reason"].as_str().unwrap_or("Unspecified labor");
                
                match self.metabolism.spend(network, amount, reason, TransactionCategory::SwarmLabor).await {
                    Ok(tx_id) => Ok(ToolOutput::success(
                        json!({"status": "paid", "tx_id": tx_id}), 
                        format!("Recorded expense of {} on {:?} for: {}", amount, network_str, reason)
                    )),
                    Err(e) => Ok(ToolOutput::failure(format!("Wallet error: {}", e))),
                }
            },
            _ => Ok(ToolOutput::failure(format!("Unsupported wallet action: {}", action))),
        }
    }
}