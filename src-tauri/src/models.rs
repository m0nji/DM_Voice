use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: String,
    pub filename: String,
    pub size_bytes: u64,
    pub quality: String,
    pub installed: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct CatalogModel {
    pub name: &'static str,
    pub filename: &'static str,
    pub size_bytes: u64,
    pub quality: &'static str,
    pub sha256: &'static str,
    /// Full download URL. Not derivable from `filename`: the German finetune
    /// lives in a different HF repo whose remote filename differs from the
    /// local one we store under.
    pub url: &'static str,
}

pub const MODELS: &[CatalogModel] = &[
    CatalogModel {
        name: "tiny",
        filename: "ggml-tiny.bin",
        size_bytes: 77_691_713,
        quality: "ausreichend",
        sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
    },
    CatalogModel {
        name: "small",
        filename: "ggml-small.bin",
        size_bytes: 487_601_967,
        quality: "gut",
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
    },
    CatalogModel {
        name: "medium",
        filename: "ggml-medium.bin",
        size_bytes: 1_533_763_059,
        quality: "sehr gut",
        sha256: "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin",
    },
    CatalogModel {
        name: "large-v3-turbo",
        filename: "ggml-large-v3-turbo-q5_0.bin",
        size_bytes: 574_041_195,
        quality: "exzellent",
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
    },
    // primeline/whisper-large-v3-turbo-german (Apache 2.0), q5_0-GGML von
    // cstr/whisper-large-v3-turbo-german-ggml. Gleiche Architektur/Größe wie
    // large-v3-turbo, aber auf Deutsch nachtrainiert.
    CatalogModel {
        name: "large-v3-turbo-de",
        filename: "ggml-large-v3-turbo-german-q5_0.bin",
        size_bytes: 574_041_195,
        quality: "exzellent (Deutsch-Finetune)",
        sha256: "15e92e3db0993c52fffa781513eec9253475331c1be808f8fb409285c9d9d030",
        url: "https://huggingface.co/cstr/whisper-large-v3-turbo-german-ggml/resolve/main/ggml-model-q5_0.bin",
    },
    CatalogModel {
        name: "large-v3",
        filename: "ggml-large-v3.bin",
        size_bytes: 3_095_033_483,
        quality: "exzellent",
        sha256: "64d182b440b98d5203c4f9bd541544d84c605196c4f7b845dfa11fb23594d1e2",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3.bin",
    },
];

pub fn models_dir() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("DM-Voice").join("models")
}

pub fn model_path(filename: &str) -> PathBuf {
    models_dir().join(filename)
}

pub fn catalog_model(filename: &str) -> Option<&'static CatalogModel> {
    MODELS.iter().find(|m| m.filename == filename)
}

pub fn is_known_model_filename(filename: &str) -> bool {
    catalog_model(filename).is_some()
}

fn ensure_known_model_filename(filename: &str) -> Result<()> {
    if is_known_model_filename(filename) {
        Ok(())
    } else {
        anyhow::bail!("unknown model filename: {}", filename)
    }
}

pub fn list_models() -> Vec<ModelInfo> {
    MODELS
        .iter()
        .map(|m| {
            let installed = model_path(m.filename).exists();
            ModelInfo {
                name: m.name.to_string(),
                filename: m.filename.to_string(),
                size_bytes: m.size_bytes,
                quality: m.quality.to_string(),
                installed,
            }
        })
        .collect()
}

pub fn delete_model(filename: &str) -> Result<()> {
    ensure_known_model_filename(filename)?;
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
    ensure_known_model_filename(filename)?;
    let metadata = catalog_model(filename).expect("known model metadata");
    let dir = models_dir();
    std::fs::create_dir_all(&dir)?;
    let response = reqwest::get(metadata.url).await?.error_for_status()?;
    if let Some(content_length) = response.content_length() {
        if content_length != metadata.size_bytes {
            anyhow::bail!(
                "unexpected model size for {}: got {} bytes, expected {}",
                filename,
                content_length,
                metadata.size_bytes
            );
        }
    }
    let mut downloaded: u64 = 0;
    let tmp_path = dir.join(format!("{}.tmp", filename));
    let final_path = dir.join(filename);
    let mut file = tokio::fs::File::create(&tmp_path).await?;
    let mut hasher = Sha256::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        hasher.update(&chunk);
        downloaded += chunk.len() as u64;
        if downloaded > metadata.size_bytes {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            anyhow::bail!(
                "model download exceeded expected size for {}: got more than {} bytes",
                filename,
                metadata.size_bytes
            );
        }
        on_progress(downloaded as f32 / metadata.size_bytes as f32);
    }
    file.flush().await?;
    if downloaded != metadata.size_bytes {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        anyhow::bail!(
            "incomplete model download for {}: got {} bytes, expected {}",
            filename,
            downloaded,
            metadata.size_bytes
        );
    }
    let actual_sha256 = format!("{:x}", hasher.finalize());
    if actual_sha256 != metadata.sha256 {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        anyhow::bail!(
            "model checksum mismatch for {}: got {}, expected {}",
            filename,
            actual_sha256,
            metadata.sha256
        );
    }
    tokio::fs::rename(tmp_path, final_path).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_models_returns_all_six() {
        let models = list_models();
        assert_eq!(models.len(), 6);
    }

    #[test]
    fn large_v3_turbo_is_default_model() {
        let models = list_models();
        let turbo = models.iter().find(|m| m.name == "large-v3-turbo");
        assert!(turbo.is_some());
        assert_eq!(turbo.unwrap().filename, "ggml-large-v3-turbo-q5_0.bin");
    }

    #[test]
    fn only_catalog_model_filenames_are_allowed() {
        assert!(is_known_model_filename("ggml-tiny.bin"));
        assert!(is_known_model_filename("ggml-large-v3-turbo-q5_0.bin"));
        assert!(!is_known_model_filename("../config.toml"));
        assert!(!is_known_model_filename("ggml-tiny.bin.tmp"));
        assert!(!is_known_model_filename("ggml-tiny.bin/evil"));
    }

    #[test]
    fn known_model_metadata_has_exact_size_and_sha256() {
        let tiny = catalog_model("ggml-tiny.bin").unwrap();
        assert_eq!(tiny.size_bytes, 77_691_713);
        assert_eq!(
            tiny.sha256,
            "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21"
        );
    }

    #[test]
    fn german_finetune_downloads_from_its_own_repo() {
        let de = catalog_model("ggml-large-v3-turbo-german-q5_0.bin").unwrap();
        assert_eq!(de.name, "large-v3-turbo-de");
        // Lokaler Dateiname ≠ Remote-Dateiname (Repo liefert ggml-model-q5_0.bin).
        assert_eq!(
            de.url,
            "https://huggingface.co/cstr/whisper-large-v3-turbo-german-ggml/resolve/main/ggml-model-q5_0.bin"
        );
        assert_eq!(de.size_bytes, 574_041_195);
    }

    #[test]
    fn model_names_are_unique_and_dom_id_safe() {
        // Die Namen landen im Frontend als Element-IDs (`model-<name>`) und in
        // Tray-Menü-IDs (`model:<name>`) — keine Leerzeichen, keine Duplikate.
        let mut seen = std::collections::HashSet::new();
        for m in MODELS {
            assert!(seen.insert(m.name), "duplicate model name: {}", m.name);
            assert!(
                m.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.'),
                "model name not id-safe: {}",
                m.name
            );
        }
    }

    #[test]
    fn delete_model_rejects_non_catalog_filename() {
        let err = delete_model("../config.toml").unwrap_err().to_string();
        assert!(
            err.contains("unknown model filename"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn models_dir_is_under_data_dir() {
        let dir = models_dir();
        assert!(dir.to_string_lossy().contains("DM-Voice"));
    }
}
