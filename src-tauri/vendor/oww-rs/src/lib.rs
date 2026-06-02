use crate::model::{Detection, Model};
#[cfg(feature = "mic")]
use crate::mic::converters::i16_to_f32;
use hound::{SampleFormat, WavReader};
use log::info;
use std::env;
use std::error::Error;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use tract_core::internal::{Graph, RunnableModel, TypedFact, TypedOp};

pub mod config;
pub mod info;
mod model;
pub mod oww;
pub mod rms;
pub mod save;
mod tests;

#[cfg(feature = "mic")]
pub mod mic;
pub mod chunk;

pub const VOICE_SAMPLE_RATE: usize = 16000;
pub const BUFFER_SECS: usize = 4;

pub const RMS_BUFFER_SIZE: usize = 16; // 1 secs+ buffer size

type ModelType = RunnableModel<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>;

pub struct Models {
    pub(crate) model1: Box<dyn Model>,
    pub(crate) model2: Box<dyn Model>,
}

impl Models {
    pub(crate) fn new(model1: Box<dyn Model>, model2: Box<dyn Model>) -> Self {
        Models { model1, model2 }
    }
    pub(crate) fn frame_length(&self) -> usize {
        self.model1.frame_length() as usize
    }

    pub fn detect1(&mut self, data: Vec<f32>) -> Option<Detection> {
        self.model1.detect(data)
    }

    pub fn detect2(&mut self, data: Vec<f32>) -> Option<Detection> {
        self.model2.detect(data)
    }

    pub fn detect1_i16(&mut self, data: Vec<i16>) -> Option<Detection> {
        self.model1.detect_i16(data)
    }

    pub fn detect2_i16(&mut self, data: Vec<i16>) -> Option<Detection> {
        self.model2.detect_i16(data)
    }
}

#[cfg(feature = "mic")]
pub fn create_unlock_task_sync(
    running: tokio_util::sync::CancellationToken,
    chunks_sender: tokio::sync::broadcast::Sender<crate::chunk::ChunkType>,
) -> Result<bool, String> {
    use crate::config::UnlockConfig;
    use crate::mic::mic_cpal::MicHandlerCpal;
    use crate::model::new_model;
    use log::{debug, warn};
    use std::sync::{Arc, Mutex};
    use std::thread::sleep;
    use std::time::Duration;

    let running2 = running.clone();
    let mut mic_failing = false;
    while !running2.is_cancelled() {
        let config = UnlockConfig::default(); // load_config(config_file_name.clone());
        let model1 = new_model(config.clone());
        let model2 = new_model(config.clone());

        let models = match (model1, model2) {
            (Ok(model1), Ok(model2)) => (model1, model2),
            _ => {
                panic!("Unable to create unlock model");
            }
        };

        let mic_loop = MicHandlerCpal::new(
            Arc::new(Mutex::new(Models::new(models.0, models.1))),
            &config,
            chunks_sender.clone(),
        );

        match mic_loop {
            Ok(mut mic) => {
                mic_failing = false;
                if let Err(e) = mic.loop_now_sync(running.clone()) {
                    warn!("Mic loop error {:?}. Reloading mic loop", e)
                }
                debug!("Mic loop successful");
            }
            Err(e) => {
                if !mic_failing {
                    warn!("Mic init error {:?}", e);
                    mic_failing = true;
                } else {
                    debug!("Mic err loop successful {:?}", e);
                }
            }
        }
        sleep(Duration::from_secs(1));
    }
    Ok(false)
}

/// loads all wav file as Vec<f32> with conversion from Int format as well
pub fn load_wav(filename: &str) -> Result<Vec<f32>, Box<dyn Error>> {
    let based_dir = env!("CARGO_MANIFEST_DIR");
    let path = Path::new(based_dir).join(filename);
    info!("Reading file {:?}", &path);
    let reader: WavReader<BufReader<File>> = hound::WavReader::open(path)?;

    match reader.spec().sample_format {
        SampleFormat::Float => match load_wav_f32(reader) {
            Ok(d) => Ok(d),
            Err(e) => Err(e),
        },
        SampleFormat::Int => {
            load_wav_i16(reader).map(|d| d.iter().map(|s| *s as f32 / 32768.0).collect())
        }
    }
}

/// loads all wav file as vec<i16>
fn load_wav_i16(mut reader: WavReader<BufReader<File>>) -> Result<Vec<i16>, Box<dyn Error>> {
    let mut data = vec![];
    for s in reader.samples::<i16>() {
        data.push(s.unwrap());
    }
    Ok(data)
}

/// loads all wav file as vec<f32>
fn load_wav_f32(mut reader: WavReader<BufReader<File>>) -> Result<Vec<f32>, Box<dyn Error>> {
    let mut data = vec![];
    for s in reader.samples::<f32>() {
        data.push(s.unwrap());
    }
    Ok(data)
}

pub fn get_exec_dir() -> PathBuf {
    let exec_dir = match env::current_exe() {
        Ok(exe) => match exe.parent() {
            None => {
                panic!("No exec directory found");
            }
            Some(p) => p.to_path_buf(),
        },
        Err(e) => {
            panic!("No exec directory found, error {:?}", e);
        }
    };
    exec_dir
}
