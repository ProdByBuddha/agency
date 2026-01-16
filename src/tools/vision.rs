use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use schemars::JsonSchema;
use std::path::PathBuf;
use tokio::sync::Mutex;
use std::sync::Arc;
use std::io::Cursor;

use crate::agent::{AgentResult, AgentError};
use crate::tools::{Tool, ToolOutput};
use screenshots::Screen;
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{CameraIndex, RequestedFormat, RequestedFormatType};
use nokhwa::Camera;
use image::{ImageFormat, DynamicImage};
use candle_core::{Device, Tensor, DType};
use candle_transformers::models::quantized_moondream;
use candle_transformers::generation::LogitsProcessor;
use tokenizers::Tokenizer;

#[derive(Serialize, Deserialize, JsonSchema, Debug)]
pub struct VisionParams {
    /// The action to perform: 'capture_screen', 'capture_camera', or 'describe'.
    pub action: String,
    /// For 'describe', the path to the image or 'last_capture'.
    pub image_source: Option<String>,
    /// For 'describe', the question about the image.
    pub prompt: Option<String>,
    /// Optional display index for screen capture.
    pub display_id: Option<usize>,
}

pub struct VisionTool {
    last_image: Arc<Mutex<Option<PathBuf>>>,
}

impl Default for VisionTool {
    fn default() -> Self {
        Self {
            last_image: Arc::new(Mutex::new(None)),
        }
    }
}

impl VisionTool {
    pub fn new() -> Self {
        Self::default()
    }

    async fn capture_screen(&self, display_id: Option<usize>) -> AgentResult<PathBuf> {
        let screens = Screen::all().map_err(|e| AgentError::Tool(format!("Failed to list screens: {}", e)))?;
        let screen = if let Some(id) = display_id {
            screens.get(id).ok_or_else(|| AgentError::Tool("Display not found".to_string()))? 
        } else {
            screens.first().ok_or_else(|| AgentError::Tool("No screens found".to_string()))?
        };

        let image = screen.capture().map_err(|e| AgentError::Tool(format!("Failed to capture screen: {}", e)))?;
        let mut buffer = Vec::new();
        
        // Convert screenshots::Image to RgbaImage buffer then to DynamicImage
        let rgba_image = image::ImageBuffer::from_raw(image.width(), image.height(), image.into_raw())
            .ok_or_else(|| AgentError::Tool("Failed to create image buffer".to_string()))?;
        let dynamic_image = DynamicImage::ImageRgba8(rgba_image);
        
        dynamic_image.write_to(&mut Cursor::new(&mut buffer), ImageFormat::Png)
            .map_err(|e| AgentError::Tool(format!("Failed to encode image: {}", e)))?;
        
        let path = PathBuf::from("artifacts/last_screen.png");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| AgentError::Io(e))?;
        }
        std::fs::write(&path, buffer).map_err(|e| AgentError::Io(e))?;
        
        let mut last = self.last_image.lock().await;
        *last = Some(path.clone());
        
        Ok(path)
    }

    async fn capture_camera(&self) -> AgentResult<PathBuf> {
        let path = {
            // Simple camera capture using nokhwa
            let index = CameraIndex::Index(0);
            let format = RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestFrameRate);
            let mut camera = Camera::new(index, format).map_err(|e| AgentError::Tool(format!("Failed to open camera: {}", e)))?;
            camera.open_stream().map_err(|e| AgentError::Tool(format!("Failed to open camera stream: {}", e)))?;
            let frame = camera.frame().map_err(|e| AgentError::Tool(format!("Failed to capture frame: {}", e)))?;
            let decoded = frame.decode_image::<RgbFormat>().map_err(|e| AgentError::Tool(format!("Failed to decode frame: {}", e)))?;
            
            let path = PathBuf::from("artifacts/last_camera.png");
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| AgentError::Io(e))?;
            }
            decoded.save(&path).map_err(|e| AgentError::Tool(format!("Failed to save frame: {}", e)))?;
            path
        };
        
        let mut last = self.last_image.lock().await;
        *last = Some(path.clone());
        
        Ok(path)
    }

    async fn describe_image(&self, image_path: PathBuf, prompt: String) -> AgentResult<String> {
        // This is a heavy operation, we'll use Moondream2 via Candle
        let device = if candle_core::utils::metal_is_available() {
            Device::new_metal(0).unwrap_or(Device::Cpu)
        } else {
            Device::Cpu
        };

        // Load model configuration
        let model_file = tokio::task::spawn_blocking(move || {
            use hf_hub::{api::sync::ApiBuilder, Repo};
            let api = ApiBuilder::new().build()?;
            let repo = api.repo(Repo::new("santiagomed/candle-moondream".to_string(), hf_hub::RepoType::Model));
            repo.get("model-q4_0.gguf")
        }).await.map_err(|e| AgentError::Execution(e.to_string()))?
        .map_err(|e| AgentError::Tool(format!("Failed to download model: {}", e)))?;

        let tokenizer_file = tokio::task::spawn_blocking(move || {
            use hf_hub::{api::sync::ApiBuilder, Repo};
            let api = ApiBuilder::new().build()?;
            let repo = api.repo(Repo::new("vikhyatk/moondream2".to_string(), hf_hub::RepoType::Model));
            repo.get("tokenizer.json")
        }).await.map_err(|e| AgentError::Execution(e.to_string()))?
        .map_err(|e| AgentError::Tool(format!("Failed to download tokenizer: {}", e)))?;

        let tokenizer = Tokenizer::from_file(tokenizer_file).map_err(|e| AgentError::Tool(e.to_string()))?;
        let config = candle_transformers::models::moondream::Config::v2();
        
        let vb = candle_transformers::quantized_var_builder::VarBuilder::from_gguf(&model_file, &device)
            .map_err(|e| AgentError::Tool(format!("Failed to load model weights: {}", e)))?;
        let mut model = quantized_moondream::Model::new(&config, vb)
            .map_err(|e| AgentError::Tool(format!("Failed to create model: {}", e)))?;

        // Process image
        let img = image::open(image_path).map_err(|e| AgentError::Tool(format!("Failed to open image: {}", e)))?;
        let img = img.resize_to_fill(378, 378, image::imageops::FilterType::Triangle).to_rgb8();
        let data = img.into_raw();
        let data = Tensor::from_vec(data, (378, 378, 3), &Device::Cpu).map_err(|e| AgentError::Tool(e.to_string()))?.permute((2, 0, 1)).map_err(|e| AgentError::Tool(e.to_string()))?;
        let mean = Tensor::new(&[0.5f32, 0.5, 0.5], &Device::Cpu).map_err(|e| AgentError::Tool(e.to_string()))?.reshape((3, 1, 1)).map_err(|e| AgentError::Tool(e.to_string()))?;
        let std = Tensor::new(&[0.5f32, 0.5, 0.5], &Device::Cpu).map_err(|e| AgentError::Tool(e.to_string()))?.reshape((3, 1, 1)).map_err(|e| AgentError::Tool(e.to_string()))?;
        let image_tensor = (data.to_dtype(DType::F32).map_err(|e| AgentError::Tool(e.to_string()))? / 255.).map_err(|e| AgentError::Tool(e.to_string()))?.
            broadcast_sub(&mean).map_err(|e| AgentError::Tool(e.to_string()))?.
            broadcast_div(&std).map_err(|e| AgentError::Tool(e.to_string()))?.
            to_device(&device).map_err(|e| AgentError::Tool(e.to_string()))?.
            unsqueeze(0).map_err(|e| AgentError::Tool(e.to_string()))?;

        let image_embeds = image_tensor.apply(model.vision_encoder()).map_err(|e| AgentError::Tool(e.to_string()))?;

        // Generate response
        let full_prompt = format!("\n\nQuestion: {}\n\nAnswer:", prompt);
        let tokens = tokenizer.encode(full_prompt, true).map_err(|e| AgentError::Tool(e.to_string()))?;
        let mut token_ids = tokens.get_ids().to_vec();
        
        let special_token = *tokenizer.get_vocab(true).get("<|endoftext|>").ok_or_else(|| AgentError::Tool("Missing special token".to_string()))?;
        let mut logits_processor = LogitsProcessor::new(42, Some(0.7), Some(0.9));
        
        let mut result = String::new();
        for index in 0..512 {
            let context_size = if index > 0 { 1 } else { token_ids.len() };
            let ctxt = &token_ids[token_ids.len().saturating_sub(context_size)..];
            let input = Tensor::new(ctxt, &device).map_err(|e| AgentError::Tool(e.to_string()))?.unsqueeze(0).map_err(|e| AgentError::Tool(e.to_string()))?;
            
            let logits = if index > 0 {
                model.text_model.forward(&input).map_err(|e| AgentError::Tool(e.to_string()))?
            } else {
                let bos_token = Tensor::new(&[special_token], &device).map_err(|e| AgentError::Tool(e.to_string()))?.unsqueeze(0).map_err(|e| AgentError::Tool(e.to_string()))?;
                model.text_model.forward_with_img(&bos_token, &input, &image_embeds).map_err(|e| AgentError::Tool(e.to_string()))?
            };
            
            let logits = logits.squeeze(0).map_err(|e| AgentError::Tool(e.to_string()))?.to_dtype(DType::F32).map_err(|e| AgentError::Tool(e.to_string()))?;
            let next_token = logits_processor.sample(&logits).map_err(|e| AgentError::Tool(e.to_string()))?;
            token_ids.push(next_token);
            
            if next_token == special_token || token_ids.ends_with(&[27, 10619, 29]) {
                break;
            }
            
            let token = tokenizer.decode(&[next_token], true).map_err(|e| AgentError::Tool(e.to_string()))?;
            result.push_str(&token);
        }

        Ok(result)
    }
}

#[async_trait]
impl Tool for VisionTool {
    fn name(&self) -> String { "vision".to_string() } 
    
    fn description(&self) -> String {
        "Give the agency eyes. Capture the screen, access the camera, and describe what's being seen using Moondream VLM.".to_string()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["capture_screen", "capture_camera", "describe"],
                    "description": "The vision action to perform."
                },
                "image_source": {
                    "type": "string",
                    "description": "For 'describe', path to image or 'last_capture' (default)."
                },
                "prompt": {
                    "type": "string",
                    "description": "For 'describe', what to look for or describe."
                },
                "display_id": {
                    "type": "integer",
                    "description": "Optional display index for screen capture."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: Value) -> AgentResult<ToolOutput> {
        let p: VisionParams = serde_json::from_value(params).map_err(|e| AgentError::Serde(e))?;
        
        match p.action.as_str() {
            "capture_screen" => {
                let path = self.capture_screen(p.display_id).await?;
                Ok(ToolOutput::success(
                    json!({"path": path.to_string_lossy()}),
                    format!("Captured screen to {:?}", path)
                ))
            },
            "capture_camera" => {
                let path = self.capture_camera().await?;
                Ok(ToolOutput::success(
                    json!({"path": path.to_string_lossy()}),
                    format!("Captured camera image to {:?}", path)
                ))
            },
            "describe" => {
                let source = p.image_source.unwrap_or_else(|| "last_capture".to_string());
                let path = if source == "last_capture" {
                    let last = self.last_image.lock().await;
                    last.clone().ok_or_else(|| AgentError::Validation("No image captured yet. Capture screen or camera first.".to_string()))?
                } else {
                    PathBuf::from(source)
                };
                
                let prompt = p.prompt.unwrap_or_else(|| "Describe this image in detail.".to_string());
                let description = self.describe_image(path, prompt).await?;
                
                Ok(ToolOutput::success(
                    json!({"description": description}),
                    format!("Vision Analysis: {}", description)
                ))
            },
            _ => Ok(ToolOutput::failure("Unknown vision action")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vision_params_parsing() {
        let json = json!({
            "action": "describe",
            "prompt": "What do you see?",
            "image_source": "test.png"
        });
        
        let params: VisionParams = serde_json::from_value(json).expect("Failed to parse params");
        assert_eq!(params.action, "describe");
        assert_eq!(params.prompt.unwrap(), "What do you see?");
        assert_eq!(params.image_source.unwrap(), "test.png");
    }
}
