pub mod auth;
pub mod transport;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct SessionRequest {
    pub build: String,
    pub rows: String,
}
