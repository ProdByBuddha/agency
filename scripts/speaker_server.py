from dotenv import load_dotenv
from pathlib import Path
import os
import gc
import sys

# Load .env file
env_path = Path(__file__).parent / ".env"
load_dotenv(dotenv_path=env_path)

token = os.getenv("HF_TOKEN")
if token:
    os.environ["HF_TOKEN"] = token

from fastapi import FastAPI, BackgroundTasks
from pydantic import BaseModel
from chatterbox.tts_turbo import ChatterboxTurboTTS
from chatterbox.models.t3.t3 import T3
import torch
import torch.nn.functional as F
import soundfile as sf
import io
import base64
import uvicorn
import warnings
from filelock import FileLock

# Suppress warnings
os.environ["TQDM_DISABLE"] = "1"
warnings.filterwarnings("ignore")

# ULTRA-CONSERVATIVE: 1 thread to prevent memory allocation spikes
torch.set_num_threads(1)

# --- TURBO MONKEYPATCH START ---
def t3_inference_turbo_optimized(self, t3_cond, text_tokens, temperature=0.1, top_k=50, top_p=0.95, repetition_penalty=1.2, max_gen_len=1500):
    print(f"DEBUG: Starting inference with max_gen_len={max_gen_len}, text_tokens shape={text_tokens.shape}")
    from transformers.generation.logits_process import (
        LogitsProcessorList, TemperatureLogitsWarper, TopKLogitsWarper, TopPLogitsWarper, RepetitionPenaltyLogitsProcessor
    )
    
    logits_processors = LogitsProcessorList()
    if temperature > 0 and temperature != 1.0:
        logits_processors.append(TemperatureLogitsWarper(temperature))
    if top_k > 0:
        logits_processors.append(TopKLogitsWarper(top_k))
    if top_p < 1.0:
        logits_processors.append(TopPLogitsWarper(top_p))
    if repetition_penalty != 1.0:
        logits_processors.append(RepetitionPenaltyLogitsProcessor(repetition_penalty))

    speech_start_token = self.hp.start_speech_token * torch.ones_like(text_tokens[:, :1])
    embeds, _ = self.prepare_input_embeds(t3_cond=t3_cond, text_tokens=text_tokens, speech_tokens=speech_start_token, cfg_weight=0.0)

    generated_speech_tokens = []
    llm_outputs = self.tfmr(inputs_embeds=embeds, use_cache=True)
    past_key_values = llm_outputs.past_key_values
    speech_logits = self.speech_head(llm_outputs[0][:, -1:])

    processed_logits = logits_processors(speech_start_token, speech_logits[:, -1, :])
    next_speech_token = torch.multinomial(F.softmax(processed_logits, dim=-1), num_samples=1)
    generated_speech_tokens.append(next_speech_token)
    current_speech_token = next_speech_token

    # Use the model's actual stop token
    STOP_TOKEN = self.hp.stop_speech_token

    for i in range(max_gen_len):
        # Use cache to save RAM and time
        llm_outputs = self.tfmr(inputs_embeds=self.speech_emb(current_speech_token), past_key_values=past_key_values, use_cache=True)
        past_key_values = llm_outputs.past_key_values
        speech_logits = self.speech_head(llm_outputs[0])
        
        input_ids = torch.cat(generated_speech_tokens, dim=1)
        processed_logits = logits_processors(input_ids, speech_logits[:, -1, :])
        
        next_speech_token = torch.multinomial(F.softmax(processed_logits, dim=-1), num_samples=1)
        
        token_val = next_speech_token.item()
        if token_val == STOP_TOKEN:
            print(f"DEBUG: Hit STOP_TOKEN at step {i}")
            break
            
        generated_speech_tokens.append(next_speech_token)
        current_speech_token = next_speech_token
    else:
        print(f"DEBUG: Hit max_gen_len ({max_gen_len})")

    # Clear memory immediately after loop
    del past_key_values
    return torch.cat(generated_speech_tokens, dim=1)

T3.inference_turbo = t3_inference_turbo_optimized
# --- TURBO MONKEYPATCH END ---

HW_LOCK_FILE = "/tmp/agency_hw.lock"
lock = FileLock(HW_LOCK_FILE)

app = FastAPI()

class SpeakRequest(BaseModel):
    text: str

device = "mps" if torch.backends.mps.is_available() else "cpu"
print(f"Loading Chatterbox Turbo model on {device}...")
sys.stdout.flush()
cb = ChatterboxTurboTTS.from_pretrained(device=device)
print(f"Turbo model loaded. (Thread Capped)")
sys.stdout.flush()

import re
import numpy as np

def split_text(text):
    # Split by sentence boundaries and significant pauses
    # This regex looks for punctuation followed by space or end of string
    chunks = re.split(r'(?<=[.!,?;])\s+', text)
    return [c.strip() for c in chunks if c.strip()]

@app.post("/speak")
async def speak(request: SpeakRequest):
    # Sanitize: Remove markdown and meta-talk
    text = request.text.replace("## Response", "").replace("##", "").replace("**", "").strip()
    if text.startswith("The system will") or text.startswith("Conversation"):
        text = "Hello. How can I help you?"

    print(f"Synthesizing: {text}")
    sys.stdout.flush()
    
    chunks = split_text(text)
    if not chunks:
        return {"status": "error", "message": "No text to synthesize"}
        
    print(f"DEBUG: Split into {len(chunks)} chunks: {chunks}")
    sys.stdout.flush()

    all_wav_data = []
    
    try:
        with lock:
            for i, chunk in enumerate(chunks):
                print(f"DEBUG: Processing chunk {i+1}/{len(chunks)}: {chunk}")
                sys.stdout.flush()
                gc.collect()
                with torch.no_grad():
                    # Generate with slightly more conservative settings if needed
                    wav_tensor = cb.generate(chunk, top_k=50, temperature=0.1)
                    wav_data = wav_tensor.squeeze().numpy()
                    all_wav_data.append(wav_data)
                    
                    # Add a tiny bit of silence between chunks (0.1s)
                    silence = np.zeros(int(cb.sr * 0.1))
                    all_wav_data.append(silence)
                    
                    del wav_tensor
        
        # Combine all chunks
        combined_wav = np.concatenate(all_wav_data)
        
        buffer = io.BytesIO()
        sf.write(buffer, combined_wav, cb.sr, format='WAV')
        audio_b64 = base64.b64encode(buffer.getvalue()).decode('utf-8')
        
        # Final cleanup
        del all_wav_data
        del combined_wav
        gc.collect()
        
        return {"status": "ok", "text": text, "audio_b64": audio_b64, "sample_rate": cb.sr}
    except Exception as e:
        print(f"Error: {e}")
        sys.stdout.flush()
        return {"status": "error", "message": str(e)}

if __name__ == "__main__":
    uvicorn.run(app, host="0.0.0.0", port=8001, log_level="warning")