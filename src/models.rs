use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BriefSong {
    pub provider: String,
    pub identifier: String,
    pub title: String,
    pub album_name: String,
    pub artists_name: String,
    pub duration_ms: String,
}
