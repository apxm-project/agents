from apxm import GraphBuilder


graph = GraphBuilder("main", metadata={"is_entry": True})
graph.done(graph.await_input(name="input"), "return_input")
