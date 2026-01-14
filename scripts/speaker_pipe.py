import sys
import torch
import gc
import base64
import io
import soundfile as sf
from chatterbox.tts_turbo import ChatterboxTurboTTS
import warnings

# Suppress all warnings for clean stdout
warnings.filterwarnings("ignore")

# Force MPS if available
device = "mps" if torch.backends.mps.is_available() else "cpu"
cb = ChatterboxTurboTTS.from_pretrained(device=device)

def speak(text):
    try:
        with torch.no_grad():
            wav_tensor = cb.generate(text, top_k=50, temperature=0.1)
            wav_data = wav_tensor.squeeze().cpu().numpy()
        
        buffer = io.BytesIO()
        sf.write(buffer, wav_data, cb.sr, format='WAV')
        audio_b64 = base64.b64encode(buffer.getvalue()).decode('utf-8')
        
        # Protocol: Send back the base64 string on one line
        print(f"AUDIO:{audio_b64}")
        sys.stdout.flush()
        
        del wav_tensor
        del wav_data
        gc.collect()
    except Exception as e:
        print(f"ERROR:{str(e)}")
        sys.stdout.flush()

if __name__ == "__main__":
    # Signal readiness
    print("READY")
    sys.stdout.flush()
    
    for line in sys.stdin:
        text = line.strip()
        if not text:
            continue
        if text == "EXIT":
            break
        speak(text)
