import onnx

def analyze_model(path):
    model = onnx.load(path)
    graph = model.graph
    
    print(f"📊 Analyzing model: {path}")
    print(f"Inputs: {[i.name for i in graph.input]}")
    print(f"Outputs: {[o.name for o in graph.output]}")
    
    op_counts = {}
    for node in graph.node:
        op_counts[node.op_type] = op_counts.get(node.op_type, 0) + 1
    
    print("\nOp Counts:")
    for op, count in sorted(op_counts.items(), key=lambda x: x[1], reverse=True):
        print(f"  {op}: {count}")

    print("\nGraph Structure (Top 50 nodes):")
    for i in range(min(50, len(graph.node))):
        node = graph.node[i]
        print(f"  {node.op_type}: {node.name} (Inputs: {node.input}, Outputs: {node.output})")

if __name__ == "__main__":
    analyze_model("rust_agency/artifacts/chatterbox/conditional_decoder.onnx")
