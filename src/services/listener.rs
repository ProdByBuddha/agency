use anyhow::{Context, Result};
use candle_core::{Device, Tensor, IndexOp};
use candle_transformers::models::whisper::{self as m, audio, Config};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use reqwest::Client;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokenizers::Tokenizer;
use tracing::{info, error, debug};

// Configuration
const WHISPER_MODEL_ID: &str = "lmz/candle-whisper";
const WHISPER_REVISION: &str = "main";
const NEXUS_URL: &str = "http://localhost:8002/v1/chat/completions";
const SAMPLE_RATE: usize = 16000;
const VAD_THRESHOLD: f32 = 0.015;
const SILENCE_DURATION_MS: u64 = 800;

pub enum WhisperModel {
    Quantized(m::quantized_model::Whisper),
}

pub struct ListenerState {
    model: Arc<Mutex<WhisperModel>>,
    tokenizer: Arc<Tokenizer>,
    config: Arc<Config>,
    mel_filters: Arc<Vec<f32>>,
    client: Client,
    device: Device,
}

pub async fn run_listener_server() -> Result<()> {
    info!("👂 Starting Integrated Listener Server...");

    let device = Device::Cpu; 

    // 1. Load Whisper Model
    let api = hf_hub::api::sync::Api::new()?;
    let repo = api.repo(hf_hub::Repo::with_revision(
        WHISPER_MODEL_ID.to_string(),
        hf_hub::RepoType::Model,
        WHISPER_REVISION.to_string(),
    ));

    let config_filename = repo.get("config-tiny-en.json")?;
    let tokenizer_filename = repo.get("tokenizer-tiny-en.json")?;
    let weights_filename = repo.get("model-tiny-en-q80.gguf")?;

    let config: Config = serde_json::from_str(&std::fs::read_to_string(config_filename)?)?;
    let tokenizer = Tokenizer::from_file(tokenizer_filename).map_err(anyhow::Error::msg)?;
    
    let vb = candle_transformers::quantized_var_builder::VarBuilder::from_gguf(&weights_filename, &device)?;
    let model = WhisperModel::Quantized(m::quantized_model::Whisper::load(&vb, config.clone())?);
    
    let mel_bytes = include_bytes!("../../candle/candle-examples/examples/whisper/melfilters.bytes").as_slice();
    let mut mel_filters = vec![0f32; mel_bytes.len() / 4];
    <byteorder::LittleEndian as byteorder::ByteOrder>::read_f32_into(mel_bytes, &mut mel_filters);

    let state = Arc::new(ListenerState {
        model: Arc::new(Mutex::new(model)),
        tokenizer: Arc::new(tokenizer),
        config: Arc::new(config),
        mel_filters: Arc::new(mel_filters),
        client: Client::new(),
        device,
    });

    // 2. Setup Audio Input
    let host = cpal::default_host();
    let audio_device = host.default_input_device().context("No audio input device found")?;
    let audio_config = audio_device.default_input_config()?;
    let channels = audio_config.channels() as usize;
    let in_sample_rate = audio_config.sample_rate().0 as usize;

    info!("🎤 Recording from: {} ({}Hz, {} channels)", audio_device.name()?, in_sample_rate, channels);

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<f32>>();
    
    let _stream = audio_device.build_input_stream(
        &audio_config.config(),
        move |pcm: &[f32], _: &cpal::InputCallbackInfo| {
            let mono_pcm = pcm.iter().step_by(channels).copied().collect::<Vec<f32>>();
            let _ = tx.send(mono_pcm);
        },
        |err| error!("Audio stream error: {}", err),
        None
    )?;
    _stream.play()?;

    // 3. Processing Loop
    let mut buffered_pcm = Vec::new();
    let mut last_activity = std::time::Instant::now();
    let mut is_speaking = false;

    info!("🚀 Listener ready. Voice-to-Nexus active.");

    while let Some(pcm) = rx.recv().await {
        let max_amp = pcm.iter().map(|&x| x.abs()).fold(0.0f32, f32::max);
        
        if max_amp > VAD_THRESHOLD {
            if !is_speaking {
                debug!("🎙️  Speech detected");
                is_speaking = true;
            }
            last_activity = std::time::Instant::now();
            buffered_pcm.extend_from_slice(&pcm);
        } else if is_speaking {
            buffered_pcm.extend_from_slice(&pcm);
            if last_activity.elapsed().as_millis() > SILENCE_DURATION_MS as u128 {
                let speech_data = std::mem::take(&mut buffered_pcm);
                let state_c = state.clone();
                
                tokio::spawn(async move {
                    if let Err(e) = process_speech(speech_data, in_sample_rate, state_c).await {
                        error!("Error processing speech: {}", e);
                    }
                });
                
                is_speaking = false;
            }
        }
    }

    Ok(())
}

async fn process_speech(pcm: Vec<f32>, in_sample_rate: usize, state: Arc<ListenerState>) -> Result<()> {
    let pcm_16k = if in_sample_rate != SAMPLE_RATE {
        tokio::task::spawn_blocking(move || resample(&pcm, in_sample_rate, SAMPLE_RATE)).await??
    } else {
        pcm
    };

    let value = state.clone();
    let text = tokio::task::spawn_blocking(move || {
        let mut model_lock = value.model.blocking_lock();
        transcribe_sync(
            &mut model_lock,
            &value.tokenizer,
            &value.config,
            &value.mel_filters,
            &pcm_16k,
            &value.device
        )
    }).await??;

    let text = text.trim();
    if text.is_empty() || text.len() < 2 {
        return Ok (());
    }

    info!("💬 Transcribed: \"{}\"", text);

    let _ = state.client.post(NEXUS_URL)
        .json(&json!({
            "messages": [{"role": "user", "content": text}],
            "stream": true 
        }))
        .send()
        .await;

    Ok(())
}

fn transcribe_sync(
    model: &mut WhisperModel,
    tokenizer: &Tokenizer,
    config: &Config,
    mel_filters: &[f32],
    pcm: &[f32],
    device: &Device
) -> Result<String> {
    let mel = audio::pcm_to_mel(config, pcm, mel_filters);
    let mel_len = mel.len();
    let mel_t = Tensor::from_vec(
        mel,
        (1, config.num_mel_bins, mel_len / config.num_mel_bins),
        device,
    )?;

    let sot_token = tokenizer.token_to_id(m::SOT_TOKEN).context("SOT missing")?;
    let eot_token = tokenizer.token_to_id(m::EOT_TOKEN).context("EOT missing")?;

    let mut tokens = vec![sot_token]; 
    let whisper = match model { WhisperModel::Quantized(w) => w };
    let audio_features = whisper.encoder.forward(&mel_t, true)?;
    
    for i in 0..config.max_target_positions / 2 {
        let tokens_t = Tensor::new(&tokens[..], device)?.unsqueeze(0)?;
        let ys = whisper.decoder.forward(&tokens_t, &audio_features, i == 0)?;
        let (_, seq_len, _) = ys.dims3()?;
        let logits = whisper.decoder.final_linear(&ys.i((..1, seq_len - 1..))?)?.i(0)?.i(0)?;
        let logits_v = logits.to_vec1::<f32>()?;
        let next_token = logits_v.iter().enumerate()
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(idx, _)| idx as u32)
            .unwrap();

        if next_token == eot_token { break; }
        tokens.push(next_token);
    }

    Ok(tokenizer.decode(&tokens, true).map_err(anyhow::Error::msg)?)
}

fn resample(input: &[f32], from: usize, to: usize) -> Result<Vec<f32>> {
    use rubato::Resampler;
    let ratio = to as f64 / from as f64;
    let mut resampler = rubato::FastFixedIn::<f32>::new(
        ratio,
        2.0,
        rubato::PolynomialDegree::Septic,
        input.len(),
        1,
    )?;
    let resampled = resampler.process(&[input], None)?;
    Ok(resampled[0].clone())
}