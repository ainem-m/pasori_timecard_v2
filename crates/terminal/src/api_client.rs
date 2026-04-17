use anyhow::Result;
use pasori_core::domain::employee::Employee;
use pasori_core::domain::punch::PunchEvent;
use pasori_core::port::policy::PunchEventType;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CardScannedResponse {
    Registered(Box<RegisteredCardScanResponse>),
    Unregistered { card_id: String },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisteredCardScanResponse {
    pub employee: Employee,
    pub recent_events: Vec<PunchEvent>,
    pub suggested_type: PunchEventType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitPunchRequest {
    pub punch_id: Uuid,
    pub card_id: String,
    pub event_type: PunchEventType,
    pub occurred_at: jiff::Zoned,
}

#[derive(Clone)]
pub struct ApiClient {
    base_url: String,
    client: reqwest::Client,
}

impl ApiClient {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            client: reqwest::Client::new(),
        }
    }

    pub async fn health_check(&self) -> Result<jiff::Zoned> {
        let resp = self
            .client
            .get(format!("{}/health", self.base_url))
            .send()
            .await?;
        
        let time_str = resp
            .headers()
            .get("Server-Time")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| anyhow::anyhow!("missing Server-Time header"))?;
        
        Ok(time_str.parse()?)
    }

    pub async fn resolve_card(&self, card_id: &str) -> Result<CardScannedResponse> {
        let resp = self
            .client
            .get(format!("{}/terminals/me/card_scanned", self.base_url))
            .query(&[("card_id", card_id)])
            .send()
            .await?;
        
        Ok(resp.json().await?)
    }

    pub async fn submit_punch(&self, req: SubmitPunchRequest) -> Result<PunchEvent> {
        let resp = self
            .client
            .post(format!("{}/terminals/me/punches", self.base_url))
            .json(&req)
            .send()
            .await?;
        
        if !resp.status().is_success() {
            return Err(anyhow::anyhow!("API error: {}", resp.status()));
        }
        
        Ok(resp.json().await?)
    }
}
