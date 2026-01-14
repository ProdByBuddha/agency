import torch
import torch.nn as nn
from chatterbox.tts_turbo import ChatterboxTurboTTS
import os
from dotenv import load_dotenv

load_dotenv()

def export_parts():
    artifact_dir = "artifacts/chatterbox"
    os.makedirs(artifact_dir, exist_ok=True)
    
    print("🚀 Loading Chatterbox...")
    cb = ChatterboxTurboTTS.from_pretrained(device="cpu")
    flow = cb.s3gen.flow
    
    # 1. Flow Encoder (Tokens -> mu)
    print("📦 Exporting Flow Encoder...")
    class FlowEncoderWrapper(nn.Module):
        def __init__(self, flow):
            super().__init__()
            self.input_embedding = flow.input_embedding
            self.encoder = flow.encoder
            self.encoder_proj = flow.encoder_proj
        def forward(self, tokens):
            x = self.input_embedding(tokens)
            # Use a dummy mask [1, 1, seq_len]
            mask = torch.ones((1, 1, tokens.size(-1)), dtype=torch.bool, device=tokens.device)
            out = self.encoder(x, mask)
            if isinstance(out, tuple):
                x = out[0]
            else:
                x = out
            x = self.encoder_proj(x)
            return x.transpose(1, 2) # [B, 80, T]

    flow_enc = FlowEncoderWrapper(flow)
    dummy_tokens = torch.zeros((1, 50), dtype=torch.long)
    torch.onnx.export(
        flow_enc, (dummy_tokens,),
        os.path.join(artifact_dir, "s3_flow_encoder.onnx"),
        input_names=["tokens"], output_names=["mu"],
        dynamic_axes={"tokens": {1: "seq_len"}, "mu": {2: "seq_len"}},
        opset_version=17
    )

    # 2. Speaker Affine
    print("📦 Exporting Speaker Affine...")
    torch.onnx.export(
        flow.spk_embed_affine_layer, (torch.randn(1, 192),),
        os.path.join(artifact_dir, "s3_spk_affine.onnx"),
        input_names=["speaker_emb"], output_names=["spk_projected"],
        opset_version=17
    )

    # 3. Flow Estimator (The Core UNet)
    print("📦 Exporting Flow Estimator...")
    # We need to wrap it to fix the optional arguments issue
    class EstimatorWrapper(nn.Module):
        def __init__(self, estimator):
            super().__init__()
            self.estimator = estimator
        def forward(self, x, mu, t, spks):
            # x: [1, 80, T], mu: [1, 80, T], t: [1], spks: [1, 80]
            # Create mask on the fly
            mask = torch.ones((1, 1, x.size(-1)), device=x.device)
            # Explicitly pass all positional args to avoid NoneType issues in tracing
            # signature: forward(self, x, mask, mu, t, spks=None, cond=None, r=None)
            # meanflow is False in Turbo, so r is ignored. cond is also often None.
            return self.estimator(x, mask, mu, t, spks, None, None)

    estimator_wrap = EstimatorWrapper(flow.decoder.estimator)
    dummy_x = torch.randn(1, 80, 50)
    dummy_mu = torch.randn(1, 80, 50)
    dummy_t = torch.tensor([0.5])
    dummy_spks = torch.randn(1, 80)
    
    torch.onnx.export(
        estimator_wrap, (dummy_x, dummy_mu, dummy_t, dummy_spks),
        os.path.join(artifact_dir, "s3_flow_estimator.onnx"),
        input_names=["x", "mu", "t", "spks"], output_names=["velocity"],
        dynamic_axes={"x": {2: "seq_len"}, "mu": {2: "seq_len"}, "velocity": {2: "seq_len"}},
        opset_version=17
    )

    print("✅ S3Gen Parts Exported successfully.")

if __name__ == "__main__":
    export_parts()
