use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: String,
    pub filename: String,
    pub size_bytes: u64,
    pub quality: String,
    pub installed: bool,
}

pub const MODELS: &[(&str, &str, u64, &str)] = &[
    ("tiny",           "ggml-tiny.bin",                   75_000_000,   "ausreichend"),
    ("small",          "ggml-small.bin",                 244_000_000,   "gut"),
    ("medium",         "ggml-medium.bin",                769_000_000,   "sehr gut"),
    ("large-v3-turbo", "ggml-large-v3-turbo-q5_0.bin",  874_000_000,   "exzellent"),
    ("large-v3",       "ggml-large-v3.bin",            1_500_000_000,   "exzellent"),
];

const BASE_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

pub fn models_dir() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("DM-Voice").join("models")
}

pub fn model_path(filename: &str) -> PathBuf {
    models_dir().join(filename)
}

pub fn list_models() -> Vec<ModelInfo> {
    MODELS.iter().map(|(name, filename, size, quality)| {
        let installed = model_path(filename).exists();
        ModelInfo {
            name: name.to_string(),
            filename: filename.to_string(),
            size_bytes: *size,
            quality: quality.to_string(),
            installed,
        }
    }).collect()
}

pub fn delete_model(filename: &str) -> Result<()> {
    let path = model_path(filename);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

pub async fn download_model<F>(filename: &str, mut on_progress: F) -> Result<()>
where
    F: FnMut(f32) + Send + 'static,
{
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    let dir = models_dir();
    std::fs::create_dir_all(&dir)?;
    let url = format!("{}/{}", BASE_URL, filename);
    let response = reqwest::get(&url).await?;
    let total = response.content_length().unwrap_or(1);
    let mut downloaded: u64 = 0;
    let tmp_path = dir.join(format!("{}.tmp", filename));
    let final_path = dir.join(filename);
    let mut file = tokio::fs::File::create(&tmp_path).await?;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        on_progress(downloaded as f32 / total as f32);
    }
    file.flush().await?;
    tokio::fs::rename(tmp_path, final_path).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_models_returns_all_five() {
        let models = list_models();
        assert_eq!(models.len(), 5);
    }

    #[test]
    fn large_v3_turbo_is_default_model() {
        let models = list_models();
        let turbo = models.iter().find(|m| m.name == "large-v3-turbo");
        assert!(turbo.is_some());
        assert_eq!(turbo.unwrap().filename, "ggml-large-v3-turbo-q5_0.bin");
    }

    #[test]
    fn models_dir_is_under_data_dir() {
        let dir = models_dir();
        assert!(dir.to_string_lossy().contains("DM-Voice"));
    }
}
