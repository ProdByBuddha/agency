import torch
from chatterbox.tts_turbo import ChatterboxTurboTTS
from safetensors.torch import save_file
import os
from dotenv import load_dotenv

load_dotenv()

def export_extras():
    artifact_dir = "artifacts/chatterbox"
    os.makedirs(artifact_dir, exist_ok=True)
    
    print("🚀 Loading Chatterbox Turbo...")
    cb = ChatterboxTurboTTS.from_pretrained(device="cpu")
    t3 = cb.t3
    
    tensors = {}
    
    print("📦 Extracting Embeddings...")
    tensors["text_emb.weight"] = t3.text_emb.weight.detach().cpu()
    tensors["speech_emb.weight"] = t3.speech_emb.weight.detach().cpu()
    
    print("📦 Extracting Heads...")
    tensors["speech_head.weight"] = t3.speech_head.weight.detach().cpu()
    if t3.speech_head.bias is not None:
        tensors["speech_head.bias"] = t3.speech_head.bias.detach().cpu()

    print("📦 Extracting CondEnc...")
    tensors["cond_enc.weight"] = t3.cond_enc.spkr_enc.weight.detach().cpu()
    tensors["cond_enc.bias"] = t3.cond_enc.spkr_enc.bias.detach().cpu()
        
    print("📦 Extracting Conds...")
    if cb.conds is not None and cb.conds.t3 is not None:
        if cb.conds.t3.speaker_emb is not None:
            tensors["speaker_emb"] = cb.conds.t3.speaker_emb.detach().cpu()
            print("✅ Captured speaker_emb")
    
    print(f"Start Speech Token: {cb.t3.hp.start_speech_token}")
    print(f"Stop Speech Token: {cb.t3.hp.stop_speech_token}")
    
    save_path = os.path.join(artifact_dir, "t3_extras.safetensors")
    save_file(tensors, save_path)
    print(f"✅ Saved extras to {save_path}")

if __name__ == "__main__":
    export_extras()