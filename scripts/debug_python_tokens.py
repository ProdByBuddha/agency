import torch
from chatterbox.tts_turbo import ChatterboxTurboTTS
from dotenv import load_dotenv
load_dotenv()

cb = ChatterboxTurboTTS.from_pretrained(device="cpu")
text = "Hi there!"
print(f"Generating for: {text}")

# Get text tokens
tokens = cb.tokenizer.encode(text)
if hasattr(tokens, 'ids'):
    text_tokens = torch.tensor([tokens.ids]).long()
else:
    text_tokens = torch.tensor([tokens]).long()

# Manual T3 loop to see tokens
t3_cond = cb.conds.t3
hp = cb.t3.hp
speech_start_token = hp.start_speech_token * torch.ones_like(text_tokens[:, :1])

# Use the monkeypatch logic
from transformers.generation.logits_process import LogitsProcessorList, RepetitionPenaltyLogitsProcessor
logits_processors = LogitsProcessorList()
logits_processors.append(RepetitionPenaltyLogitsProcessor(1.2))

speech_tokens = [speech_start_token]
past_key_values = None

print(f"Start token: {hp.start_speech_token}")
print(f"Stop token: {hp.stop_speech_token}")

with torch.no_grad():
    # Initial prefill
    embeds, _ = cb.t3.prepare_input_embeds(t3_cond=t3_cond, text_tokens=text_tokens, speech_tokens=speech_start_token)
    outputs = cb.t3.tfmr(inputs_embeds=embeds, use_cache=True)
    past_key_values = outputs.past_key_values
    logits = cb.t3.speech_head(outputs.last_hidden_state[:, -1:])
    
    for i in range(20):
        # Apply processors
        processed_logits = logits_processors(torch.cat(speech_tokens, dim=1), logits[:, -1, :])
        next_token = torch.argmax(processed_logits, dim=-1)
        print(f"  Step {i}: token={next_token.item()}")
        if next_token.item() == hp.stop_speech_token:
            print("  Hit stop token!")
            break
        speech_tokens.append(next_token.unsqueeze(0))
        
        # Next step
        inputs = cb.t3.speech_emb(next_token.unsqueeze(0))
        outputs = cb.t3.tfmr(inputs_embeds=inputs, past_key_values=past_key_values, use_cache=True)
        past_key_values = outputs.past_key_values
        logits = cb.t3.speech_head(outputs.last_hidden_state)
