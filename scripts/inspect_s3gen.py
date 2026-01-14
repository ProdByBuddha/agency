from chatterbox.tts_turbo import ChatterboxTurboTTS
import torch
from dotenv import load_dotenv

load_dotenv()

print("Loading Chatterbox...")
cb = ChatterboxTurboTTS.from_pretrained(device="cpu")
print(f"S3Gen type: {type(cb.s3gen)}")
print(f"S3Gen dir: {dir(cb.s3gen)}")

import inspect
try:
    print("Source of mel2wav.decode:")
    print(inspect.getsource(cb.s3gen.mel2wav.decode))
except:
    print("Could not get source of mel2wav.decode")

try:
    print("Source of inference:")
    print(inspect.getsource(cb.s3gen.inference))
except:
    print("Could not get source of inference")
