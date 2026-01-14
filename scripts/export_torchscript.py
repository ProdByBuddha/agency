import torch
from chatterbox.tts_turbo import ChatterboxTurboTTS
import os
from dotenv import load_dotenv

load_dotenv()

def export_torchscript():
    artifact_dir = "artifacts/chatterbox"
    os.makedirs(artifact_dir, exist_ok=True)
    
    print("🚀 Loading Chatterbox...")
    cb = ChatterboxTurboTTS.from_pretrained(device="cpu")
    
    print("🔥 Jitting T3...")
    # T3 is easy to jit trace
    dummy_text = torch.randint(0, 100, (1, 10))
    # We need to wrap it because generate() has complex logic
    # But we can jit the core transformer
    try:
        traced_t3 = torch.jit.trace(cb.t3, (dummy_text,), check_trace=False)
        traced_t3.save(os.path.join(artifact_dir, "t3.pt"))
        print("✅ T3 TorchScript saved.")
    except Exception as e:
        print(f"❌ T3 JIT failed: {e}")

    print("🌊 Jitting S3Gen...")
    # S3Gen inference
    dummy_tokens = torch.randint(0, 100, (1, 50))
    # S3Gen needs ref_wav or ref_dict. We can use the default ones in cb
    try:
        # We trace the inference method
        # We wrap it to handle the ref_dict correctly
        class S3GenJitWrapper(torch.nn.Module):
            def __init__(self, s3gen, conds):
                super().__init__()
                self.s3gen = s3gen
                self.ref_dict = conds.gen
            def forward(self, tokens):
                # tokens -> wav
                # We use the internal inference to avoid dict issues in tracing if possible
                # or just call it
                wav, _ = self.s3gen.inference(tokens, ref_dict=self.ref_dict)
                return wav

        s3jit = S3GenJitWrapper(cb.s3gen, cb.conds)
        # Trace it
        traced_s3 = torch.jit.trace(s3jit, (dummy_tokens,), check_trace=False)
        traced_s3.save(os.path.join(artifact_dir, "s3gen.pt"))
        print("✅ S3Gen TorchScript saved.")
    except Exception as e:
        print(f"❌ S3Gen JIT failed: {e}")

if __name__ == "__main__":
    export_torchscript()