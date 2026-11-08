use crate::secure::{Store, invalid};
use serde::{Deserialize, Serialize};
use std::io;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Starting,
    Recording,
    Processing,
    Saved,
    Cancelled,
    Failed,
    Rejected,
}
impl Status {
    pub(crate) fn active(&self) -> bool {
        matches!(self, Self::Starting | Self::Recording | Self::Processing)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub version: u32,
    pub request_id: String,
    pub bundle_id: String,
    pub status: Status,
    pub bundle_path: Option<std::path::PathBuf>,
    pub error: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    pub receipt: Receipt,
    pub token: String,
    pub control: Option<crate::Control>,
}
impl Record {
    pub fn read(store: &Store, id: &str) -> io::Result<Option<Self>> {
        let Some(bytes) = store.read(&format!("request-{id}.json"))? else {
            return Ok(None);
        };
        let record: Self = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if record.receipt.version != 1 || record.receipt.request_id != id {
            return Err(invalid());
        }
        crate::identity::validate_bundle(&record.receipt.bundle_id)?;
        crate::identity::validate(&record.token, 64)?;
        if (record.receipt.status == Status::Saved) != record.receipt.bundle_path.is_some() {
            return Err(invalid());
        }
        Ok(Some(record))
    }
    pub fn write(&self, store: &Store) -> io::Result<()> {
        store.write(
            &format!("request-{}.json", self.receipt.request_id),
            &serde_json::to_vec(self).map_err(|_| invalid())?,
        )
    }
}
