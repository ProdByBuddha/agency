import sys
from chatterbox import ChatterboxTTS
import os
import torch
import soundfile as sf
import tempfile
import subprocess

def main():
    if len(sys.argv) < 2:
        print("Usage: python speak.py <text>")
        return

    text = " ".join(sys.argv[1:])
    
    device = "mps" if torch.backends.mps.is_available() else "cpu"
    print(f"Using device: {device}")

    try:
        # Load the model
        cb = ChatterboxTTS.from_pretrained(device=device)
        
        # Generate audio
        # Note: generate returns a tensor
        wav_tensor = cb.generate(text)
        
        # Convert to numpy
        wav_data = wav_tensor.squeeze().cpu().numpy()
        
        # Save to temporary file
        with tempfile.NamedTemporaryFile(suffix=".wav", delete=False) as tf:
            temp_path = tf.name
        
        sf.write(temp_path, wav_data, cb.sr)
        
        # Play the audio using system player (afplay on Mac)
        subprocess.run(["afplay", temp_path])
        
        # Cleanup
        os.remove(temp_path)

    except Exception as e:
        print(f"Error: {e}")

if __name__ == "__main__":
    main()