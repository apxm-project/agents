import { GraphBuilder } from "@apxm/frontend";

const graph = new GraphBuilder("main", { metadata: { is_entry: true } });
graph.done(graph.awaitInput({ name: "input" }), "return_input");
console.log(JSON.stringify(graph.toDict()));
