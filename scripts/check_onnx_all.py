import onnxruntime as ort
sess = ort.InferenceSession("artifacts/chatterbox/t3_turbo.onnx")
print("Inputs:")
for i in sess.get_inputs():
    print(f"  Name: {i.name}, Shape: {i.shape}, Type: {i.type}")
