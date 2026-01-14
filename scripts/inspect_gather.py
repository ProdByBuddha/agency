import onnx
model = onnx.load("artifacts/chatterbox/t3_turbo.onnx")
for node in model.graph.node:
    if node.op_type == "Gather":
        print(f"Node: {node.name}")
        print(f"  Inputs: {node.input}")
        print(f"  Outputs: {node.output}")
