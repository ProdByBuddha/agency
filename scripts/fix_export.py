import torch
import torch.nn as nn
from chatterbox.tts_turbo import ChatterboxTurboTTS
import os
from dotenv import load_dotenv

# Load environment variables from .env
load_dotenv()
hf_token = os.getenv("HF_TOKEN")

def export_fixed_vocoder():
    artifact_dir = "artifacts/chatterbox"
    os.makedirs(artifact_dir, exist_ok=True)
    
    if hf_token:
        print(f"🚀 Setting HF_TOKEN... {hf_token[:5]}***")
        os.environ["HF_TOKEN"] = hf_token
        
    # Standard practice: Export on CPU to avoid MPS-specific float64/node issues
    device = "cpu"
    print(f"🚀 Loading Chatterbox on {device} for stable export...")
    cb = ChatterboxTurboTTS.from_pretrained(device=device)
    
    print("🌊 Exporting Vocoder Core (Raw Waveform Output)...")
    class VocoderWaveformWrapper(nn.Module):
        def __init__(self, v):
            super().__init__()
            self.v = v
        def forward(self, mel, s_stft):
            # HiFTGenerator.inference returns (wav, source_stft)
            # We want the wav (index 0) which is already reconstructed in the model
            wav, _ = self.v.inference(mel, s_stft)
            return wav

    # Test inputs matching expected shapes
    test_mel = torch.randn(1, 80, 20).to(device)
    test_s_stft = torch.zeros(1, 1, 0).to(device) 

    torch.onnx.export(
        VocoderWaveformWrapper(cb.s3gen.mel2wav), 
        (test_mel, test_s_stft),
        os.path.join(artifact_dir, "s3_vocoder_fixed.onnx"),
        input_names=["mel", "s_stft"], 
        output_names=["waveform"],
        dynamic_axes={
            "mel": {2: "seq_len"}, 
            "s_stft": {2: "stft_len"}, 
            "waveform": {2: "wav_len"}
        },
        opset_version=17,
        do_constant_folding=True
    )

    print("\n✅ Fixed Vocoder exported successfully to s3_vocoder_fixed.onnx")

if __name__ == "__main__":
    export_fixed_vocoder()
