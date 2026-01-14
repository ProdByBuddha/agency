from chatterbox.tts_turbo import ChatterboxTurboTTS
import torch
from dotenv import load_dotenv

load_dotenv()

cb = ChatterboxTurboTTS.from_pretrained(device="cpu")
v = cb.s3gen.mel2wav

print(f"Sampling Rate: {cb.sr}")
print(f"istft_params: {v.istft_params}")
print(f"win_length (attr): {getattr(v, 'win_length', 'N/A')}")
print(f"n_fft (attr): {getattr(v, 'n_fft', 'N/A')}")
