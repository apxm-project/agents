# LLM-Augmented Compiler Passes for APXM

*Design document: using LLMs inside the compiler, not just the runtime*

---

## The Insight

Traditional compilers have deterministic passes: constant folding, dead code elimination, loop unrolling. The transformations are mechanical — given the same input, you get the same output.

But APXM compiles *agent workflows*. The "code" contains natural language prompts, reasoning chains, and semantic operations. **What if the compiler could use LLMs to optimize these?**

This is not the runtime executing the graph. This is the **compiler** using LLMs during compilation to produce better artifacts.

---

## Three Classes of LLM Compiler Passes

### Class 1: Prompt Optimization Passes

**Idea**: A pass that takes `template_str` fields and optimizes them using DSPy/OPRO-style techniques.

```
Pass: OptimizePrompts
Input: THINK node with template_str = "Solve this problem: {{input}}"
Process: 
  1. Generate N prompt variants using LLM
  2. Evaluate each on sample tasks (from graph metadata or MemoCache)
  3. Select best performer
Output: THINK node with template_str = "Break this into steps. For each step, state your reasoning, then your conclusion. Problem: {{input}}"
```

**Research backing**:
- OPRO (Google, ICLR 2024): LLMs generate and evaluate prompt variants, +8-50% on benchmarks
- DSPy: Compiles signatures to optimized prompts automatically
- Eureka (NVIDIA, ICLR 2024): Evolutionary LLM optimization beats human experts on 83% of tasks

**Implementation sketch**:
```rust
pub struct OptimizePromptsPass {
    optimizer_model: ModelId,      // Small fast model for generation
    evaluator_model: ModelId,      // Can be same or different
    num_candidates: usize,         // How many variants to try
    eval_samples: usize,           // How many test cases
    cache: Arc<MemoCache>,         // Retrieve past executions as training data
}

impl CompilerPass for OptimizePromptsPass {
    fn run(&self, graph: &mut AISGraph) -> Result<bool> {
        let mut changed = false;
        for node in graph.nodes_mut() {
            if let Some(template) = node.get_template_str() {
                // Get historical inputs/outputs from MemoCache for this node type
                let examples = self.cache.get_examples_for_op(node.op_type())?;
                
                // Generate candidate prompts
                let candidates = self.generate_variants(template, &examples)?;
                
                // Evaluate each candidate
                let best = self.evaluate_candidates(&candidates, &examples)?;
                
                if best.score > self.evaluate_single(template, &examples)?.score {
                    node.set_template_str(best.prompt);
                    changed = true;
                }
            }
        }
        Ok(changed)
    }
}
```

**Key insight**: The MemoCache already stores past executions. This is *training data* for the optimizer. The compiler can learn from runtime history.

---

### Class 2: Graph Structure Optimization (Minions/UltraThink)

**Idea**: A pass that analyzes the graph and restructures it for better parallelism or decomposition using an LLM "architect."

```
Pass: DecomposeComplexNodes
Input: Single THINK node with complex task
Process:
  1. LLM analyzes the task semantics
  2. Proposes decomposition into parallel subtasks
  3. Generates replacement subgraph
Output: Multiple THINK nodes with WAIT_ALL + MERGE
```

This is the **ultrathink-coder pattern** applied at compile time:

```
Before (single node):
  THINK("Build a web app with auth, database, and API")

After (decomposed graph):
  THINK("Design database schema") ─┐
  THINK("Design API endpoints")  ──┼─> WAIT_ALL -> MERGE -> THINK("Integrate and refine")
  THINK("Design auth flow")      ─┘
```

**Implementation sketch**:
```rust
pub struct DecomposeComplexPass {
    architect_model: ModelId,      // Strong model for decomposition
    complexity_threshold: f32,     // When to trigger decomposition
    max_parallel_branches: usize,  // Limit fan-out
}

impl CompilerPass for DecomposeComplexPass {
    fn run(&self, graph: &mut AISGraph) -> Result<bool> {
        let mut changed = false;
        
        // Find nodes above complexity threshold
        for node_id in graph.node_ids() {
            let node = graph.get_node(node_id)?;
            let complexity = self.estimate_complexity(node)?;
            
            if complexity > self.complexity_threshold {
                // Ask architect LLM to decompose
                let decomposition = self.architect_decompose(node)?;
                
                // Validate decomposition is sound
                if self.validate_decomposition(&decomposition, node)? {
                    // Replace single node with subgraph
                    graph.replace_with_subgraph(node_id, decomposition.subgraph)?;
                    changed = true;
                }
            }
        }
        Ok(changed)
    }
    
    fn architect_decompose(&self, node: &AISNode) -> Result<Decomposition> {
        let prompt = format!(
            "Analyze this task and decompose into parallel subtasks that can be merged:\n\
             Task: {}\n\n\
             Output JSON with: {{\n\
               \"subtasks\": [{{\"id\": int, \"task\": str, \"depends_on\": [int]}}],\n\
               \"merge_strategy\": str\n\
             }}",
            node.get_template_str().unwrap_or_default()
        );
        
        // Call architect model
        let response = self.architect_model.complete(&prompt)?;
        
        // Parse and build subgraph
        Decomposition::from_json(&response)
    }
}
```

---

### Class 3: Feedback-Driven Optimization (Learning from Execution)

**Idea**: The compiler receives execution traces and uses them to optimize future compilations.

```
Pass: LearnFromExecution
Input: Graph + historical execution traces from ~/.apxm/sessions/
Process:
  1. Identify patterns: which nodes fail often? which are slow?
  2. LLM analyzes failure patterns and proposes fixes
  3. Generate optimized graph with guards, retries, or restructuring
Output: Hardened graph with learned optimizations
```

**This closes the loop**: Runtime → Traces → Compiler → Better Graph → Runtime

```rust
pub struct LearnFromExecutionPass {
    analyzer_model: ModelId,
    trace_dir: PathBuf,
    min_executions: usize,  // Need enough data to learn
}

impl CompilerPass for LearnFromExecutionPass {
    fn run(&self, graph: &mut AISGraph) -> Result<bool> {
        // Load execution traces for this graph
        let traces = self.load_traces_for_graph(graph.id())?;
        
        if traces.len() < self.min_executions {
            return Ok(false);  // Not enough data
        }
        
        // Analyze failure patterns
        let failures = self.extract_failure_patterns(&traces);
        
        // Ask LLM to propose fixes
        let fixes = self.analyze_failures_with_llm(&failures, graph)?;
        
        // Apply fixes: add guards, retries, restructure
        let mut changed = false;
        for fix in fixes {
            match fix {
                Fix::AddGuard { node_id, condition } => {
                    graph.wrap_with_guard(node_id, condition)?;
                    changed = true;
                }
                Fix::AddRetry { node_id, max_attempts } => {
                    graph.wrap_with_try_catch(node_id, max_attempts)?;
                    changed = true;
                }
                Fix::RewritePrompt { node_id, new_prompt } => {
                    graph.get_node_mut(node_id)?.set_template_str(new_prompt);
                    changed = true;
                }
            }
        }
        
        Ok(changed)
    }
}
```

---

## The "Agent is a SkillGraph" Angle

You mentioned this is powerful. Here's how it connects:

**Traditional view**: Agent = LLM + tools + memory
**APXM view**: Agent = compiled skillgraph with typed operations

If the agent IS a skillgraph, then **optimizing the agent = optimizing the graph**. And if the compiler can use LLMs, then:

> **The agent can optimize itself through compilation.**

This is different from runtime self-modification. The optimization happens at compile time, producing a better artifact. The runtime executes deterministically. But the *next* compilation can learn from the *previous* runtime.

```
Compilation 1 → Execution 1 → Traces 1
       ↓                          ↓
Compilation 2 ←─── Learn ────────┘
       ↓
Execution 2 → Traces 2
       ↓
Compilation 3 ←─── Learn ────────┘
       ...
```

This is **evolutionary optimization of skillgraphs**, where each generation is a compile cycle.

---

## Implementation Path

### Phase 1: OptimizePrompts pass
- Integrate DSPy or OPRO as a library
- Use MemoCache as training data source
- Run during `--optimize=aggressive` compilation

### Phase 2: DecomposeComplex pass
- Define complexity heuristics (token count, task breadth)
- Build architect prompt for decomposition
- Validate decomposed graphs before substitution

### Phase 3: LearnFromExecution pass
- Build trace analyzer
- Define fix vocabulary (guards, retries, rewrites)
- Integrate with session history

### Phase 4: Meta-optimization
- The passes themselves become a graph
- Use LLM to decide which passes to run
- Evolutionary optimization of the pass pipeline itself

---

## Research References

- **OPRO** (Google DeepMind, ICLR 2024): LLMs as optimizers, +8-50% prompt gains
  - arxiv:2309.03409
- **Eureka** (NVIDIA, ICLR 2024): Evolutionary reward design, beats humans 83%
  - arxiv:2310.12931
- **DSPy / DSP** (Stanford): Compiling declarative LM programs
  - arxiv:2212.14024, arxiv:2310.03714
- **Buffer of Thoughts** (NeurIPS 2024): Meta-buffer of thought templates, 12% cost
  - arxiv:2406.04271
- **Meta Semi-Formal Reasoning** (March 2026): Structure improves LLM reliability
  - arxiv:2603.01896

---

## The Deeper Question

If MLIR can have LLM-augmented passes, what else changes?

1. **Type inference becomes probabilistic** — the LLM suggests types based on semantic understanding
2. **Optimization becomes learned** — not hand-coded heuristics, but patterns extracted from execution
3. **The compiler becomes an agent** — it reasons about code, not just transforms it

This is the frontier: **compilers that think**.

APXM is positioned to explore this because:
- It already has MLIR infrastructure
- It already has MemoCache (training data)
- It already has execution traces (feedback signal)
- The "code" is semantic (prompts, not just syntax)

The pieces are there. The question is which pass to build first.

---

## Recommendation

Start with **OptimizePrompts** using OPRO-style generation:

1. It's the simplest to implement (single-node transformation)
2. It has clear metrics (task success rate on cached examples)
3. It directly improves runtime quality
4. Research backing is strongest here

Then build **LearnFromExecution** to close the feedback loop.

Save **DecomposeComplex** for when the simpler passes are proven — it's the most architecturally risky (generates new graph structure).
