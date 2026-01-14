import onnx
model = onnx.load("artifacts/chatterbox/t3_turbo.onnx")
print("Nodes:")
for node in model.graph.node[:10]:
    print(f"  {node.name} ({node.op_type})")
