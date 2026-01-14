import torch
import torch.nn as nn
from chatterbox.tts_turbo import ChatterboxTurboTTS
from safetensors.torch import save_file
import os
from dotenv import load_dotenv

load_dotenv()

def export_core_blocks():
    artifact_dir = "artifacts/chatterbox"
    os.makedirs(artifact_dir, exist_ok=True)
    
    print("🚀 Loading Chatterbox...")
    cb = ChatterboxTurboTTS.from_pretrained(device="cpu")
    
    tensors = {}
    
    # 1. Top-level weights
    print("📦 Extracting top-level weights...")
    tensors["text_emb.weight"] = cb.t3.text_emb.weight.detach().cpu()
    tensors["speech_emb.weight"] = cb.t3.speech_emb.weight.detach().cpu()
    tensors["speech_head.weight"] = cb.t3.speech_head.weight.detach().cpu()
    tensors["speech_head.bias"] = cb.t3.speech_head.bias.detach().cpu()
    
    # 2. Transformer layers
    print("📦 Extracting transformer layers...")
    tfmr = cb.t3.tfmr
    # ln_f
    tensors["ln_f.weight"] = tfmr.ln_f.weight.detach().cpu()
    tensors["ln_f.bias"] = tfmr.ln_f.bias.detach().cpu()
    
    # Blocks
    for i, block in enumerate(tfmr.h):
        prefix = f"h.{i}."
        # ln_1
        tensors[f"{prefix}ln_1.weight"] = block.ln_1.weight.detach().cpu()
        tensors[f"{prefix}ln_1.bias"] = block.ln_1.bias.detach().cpu()
        # attn
        tensors[f"{prefix}attn.c_attn.weight"] = block.attn.c_attn.weight.detach().cpu()
        tensors[f"{prefix}attn.c_attn.bias"] = block.attn.c_attn.bias.detach().cpu()
        tensors[f"{prefix}attn.c_proj.weight"] = block.attn.c_proj.weight.detach().cpu()
        tensors[f"{prefix}attn.c_proj.bias"] = block.attn.c_proj.bias.detach().cpu()
        # ln_2
        tensors[f"{prefix}ln_2.weight"] = block.ln_2.weight.detach().cpu()
        tensors[f"{prefix}ln_2.bias"] = block.ln_2.bias.detach().cpu()
        # mlp
        tensors[f"{prefix}mlp.c_fc.weight"] = block.mlp.c_fc.weight.detach().cpu()
        tensors[f"{prefix}mlp.c_fc.bias"] = block.mlp.c_fc.bias.detach().cpu()
        tensors[f"{prefix}mlp.c_proj.weight"] = block.mlp.c_proj.weight.detach().cpu()
        tensors[f"{prefix}mlp.c_proj.bias"] = block.mlp.c_proj.bias.detach().cpu()

    # 3. Conditioning and Speakers
    print("🎯 Preparing conditioning and speaker embs...")
    t3_cond = cb.conds.t3
    cond_emb = cb.t3.prepare_conditioning(t3_cond)
    tensors["t3_cond_emb"] = cond_emb.detach().cpu()
    tensors["s3_speaker_emb"] = cb.conds.gen["embedding"].detach().cpu()

    # 4. Other models
    # We already have these as ONNX but we can export the encoder if needed
    # For now, let's just make sure we have all T3 weights for Candle
    
    save_file(tensors, os.path.join(artifact_dir, "speaker_weights.safetensors"))
    print(f"\n✅ Weights (including all {len(tfmr.h)} transformer layers) exported successfully to {artifact_dir}/speaker_weights.safetensors")

if __name__ == "__main__":
    export_core_blocks()
