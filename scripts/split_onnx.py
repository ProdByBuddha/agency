import onnx
import os

def split_t3():
    model_path = "rust_agency/artifacts/chatterbox/t3_turbo.onnx"
    if not os.path.exists(model_path):
        print(f"Model not found at {model_path}")
        return

    print("✂️ Splitting T3 Turbo into 2 parts...")
    model = onnx.load(model_path)
    
    # Part 1: layers 0-11
    # Inputs: inputs_embeds + past_key_values.0-11.key/value (Total: 1 + 24 = 25)
    # Outputs: hidden_states_12 + present.0-11.key/value
    
    # Part 2: layers 12-23
    # Inputs: hidden_states_12 + past_key_values.12-23.key/value (Total: 1 + 24 = 25)
    # Outputs: last_hidden_state + present.12-23.key/value

    # This requires detailed knowledge of the node names which is hard to automate perfectly without a tool.
    # However, since we are doing this to fix a Segfault, let's try a different approach:
    # Reduce the input count by combining KV caches into a single tensor if possible? No, ONNX doesn't like that easily.
    
    # What if we just fix the Segfault in ONNX Runtime itself since we have the source?
    # Or use the split-model strategy but implement it in the export script.
    pass

if __name__ == "__main__":
    split_t3()