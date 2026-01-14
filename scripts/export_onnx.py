import torch
import torch.nn as nn
from chatterbox.tts_turbo import ChatterboxTurboTTS
import os
import json
from dotenv import load_dotenv

# Load .env file
load_dotenv()

class TransformerWrapper(nn.Module):
    def __init__(self, model):
        super().__init__()
        self.model = model
        
    def forward(self, inputs_embeds):
        # Ensure we pass inputs_embeds as a keyword argument
        return self.model(inputs_embeds=inputs_embeds)[0]

def export():
    artifact_dir = "artifacts/chatterbox"
    os.makedirs(artifact_dir, exist_ok=True)
    
    print("🚀 Loading Chatterbox Turbo...")
    cb = ChatterboxTurboTTS.from_pretrained(device="cpu")
    
    # Save the tokenizer and config for Rust use
    print("📦 Saving metadata...")
    # Tokenizer is already in the cache, we'll copy it manually or assume it's there
    
    # 1. Export T3 Core
    print("🔥 Exporting T3 Transformer...")
    dummy_text_embeds = torch.randn(1, 10, 1024) # GPT2 Medium hidden size
    
    # Wrap the model to handle argument mapping
    wrapper = TransformerWrapper(cb.t3.tfmr)
    
    torch.onnx.export(
        wrapper,
        (dummy_text_embeds,),
        os.path.join(artifact_dir, "t3_turbo.onnx"),
        input_names=["inputs_embeds"],
        output_names=["last_hidden_state"],
        dynamic_axes={"inputs_embeds": {1: "seq_len"}},
        opset_version=17
    )

    # 2. Export S3Gen Vocoder
    print("🌊 Exporting S3Gen Vocoder...")
    
    class S3GenWrapper(nn.Module):
        def __init__(self, model):
            super().__init__()
            self.model = model
            
        def forward(self, speech_tokens, ref_embedding):
            # Use forward which calls flow + vocoder
            # We assume tracing will unroll the flow matching loop
            return self.model.forward(
                speech_tokens, 
                ref_wav=None, 
                ref_sr=None, 
                ref_dict={"embedding": ref_embedding},
                finalize=True
            )

    dummy_tokens = torch.zeros((1, 50), dtype=torch.long)
    dummy_embedding = torch.randn(1, 512)
    
    wrapper_s3 = S3GenWrapper(cb.s3gen)

    try:
        torch.onnx.export(
            wrapper_s3,
            (dummy_tokens, dummy_embedding),
            os.path.join(artifact_dir, "s3gen.onnx"),
            input_names=["speech_tokens", "ref_embedding"],
            output_names=["waveform"],
            dynamic_axes={"speech_tokens": {1: "token_len"}},
            opset_version=17
        )
    except Exception as e:
        print(f"⚠️ S3Gen export failed: {e}. You may need to use the Python bridge for audio.")

    print(f"\n✅ Export process finished. Check {artifact_dir}")

if __name__ == "__main__":
    export()