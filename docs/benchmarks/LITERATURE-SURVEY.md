# Literature Survey: LLM Inference Optimization Measurement Methodologies

**Survey Date**: April 8, 2026
**Scope**: Academic and industry research on measuring LLM inference optimization (2023-2026)
**Focus**: Methodologies for evaluating prefix caching, scheduling, prompt optimization, and multi-agent workflows

---

## Executive Summary

This survey reviews recent literature (2023-2026) on LLM inference optimization measurement, covering five key areas:

1. **Prefix Caching / KV-Cache Reuse**: vLLM, SGLang RadixAttention, PagedAttention
2. **Graph-Aware LLM Scheduling**: DAG-based scheduling, priority-aware serving
3. **Prompt Optimization**: DSPy evaluation, prompt compression metrics
4. **Multi-Agent Execution**: Coordination overhead measurement, workflow benchmarks
5. **Best Practices**: Gold standard methodologies and common pitfalls

**Key Finding**: The field has shifted from single-metric optimization (tokens/second) to **multi-dimensional evaluation frameworks** that measure latency, throughput, cache efficiency, SLO attainment, and quality preservation simultaneously.

---

## 1. Prefix Caching / KV-Cache Reuse Measurement

### 1.1 vLLM Prefix Caching

**System Overview**: vLLM implements Automatic Prefix Caching using hash tables for scalability, with LRU-based eviction policies.

**Evaluation Methodology**:
- **Benchmarks**: `benchmark_prefix_caching.py` in vLLM repository
- **Dataset Design**: Fixed random tokens with controlled input/output lengths (1K, 2K, 4K). Dynamic-Sonnet datasets share a common prefix whose length is 1/4 of the input length
- **Controlled Testing**: 256 requests with max_batch_size of 128
- **Comparative Baseline**: Random datasets without shared prefixes to measure computational overhead of prefix caching logic itself
- **Metrics**:
  - Prefill duration (compute-bound vs cache-read)
  - Time to First Token (TTFT) reduction
  - Token hit rate
  - Throughput improvement

**Key Results**:
- Near-zero KV cache memory waste (<4% vs 60-80% in traditional systems)
- 2-4× throughput improvement over FasterTransformer/Orca (Kwon et al., 2023)
- LMSYS cut GPU count by 50% while serving 2-3× more requests/second

**Tool**: [GuideLLM](https://github.com/vllm-project/guidellm) - official benchmarking platform that simulates end-to-end interactions with OpenAI-compatible servers, generating workload patterns reflecting production usage.

**References**:
- [vLLM Benchmarks](https://github.com/vllm-project/vllm/tree/main/benchmarks)
- [Automatic Prefix Caching Documentation](https://docs.vllm.ai/en/stable/design/prefix_caching/)
- [SqueezeBits vLLM vs TensorRT-LLM Analysis (Feb 2025)](https://blog.squeezebits.com/vllm-vs-tensorrtllm-12-automatic-prefix-caching-38189)
- [VAST Data Benchmark Analysis (Jul 2025)](https://www.vastdata.com/blog/accelerating-inference)
- [vLLM Issue #7518: Measuring Prefix Caching Performance](https://github.com/vllm-project/vllm/issues/7518)

### 1.2 SGLang RadixAttention

**System Overview**: RadixAttention retains KV cache for both prompts and generation results in a radix tree, enabling automatic reuse during runtime.

**Evaluation Methodology**:
- **Multi-level sharing tests**:
  - MMLU: Reuse KV cache of 5-shot examples
  - HellaSwag: Two-level sharing (few-shot examples + common question prefix for multiple choices)
- **Ablation study**: Measure overhead in absence of cache hits
- **Metrics**:
  - Throughput gains
  - First token latency reduction
  - Multi-turn conversation efficiency

**Key Results** (Zheng et al., 2024):
- **Up to 5× higher throughput** compared to baseline systems
- **Up to 6.4× throughput** improvement on academic benchmarks (agent control, logical reasoning, few-shot learning, JSON decoding, RAG pipelines, multi-turn chat)
- **10% boost** over vLLM in large multi-turn conversations with cache involvement (2025 comparison)
- **No noticeable overhead** when cache hits are absent (ablation study result)

**References**:
- [SGLang LMSYS Blog (Jan 2024)](https://www.lmsys.org/blog/2024-01-17-sglang/)
- [SGLang Paper (arXiv:2312.07104)](https://arxiv.org/abs/2312.07104)
- [RadixAttention Medium Tutorial](https://medium.com/@dharamendra1314.kumar/sglang-learning-series-part-1-shared-prefix-kv-cache-and-radixattention-d7a847d20b1f)
- [Runpod: SGLang vs vLLM Comparison](https://www.runpod.io/blog/sglang-vs-vllm-kv-cache)

### 1.3 PagedAttention (vLLM Foundation)

**System Overview**: PagedAttention applies virtual memory paging techniques to KV cache management, eliminating fragmentation and redundant duplication.

**Evaluation Methodology**:
- **Memory waste measurement**: Compare actual GPU memory usage vs theoretical requirements
- **Throughput benchmarking**: Same latency constraints, measure tokens/second
- **Fragmentation analysis**: Block-level memory utilization

**Key Results** (Kwon et al., 2023):
- **Memory waste**: Under 4% (vs 60-80% in existing systems)
- **Throughput**: 2-4× improvement over state-of-the-art systems
- **Real-world impact**: LMSYS halved GPU count while serving 2-3× more requests/second

**References**:
- [Efficient Memory Management for LLM Serving with PagedAttention (arXiv:2309.06180)](https://arxiv.org/abs/2309.06180)
- [SOSP 2023 Proceedings](https://dl.acm.org/doi/10.1145/3600006.3613165)
- [Runpod Introduction to vLLM and PagedAttention](https://www.runpod.io/blog/introduction-to-vllm-and-pagedattention)

### 1.4 Advanced Prefix Caching Research (2024-2025)

**Learned Prefix Caching (LPC)** - NeurIPS 2025:
- Leverages conversational content analysis for predictive cache eviction
- Combines content insights with last access timestamps
- Evaluated on LMSys, ShareGPT, and Chatbot-Arena datasets

**MARCONI: Prefix Caching for Hybrid LLMs** (Amazon Science):
- Novel admission/eviction policies based on reuse likelihood forecasts
- **34.4× higher token hit rates** compared to state-of-the-art prefix caching
- Compute savings vs memory footprint trade-off analysis

**KVFlow: Multi-Agent Workflow Optimization** (July 2025):
- Workflow-aware KV cache management for agentic workloads
- Agent Step Graph abstraction
- Steps-to-execution value for temporal usage estimation

**References**:
- [LPC NeurIPS 2025](https://neurips.cc/virtual/2025/poster/117662)
- [MARCONI Paper (Amazon Science)](https://assets.amazon.science/96/d4/ee6df8f84a34b49a71f9c39212f2/marconi-prefix-caching-for-the-era-of-hybrid-llms.pdf)
- [KVFlow Paper (arXiv:2507.07400)](https://arxiv.org/html/2507.07400v1)
- [BentoML Prefix Caching Guide](https://bentoml.com/llm/inference-optimization/prefix-caching)

### 1.5 Multi-Turn Conversation KV Cache Sharing (2024-2025)

**CachedAttention** (USENIX ATC 2024):
- Hierarchical KV caching system for multi-turn conversations
- **Results**:
  - TTFT reduction: **up to 87%**
  - Prompt prefilling throughput: **up to 7.8×**
  - End-to-end inference cost reduction: **up to 70%**

**LMCache** (Oct 2024):
- Supports reuse of KV caches for repeated input content (not just prefixes)
- Cross-instance KV cache sharing
- **Results**: 3×–10× latency reductions when combined with vLLM

**Amazon SageMaker HyperPod** (2025):
- Managed tiered KV cache with intelligent routing
- **Results**:
  - TTFT reduction: **up to 40%**
  - Compute cost reduction: **up to 25%** for long context prompts

**References**:
- [CachedAttention Paper (USENIX ATC 2024)](https://dl.acm.org/doi/10.5555/3691992.3691999)
- [LMCache Paper (arXiv:2510.09665)](https://arxiv.org/pdf/2510.09665)
- [AWS SageMaker Blog](https://aws.amazon.com/blogs/machine-learning/managed-tiered-kv-cache-and-intelligent-routing-for-amazon-sagemaker-hyperpod/)
- [Awesome KV Cache Management Survey](https://github.com/TreeAI-Lab/Awesome-KV-Cache-Management)

---

## 2. Graph-Aware LLM Scheduling

### 2.1 DAG-Based and Workflow-Aware Scheduling

**HEXGEN-TEXT2SQL** (Peng et al., May 2025):
- Hierarchical scheduling for agentic, multi-stage workflows
- **Global workload-balanced dispatch** + **local urgency-guided prioritization**
- Simulation-based hyperparameter tuning
- **Metrics**: End-to-end latency, SLO violations under dependency constraints

**Prompt2DAG** (Sept 2024):
- Multi-step methodology for LLM-based data enrichment pipeline generation
- Structured workflow generation → executable DAG generation
- LLM-driven code synthesis for final DAG generation step

**References**:
- [Efficient LLM Serving for Agentic Workflows (arXiv:2603.16104)](https://arxiv.org/html/2603.16104)
- [Prompt2DAG Paper (arXiv:2509.13487)](https://arxiv.org/html/2509.13487v1)
- [LLM Inference Scheduling Survey](https://www.techrxiv.org/users/994660/articles/1355915)

### 2.2 Priority-Based Scheduling

**Mixed-Priority Workloads**:
- **High priority (LS)**: Latency-sensitive requests with SLO guarantees
- **Low priority (BE)**: Best-effort requests
- **Problem**: Current systems are priority-oblivious, causing LS delays

**Hierarchical Scheduling** (HEXGEN):
- Global workload-balanced dispatch
- Local urgency-guided prioritization
- Simulation-based hyperparameter tuning

**Semantic Scheduling** (June 2025):
- Uses LLMs to analyze request urgency and importance
- Content-aware approach beyond latency-based scheduling
- Stage-aware continuous batching prevents priority inversions

**Learning-to-Rank (LTR)** (NeurIPS 2024):
- Ranks requests based on output size
- Prioritizes requests with fewer remaining tokens

**References**:
- [Priority-Aware Preemptive Scheduling for MoE (arXiv:2503.09304)](https://arxiv.org/html/2503.09304)
- [Semantic Scheduling Paper (arXiv:2506.12204)](https://arxiv.org/html/2506.12204)
- [LTR Paper (NeurIPS 2024)](https://proceedings.neurips.cc/paper_files/paper/2024/file/6c8985579293e0209bdaa4f21bb1d237-Paper-Conference.pdf)

### 2.3 Cache-Aware Scheduling

**llm-d Intelligent Scheduling**:
- **Scorers**: Optimize for KV cache locality (boost prefix-cache hit rates)
- **Disaggregated scheduling**: Multiple passes to separate prefill and decode phases onto specialized pod variants
- **Predicted latency-based routing**: Lightweight ML model trained online from live traffic

**Multi-Stage Flow Scheduling (MFS)** (May 2025):
- Holistic multi-stage communication scheduler
- Translates global TTFT deadline into explicit flow-level deadlines
- Maximizes TTFT SLO attainment

**References**:
- [llm-d Intelligent Inference Scheduling](https://llm-d.ai/blog/intelligent-inference-scheduling-with-llm-d)
- [Predicted-Latency Based Scheduling](https://llm-d.ai/blog/predicted-latency-based-scheduling-for-llms)
- [Multi-Stage Flow Scheduling (arXiv:2603.17456)](https://arxiv.org/html/2603.17456)

### 2.4 State-Aware Scheduling

**Astraea** (Dec 2025):
- State-aware multi-level feedback queue (Stateful-MLFQ) algorithm
- Dynamically adjusts priorities
- Extends optimization from individual segments to entire lifecycle

**References**:
- [Astraea Paper (arXiv:2512.14142)](https://www.arxiv.org/pdf/2512.14142)

---

## 3. Prompt Optimization Measurement

### 3.1 DSPy Evaluation Methodology

**What is DSPy**: Framework for programming—not prompting—language models. Separates "what should the LM do?" from "how do we tell it to do that?"

**Evaluation Approach**:
- **Metrics**: Functions that evaluate program output and assign scores (higher is better)
- **Data Requirements**: Training inputs can be very small (5-10 examples), possibly incomplete (only inputs, no labels)
- **Training/Dev Split**: Training set for optimization, dev set for evaluation

**MIPROv2 Optimizer**:
1. **Bootstrapping stage**: Run program many times across different inputs to collect traces
2. **Filtering**: Keep only high-scoring trajectories (based on metric)
3. **Instruction synthesis**: Draft many potential instructions for every prompt
4. **Evaluation**: Test candidate programs on mini-batches

**Key Results**:
- Prompt evaluation criterion task: **46.2% → 64.0% accuracy**
- ReAct score: **24% → 51%** (teaching gpt-4o-mini task specifics)

**References**:
- [DSPy Optimizers Documentation](https://dspy.ai/learn/optimization/optimizers/)
- [Multi-Use Case Study (arXiv:2507.03620)](https://arxiv.org/html/2507.03620v1)
- [Systematic LLM Prompt Engineering with DSPy](https://towardsdatascience.com/systematic-llm-prompt-engineering-using-dspy-optimization/)
- [GitHub: DSPy](https://github.com/stanfordnlp/dspy)

### 3.2 Prompt Compression & Context Distillation

**LLMLingua Series**:
- **LLMLingua**: 20× compression ratio with minimal performance loss
- **LongLLMLingua**: 17.1% performance improvement with 4× compression
- **LLMLingua-2**: Small BERT-level encoder trained via data distillation from GPT-4

**Evaluation Benchmarks**:
- **GSM8K**: Complex 9-step Chain-of-Thought prompts. Similar performance maintained at **up to 14× compression**
- **LongBench**: Long-context capabilities (4k-18k tokens)
  - Single/multi-document QA: Qasper, MultiFieldQA, NarrativeQA, Musique, HotpotQA, 2WikiMultiQA
  - **Results**: Extractive reranker-based compression achieved **+7.89 F1 points** on 2WikiMultihopQA at 4.5× compression (compression improved accuracy by filtering noise)

**P-Distill** (2025):
- Novel prompt compression via knowledge distillation
- **Peak improvement**: 1.90% even with prompt lengths compressed to **one-eighth**

**References**:
- [LLMLingua Series](https://llmlingua.com/)
- [LLMLingua-2](https://llmlingua.com/llmlingua2.html)
- [Prompt Compression Survey (NAACL 2025)](https://github.com/ZongqianLi/Prompt-Compression-Survey)
- [P-Distill Paper (MDPI 2025)](https://www.mdpi.com/2076-3417/15/5/2420)

### 3.3 Token Efficiency vs Quality Metrics

**Token Efficiency Metrics**:
- **Output token efficiency**: Conciseness while maintaining quality
- **Token usage**: Number of tokens processed (affects cost and context window)
- **Input token efficiency**: Minimal prompt length for desired results
- **Context window utilization**: Percentage of available token capacity used

**Quality Evaluation Metrics**:
- **Answer correctness**: Factually correct based on ground truth
- **Semantic similarity**: Relevance to input
- **Hallucination detection**: Identifies fake or made-up information
- **Answer relevancy**: Informative and concise

**Evaluation Approaches**:
- **LLM-as-a-judge**: Most reliable method using natural language rubrics (requires G-Eval techniques)
- **Avoid traditional scorers**: BLEU/ROUGE don't capture semantic nuance in LLM outputs

**References**:
- [LLM Evaluation Metrics Guide (Confident AI)](https://www.confident-ai.com/blog/llm-evaluation-metrics-everything-you-need-for-llm-evaluation)
- [BentoML: Beyond Tokens-per-Second](https://www.bentoml.com/blog/beyond-tokens-per-second-how-to-balance-speed-cost-and-quality-in-llm-inference)
- [Microsoft: LLM Evaluation Metrics](https://learn.microsoft.com/en-us/ai/playbook/technology-guidance/generative-ai/working-with-llms/evaluation/list-of-eval-metrics)

---

## 4. Multi-Agent Execution Benchmarks

### 4.1 Agent Workflow Benchmarks

**HumanEval**:
- 164 handwritten programming problems
- Measures functional correctness
- **Status**: Saturated (top methods solve >94%)

**SWE-bench Family**:
- **SWE-bench**: 2,294 real GitHub issues from 12 Python repositories
- **SWE-bench Lite/Verified**: High-quality representative issues for faster evaluation
- **Multi-SWE-bench**: Multiple languages (Java, TypeScript, Go, Rust, C, C++)
- **Multimodal SWE-bench**: JavaScript + UI screenshots for cross-modal reasoning
- **SWE-bench Pro**: Enterprise-level complexity
- **SWE-bench-Live/SWE-rebench**: Continuous collection of new issues
- **Status**: Gold standard for autonomous coding agents (2024-2025)

**SWE-EVO** (Dec 2025):
- Benchmarks coding agents in long-horizon software evolution scenarios

**Emerging Benchmarks**:
- **τ-Bench** (June 2024): Long-horizon, tool-enabled conversational workflows with human-in-the-loop
- **SWT-Bench** (Oct 2024): Generate, repair, and execute test suites
- **Context-Bench** (Oct 2025): Maintain, reuse, and reason over long-running context

**References**:
- [SWE-bench Leaderboards](https://www.swebench.com/)
- [SWE-bench Pro Paper (arXiv:2509.16941)](https://arxiv.org/html/2509.16941)
- [SWE-EVO Paper (arXiv:2512.18470)](https://arxiv.org/html/2512.18470v1)
- [Evaluation and Benchmarking of LLM Agents Survey (arXiv:2507.21504)](https://arxiv.org/html/2507.21504v1)

### 4.2 Multi-Agent Coordination Metrics

**MultiAgentBench/MARBLE** (ACL 2025):
- Comprehensive benchmark for LLM-based multi-agent systems
- Measures task completion AND collaboration/competition quality
- **Milestone-based KPIs**

**Key Metrics**:

1. **Coordination Score**: Average of Communication + Planning scores

2. **Communication Quality**:
   - LLM judge rates inter-agent utterances 1-5
   - Evaluates clarity and relevance

3. **Graph-Based Metrics (GEMMAS Framework - 2025)**:
   - **Information Diversity Score (IDS)**: Measures collaboration quality
   - **Unnecessary Path Ratio (UPR)**: Evaluates efficiency
   - Goes beyond final-task accuracy

**Coordination Protocols**:
- Star, chain, tree, and graph topologies
- **Result**: Graph structure performs best in research scenarios

**Performance Results**:
- GPT-4o-mini: 84.13% task scores in research scenarios
- Graph-based coordination: Excels in token usage efficiency
- Supervisor architecture: 50% performance improvement after optimization
- Cognitive planning: 3% improvement in milestone achievement rates

**References**:
- [MultiAgentBench Paper (arXiv:2503.01935)](https://arxiv.org/abs/2503.01935)
- [MultiAgentBench ACL 2025](https://aclanthology.org/2025.acl-long.421/)
- [GEMMAS Framework Paper (EMNLP 2025)](https://aclanthology.org/2025.emnlp-industry.106.pdf)
- [Benchmarking Multi-Agent AI (Galileo)](https://galileo.ai/blog/benchmarks-multi-agent-ai)

### 4.3 Framework Coordination Overhead

**Coordination Overhead Measurements**:
- **CrewAI**: 5× cost of single LangChain agent per task (5 agents × increased LLM calls)
- **CrewAI vs LangGraph**: CrewAI's standalone architecture results in **5.76× faster execution** on 10-step research pipeline
- **LangGraph**: Lower overhead from minimal orchestration layer, excels at complex workflows requiring fine-grained control

**Framework Evolution (2024-2025)**:
- **LangGraph**: Reached v1.0 in late 2024, became default runtime for LangChain agents
- **CrewAI**: 280% increase in adoption in 2025
- **AutoGPT**: Early 2023 experiment that popularized autonomous agents

**References**:
- [Top 5 AI Agent Frameworks 2026](https://www.intuz.com/blog/top-5-ai-agent-frameworks-2025)
- [Best AI Agent Frameworks 2025](https://www.getmaxim.ai/articles/top-5-ai-agent-frameworks-in-2025-a-practical-guide-for-ai-builders/)
- [LangGraph vs CrewAI vs AutoGen Comparison](https://o-mega.ai/articles/langgraph-vs-crewai-vs-autogen-top-10-agent-frameworks-2026)

### 4.4 Multi-Dimensional Assessment Frameworks

**CLEAR Framework** (2024-2025):
Five dimensions of evaluation:
1. **Cost**
2. **Latency**
3. **Efficiency**
4. **Assurance**
5. **Reliability**

**Shift in Evaluation Philosophy**:
- From: Accuracy-focused evaluation
- To: Multi-dimensional assessment
- Emphasis: Realistic scenarios over isolated task completion

**References**:
- [AI Agent Orchestration Analysis](https://medium.com/@josefsosa/ai-agent-orchestration-enterprise-framework-evolution-and-technical-performance-analysis-4463b2c3477d)
- [Evaluating LLM-based Agents Best Practices](https://samiranama.com/posts/Evaluating-LLM-based-Agents-Metrics,-Benchmarks,-and-Best-Practices/)

---

## 5. Best Practices & Gold Standard Methodologies

### 5.1 Common Benchmarking Pitfalls

**1. Evaluation Methodology Issues**:
- **Pitfall**: Micro-benchmarks that don't reflect real workloads
- **Example**: CPU scheduling changes tested with single-threaded loops
- **Fix**: Test with actual model inference runs, varying batch sizes/sequence lengths

**2. Single-Metric Optimization**:
- **Pitfall**: Focusing only on throughput or only on latency
- **Fix**: Measure tail latency (P95/P99) alongside throughput
- **Critical metrics**: Token usage, cache hits, SLO attainment

**3. Data Contamination**:
- **Pitfall**: Training data leakage into evaluation sets
- **Impact**: Inflates performance metrics, unreliable leaderboards, wastes resources
- **Fix**: LiveCodeBench approach - test only on problems released after training cutoff
- **Results**: 20-30% performance drops when models face truly novel problems

**4. Ignoring Tail Latency**:
- **Pitfall**: High throughput doesn't guarantee low latency if tail requests lag
- **Fix**: P99 latency ensures even slowest requests meet SLOs

**5. LLM-as-Judge Biases** (2025 Research):
- **Self-preference bias**: Models favor outputs from their own family
- **Verbosity bias**: Longer responses rated higher regardless of quality
- **Logic error blindness**: LLM judges miss errors that human experts catch
- **Fix**: Use diverse judge models, include human validation

**References**:
- [LLM Inference Optimization Guide (Clarifai)](https://www.clarifai.com/blog/llm-inference-optimization/)
- [Pitfalls of Evaluating Language Models (arXiv:2507.00460)](https://arxiv.org/html/2507.00460)
- [2025 Year in Review for LLM Evaluation (Goodeye Labs)](https://www.goodeyelabs.com/insights/llm-evaluation-2025-review)

### 5.2 Gold Standard Methodologies

**SWE-bench Verified** (Autonomous Coding Agents):
- Hundreds of carefully validated GitHub issues
- Filtered to remove ambiguous or impossible tasks
- Became the gold standard for measuring autonomous coding agents

**LiveCodeBench** (Addressing Memorization):
- Tests models only on problems released after training cutoff dates
- Reveals true generalization capability
- 20-30% performance drops demonstrate measurement of intelligence vs memorization

**FrontierMath & Humanity's Last Exam**:
- Collaborative design with domain experts
- Negative filtering against existing models
- Emphasis on synthesis over retrieval

**Shift to System-Oriented Evaluation**:
- From: Mere measurement of outputs
- To: System-oriented approach focused on outcomes
- Success = outcomes achieved, not just outputs generated

**References**:
- [LLM Evaluation Benchmarks 2025 (Responsible AI Labs)](https://responsibleailabs.ai/knowledge-hub/articles/llm-evaluation-benchmarks-2025)
- [LLM Evaluation Frameworks 2025 vs 2026](https://www.mlaidigital.com/blogs/llm-evaluation-frameworks-2025-vs-2026-what-matters-now-2026)
- [Benchmarking LLMs' Judgments with No Gold Standard (ICLR 2025)](https://arxiv.org/html/2411.07127v1)

### 5.3 vLLM Benchmarking Best Practices

**Official Tools**:
- **Performance Dashboard**: Confirms whether new changes improve/degrade performance
- **GuideLLM**: Official benchmarking platform for real workload simulation
  - End-to-end interaction simulation
  - Complete latency and token-level statistics
  - SLO-driven evaluation

**Key Metrics**:
- **Latency**: Time from request to full response
- **TTFT** (Time to First Token): Critical for user experience
- **TPOT** (Time Per Output Token): Average time to generate each token
- **ITL** (Inter Token Latency): Token-to-token generation time
- **Throughput**: Tokens generated per second

**Configuration Best Practices**:
- **gpu_memory_utilization**: Default 90%, set as high as possible without OOM
- **max-concurrency**: Set to 80-90% of reported maximum for realistic testing
- **Capacity planning**: Test under realistic resource constraints

**Performance Improvements** (v0.6.0):
- 2.7× throughput improvement
- 5× latency reduction

**References**:
- [vLLM Performance Dashboard](https://docs.vllm.ai/en/latest/benchmarking/dashboard/)
- [vLLM 2024 Retrospective and 2025 Vision](https://blog.vllm.ai/2025/01/10/vllm-2024-wrapped-2025-vision.html)
- [vLLM Performance Tuning Guide (Google Cloud)](https://cloud.google.com/blog/topics/developers-practitioners/vllm-performance-tuning-the-ultimate-guide-to-xpu-inference-configuration)
- [Benchmarking vLLM Inference Performance (Medium)](https://medium.com/@kimdoil1211/benchmarking-vllm-inference-performance-measuring-latency-throughput-and-more-1dba830c5444)
- [GuideLLM GitHub](https://github.com/vllm-project/guidellm)

### 5.4 Continuous Batching Evaluation

**Key Metrics**:
- **TTFT target**: Under 200ms
- **TPOT**: More meaningful SLO for user experience than total latency
- **SLO attainment rate**: For online scenarios
- **Service load capacity**: Maximum sustainable throughput
- **Throughput**: For offline workloads
- **Resource utilization**: GPU memory and compute efficiency

**Performance Benchmarks** (2025):
- **vLLM vs Traditional**: 60+ tokens/second (vLLM) vs 15-20 tokens/second (HuggingFace Transformers) on A100 for long-context tasks (**3-4× improvement**)
- **vLLM vs TGI**: 2-24× higher throughput depending on concurrency/model size
- **Memory efficiency**: vLLM uses 19-27% less GPU memory, enables larger batch sizes
- **GPU utilization**: vLLM 85-92% vs TGI 68-74%
- **BucketServe**: Up to 3.58× higher throughput, nearly 2× greater system load capacity
- **Production p99 latency**: 2× better (<500ms) over TensorRT-LLM for variable workloads

**References**:
- [vLLM Continuous Batching Guide (2025)](https://www.johal.in/vllm-continuous-batching-high-throughput-serving-for-long-contexts-2025/)
- [BucketServe Paper (arXiv:2507.17120)](https://arxiv.org/html/2507.17120v1)
- [Comparative Analysis: vLLM vs TGI (arXiv:2511.17593)](https://arxiv.org/html/2511.17593v1)
- [Survey of Efficient LLM Inference Serving (ACL 2025)](https://aclanthology.org/2025.inlg-main.32.pdf)

---

## 6. Emerging Research Areas (2024-2026)

### 6.1 Speculative Decoding

**Technique**: Draft several future tokens efficiently, then verify them in parallel, enabling simultaneous decoding of multiple tokens per step.

**Key Challenges**:
- Fragile and highly variable performance in real-world systems
- Effectiveness depends on workload characteristics, batch sizes, model configurations, system conditions

**Recent Innovations**:

**Online Speculative Decoding (OSD)** - UC Berkeley Dec 2025:
- Improves token acceptance rates on-the-fly
- Reduces inference latency without increasing draft model size

**TurboSpec** - UC Berkeley 2025:
- Closed-loop control system
- **Goodput** as unifying metric: rate of successfully generated tokens
- Dynamic adjustment of speculative parameters at runtime using offline profiling + online feedback

**Performance Results**:
- **Smurfs**: 2.01×/1.86× speedup on OPT-13B, 2.83×/2.55× on Llama2-70B-chat (throughput/latency)
- **Google**: 2×-3× improvements in translation and summarization

**Recent Methods**:
- **Mirror Speculative Decoding** (Dec 2025): Breaks latency-acceptance tradeoff via parallel branch-complete rollouts
- **DART** (Jan 2026): Addresses high drafting latency (>75% of total inference time in vanilla speculative decoding)
- **Distributed Speculative Decoding** (Jan 2026): Speedup compared to standalone LLM inference

**References**:
- [UC Berkeley Tech Report (EECS-2025-224)](https://www2.eecs.berkeley.edu/Pubs/TechRpts/2025/EECS-2025-224.html)
- [Unlocking Efficiency Survey (ACL 2024)](https://aclanthology.org/2024.findings-acl.456/)
- [Mirror Speculative Decoding (Apple ML Research)](https://machinelearning.apple.com/research/mirror)
- [Google: Looking Back at Speculative Decoding](https://research.google/blog/looking-back-at-speculative-decoding/)
- [DART Paper (arXiv:2601.19278)](https://arxiv.org/html/2601.19278v1)

### 6.2 LLM Inference Scheduling Survey Themes

**Control Plane Meta-Scheduling**:
- Managing dynamic interplay between models, compute resources, and inference workloads

**Learning-Based Prediction Methods**:
- Embedding-based scheduling (ICLR 2025)
- Predicted latency-based routing
- Lightweight ML models trained online from live traffic

**Multi-Resource Optimization**:
- Balancing memory, compute, energy, and QoS constraints

**Workflow-Specific Scheduling**:
- Complex agentic and multi-stage LLM applications
- Dependency-aware scheduling for multi-step workflows

**References**:
- [LLM Inference Scheduling Survey (TechRxiv Oct 2025)](https://www.techrxiv.org/users/994660/articles/1355915)
- [Embedding Based Scheduling (ICLR 2025)](https://proceedings.iclr.cc/paper_files/paper/2025/file/9eb8b5ccb0de594a16548f7c058fdadf-Paper-Conference.pdf)
- [Emergent Mind: LLM Inference Scheduling](https://www.emergentmind.com/topics/llm-inference-scheduling)

---

## 7. Synthesis: Recommendations for APXM Benchmark Suite

Based on this literature survey, here are recommendations for APXM's benchmark methodology:

### 7.1 Core Metrics to Measure

**Prefix Caching Efficiency**:
- ✅ **Prefix cache hit rate** (as % of input tokens reused)
- ✅ **TTFT reduction** (compare cache-hit vs cache-miss)
- ✅ **Token savings** (total tokens not recomputed due to caching)
- Methodology: Use datasets with controlled shared prefix ratios (vLLM/SGLang approach)

**Scheduling Effectiveness**:
- ✅ **SLO attainment rate** (% of requests meeting latency target)
- ✅ **Critical path latency** (time to complete longest dependency chain)
- ✅ **Parallelism exploitation** (concurrent node execution ratio)
- ✅ **Priority inversion rate** (low-priority blocking high-priority)

**Multi-Agent Coordination**:
- ✅ **Coordination overhead** (additional LLM calls beyond single-agent baseline)
- ✅ **Token usage efficiency** (tokens per milestone achieved)
- ✅ **Communication quality** (LLM-as-judge rating of inter-agent messages)
- Use MultiAgentBench/MARBLE methodology

**Quality Preservation**:
- ✅ **Task completion rate** (does optimization break functionality?)
- ✅ **Semantic similarity** (optimized vs baseline outputs)
- ✅ **Hallucination rate** (does optimization increase errors?)
- Use LLM-as-judge with diverse judge models to mitigate bias

### 7.2 Gold Standard Methodology

**Dataset Design**:
1. **Controlled synthetic benchmarks**: Fixed input/output lengths, controlled sharing ratios (vLLM approach)
2. **Real-world workload traces**: Multi-turn conversations, agentic workflows (LMSys/ShareGPT datasets)
3. **Adversarial cases**: Stress-test edge cases (no sharing, full sharing, priority conflicts)

**Comparative Baselines**:
- Always compare optimized vs non-optimized (e.g., `-O2` vs `-O0`)
- Include overhead measurements (e.g., prefix caching with no cache hits)
- Report full distributions (P50, P95, P99), not just averages

**Multi-Dimensional Reporting**:
- NEVER report single metric in isolation
- Always include: latency, throughput, cache efficiency, SLO attainment, quality preservation
- Use CLEAR framework dimensions: Cost, Latency, Efficiency, Assurance, Reliability

**Reproducibility**:
- Pin model versions, temperature=0 for determinism where possible
- Report hardware configuration (GPU type, memory, batch size limits)
- Provide session traces for replay (already implemented in APXM!)

### 7.3 What NOT to Do (Common Pitfalls)

❌ **Don't**: Test only average-case scenarios
✅ **Do**: Include tail latency (P95/P99) and adversarial cases

❌ **Don't**: Use micro-benchmarks that don't reflect real workloads
✅ **Do**: Use real multi-turn conversations and agentic workflow patterns

❌ **Don't**: Rely solely on tokens/second
✅ **Do**: Measure TTFT, TPOT, ITL, SLO attainment

❌ **Don't**: Optimize for throughput at expense of latency (or vice versa)
✅ **Do**: Report Pareto frontier of throughput-latency tradeoffs

❌ **Don't**: Use single LLM-as-judge without bias mitigation
✅ **Do**: Use diverse judge models, include human validation samples

❌ **Don't**: Claim optimization wins without measuring quality preservation
✅ **Do**: Always measure task completion rate and semantic similarity

### 7.4 Specific APXM Optimizations to Benchmark

**Fusion Passes**:
- Measure: Token savings (eliminated intermediate prompts), latency reduction
- Quality check: Semantic similarity of fused vs unfused outputs

**Common Subexpression Elimination**:
- Measure: Prefix cache hit rate, duplicate computation eliminated
- Quality check: Output equivalence (should be exact match)

**Dead Context Elimination**:
- Measure: Token savings, memory usage reduction
- Quality check: No degradation in downstream node outputs

**Shared Prefix Detection**:
- Measure: KV cache reuse rate, TTFT reduction for shared-prefix nodes
- Methodology: Vary prefix sharing ratio (0%, 25%, 50%, 75%, 100%)

**Priority-Based Scheduling**:
- Measure: SLO attainment for high-priority nodes, overall makespan
- Quality check: No priority inversions

**Multi-Model Optimization**:
- Measure: Cost savings (small model usage %), quality degradation rate
- Methodology: Define accuracy threshold, measure % of requests meeting threshold with smaller model

### 7.5 Benchmark Suite Structure

Recommended structure based on literature:

```
benchmarks/
├── unit/                    # Isolated feature tests
│   ├── prefix_caching/      # Controlled sharing ratios
│   ├── priority_scheduling/ # SLO-based tests
│   └── fusion/              # Token savings measurement
├── integration/             # Real-world workflow patterns
│   ├── multi_turn_chat/     # Conversational agents
│   ├── agentic_research/    # Multi-step reasoning
│   └── parallel_fanout/     # Concurrent execution
├── stress/                  # Adversarial/edge cases
│   ├── no_cache_hits/       # Overhead measurement
│   ├── priority_conflicts/  # Priority inversion tests
│   └── deep_pipelines/      # Long dependency chains
└── datasets/
    ├── synthetic/           # Controlled synthetic data
    └── real_world/          # LMSys, ShareGPT traces
```

**Key Numbers from Literature** (for calibration):
- vLLM prefix caching: 2-4× throughput improvement, <4% memory waste
- SGLang RadixAttention: 5-6.4× throughput improvement
- CachedAttention: 87% TTFT reduction, 7.8× prefilling throughput
- LLMLingua: 20× compression, minimal quality loss
- Multi-agent coordination overhead: 5× LLM calls (CrewAI 5-agent)
- Continuous batching: 3-4× improvement over traditional serving

---

## 8. Conclusion

The LLM inference optimization field has matured significantly from 2023-2026:

1. **From single-metric to multi-dimensional**: Success requires balancing latency, throughput, cache efficiency, cost, and quality
2. **From micro-benchmarks to system evaluation**: Realistic workloads (multi-turn conversations, agentic workflows) are now standard
3. **From static benchmarks to continuous evaluation**: LiveCodeBench approach addresses memorization vs intelligence
4. **From manual prompting to learned optimization**: DSPy, MIPROv2 demonstrate data-driven prompt improvement
5. **From isolated requests to workflow-aware serving**: KVFlow, HEXGEN, Astraea optimize based on multi-step dependency structure

**For APXM**: The compiler's graph-aware optimizations (fusion, CSE, dead context elimination, prefix detection) align perfectly with cutting-edge research. The key is measuring not just "did we save tokens?" but "did we maintain quality while improving latency, throughput, cache efficiency, and SLO attainment across realistic multi-agent workloads?"

**Next Steps**:
1. Implement multi-dimensional benchmark harness using GuideLLM-style workload simulation
2. Create controlled synthetic benchmarks with varying prefix sharing ratios
3. Collect real-world APXM session traces as evaluation datasets
4. Establish quality preservation baselines (LLM-as-judge + human validation samples)
5. Report Pareto frontiers (throughput-latency-cost-quality tradeoffs) for each optimization

---

## Complete Bibliography

### Prefix Caching & KV-Cache Reuse
1. [vLLM Benchmarks](https://github.com/vllm-project/vllm/tree/main/benchmarks)
2. [vLLM Automatic Prefix Caching Documentation](https://docs.vllm.ai/en/stable/design/prefix_caching/)
3. [SqueezeBits: vLLM vs TensorRT-LLM #12 (Feb 2025)](https://blog.squeezebits.com/vllm-vs-tensorrtllm-12-automatic-prefix-caching-38189)
4. [VAST Data: Accelerating Inference (Jul 2025)](https://www.vastdata.com/blog/accelerating-inference)
5. [vLLM Issue #7518: Measuring Prefix Caching Performance](https://github.com/vllm-project/vllm/issues/7518)
6. [SGLang LMSYS Blog (Jan 2024)](https://www.lmsys.org/blog/2024-01-17-sglang/)
7. [SGLang Paper (arXiv:2312.07104)](https://arxiv.org/abs/2312.07104)
8. [RadixAttention Medium Tutorial](https://medium.com/@dharamendra1314.kumar/sglang-learning-series-part-1-shared-prefix-kv-cache-and-radixattention-d7a847d20b1f)
9. [Runpod: SGLang vs vLLM](https://www.runpod.io/blog/sglang-vs-vllm-kv-cache)
10. [PagedAttention Paper (arXiv:2309.06180)](https://arxiv.org/abs/2309.06180)
11. [PagedAttention SOSP 2023](https://dl.acm.org/doi/10.1145/3600006.3613165)
12. [Runpod: Introduction to vLLM and PagedAttention](https://www.runpod.io/blog/introduction-to-vllm-and-pagedattention)
13. [Learned Prefix Caching (NeurIPS 2025)](https://neurips.cc/virtual/2025/poster/117662)
14. [MARCONI Paper (Amazon Science)](https://assets.amazon.science/96/d4/ee6df8f84a34b49a71f9c39212f2/marconi-prefix-caching-for-the-era-of-hybrid-llms.pdf)
15. [KVFlow Paper (arXiv:2507.07400)](https://arxiv.org/html/2507.07400v1)
16. [BentoML Prefix Caching Guide](https://bentoml.com/llm/inference-optimization/prefix-caching)
17. [CachedAttention (USENIX ATC 2024)](https://dl.acm.org/doi/10.5555/3691992.3691999)
18. [LMCache Paper (arXiv:2510.09665)](https://arxiv.org/pdf/2510.09665)
19. [AWS SageMaker HyperPod Blog](https://aws.amazon.com/blogs/machine-learning/managed-tiered-kv-cache-and-intelligent-routing-for-amazon-sagemaker-hyperpod/)
20. [Awesome KV Cache Management Survey](https://github.com/TreeAI-Lab/Awesome-KV-Cache-Management)

### Graph-Aware Scheduling
21. [Efficient LLM Serving for Agentic Workflows (arXiv:2603.16104)](https://arxiv.org/html/2603.16104)
22. [Prompt2DAG Paper (arXiv:2509.13487)](https://arxiv.org/html/2509.13487v1)
23. [LLM Inference Scheduling Survey (TechRxiv)](https://www.techrxiv.org/users/994660/articles/1355915)
24. [Priority-Aware Preemptive Scheduling (arXiv:2503.09304)](https://arxiv.org/html/2503.09304)
25. [Semantic Scheduling Paper (arXiv:2506.12204)](https://arxiv.org/html/2506.12204)
26. [Learning-to-Rank (NeurIPS 2024)](https://proceedings.neurips.cc/paper_files/paper/2024/file/6c8985579293e0209bdaa4f21bb1d237-Paper-Conference.pdf)
27. [llm-d Intelligent Scheduling](https://llm-d.ai/blog/intelligent-inference-scheduling-with-llm-d)
28. [llm-d Predicted-Latency Scheduling](https://llm-d.ai/blog/predicted-latency-based-scheduling-for-llms)
29. [Multi-Stage Flow Scheduling (arXiv:2603.17456)](https://arxiv.org/html/2603.17456)
30. [Astraea Paper (arXiv:2512.14142)](https://www.arxiv.org/pdf/2512.14142)
31. [Emergent Mind: LLM Inference Scheduling](https://www.emergentmind.com/topics/llm-inference-scheduling)
32. [Embedding Based Scheduling (ICLR 2025)](https://proceedings.iclr.cc/paper_files/paper/2025/file/9eb8b5ccb0de594a16548f7c058fdadf-Paper-Conference.pdf)

### Prompt Optimization
33. [DSPy Optimizers Documentation](https://dspy.ai/learn/optimization/optimizers/)
34. [DSPy Multi-Use Case Study (arXiv:2507.03620)](https://arxiv.org/html/2507.03620v1)
35. [Systematic Prompt Engineering with DSPy](https://towardsdatascience.com/systematic-llm-prompt-engineering-using-dspy-optimization/)
36. [GitHub: DSPy](https://github.com/stanfordnlp/dspy)
37. [LLMLingua Series](https://llmlingua.com/)
38. [LLMLingua-2](https://llmlingua.com/llmlingua2.html)
39. [Prompt Compression Survey (NAACL 2025)](https://github.com/ZongqianLi/Prompt-Compression-Survey)
40. [P-Distill Paper (MDPI 2025)](https://www.mdpi.com/2076-3417/15/5/2420)
41. [LLM Evaluation Metrics (Confident AI)](https://www.confident-ai.com/blog/llm-evaluation-metrics-everything-you-need-for-llm-evaluation)
42. [BentoML: Beyond Tokens-per-Second](https://www.bentoml.com/blog/beyond-tokens-per-second-how-to-balance-speed-cost-and-quality-in-llm-inference)
43. [Microsoft: LLM Evaluation Metrics](https://learn.microsoft.com/en-us/ai/playbook/technology-guidance/generative-ai/working-with-llms/evaluation/list-of-eval-metrics)

### Multi-Agent Systems
44. [SWE-bench Leaderboards](https://www.swebench.com/)
45. [SWE-bench Pro Paper (arXiv:2509.16941)](https://arxiv.org/html/2509.16941)
46. [SWE-EVO Paper (arXiv:2512.18470)](https://arxiv.org/html/2512.18470v1)
47. [LLM Agents Survey (arXiv:2507.21504)](https://arxiv.org/html/2507.21504v1)
48. [MultiAgentBench Paper (arXiv:2503.01935)](https://arxiv.org/abs/2503.01935)
49. [MultiAgentBench ACL 2025](https://aclanthology.org/2025.acl-long.421/)
50. [GEMMAS Framework (EMNLP 2025)](https://aclanthology.org/2025.emnlp-industry.106.pdf)
51. [Benchmarking Multi-Agent AI (Galileo)](https://galileo.ai/blog/benchmarks-multi-agent-ai)
52. [Top 5 AI Agent Frameworks 2026](https://www.intuz.com/blog/top-5-ai-agent-frameworks-2025)
53. [Best AI Agent Frameworks 2025](https://www.getmaxim.ai/articles/top-5-ai-agent-frameworks-in-2025-a-practical-guide-for-ai-builders/)
54. [LangGraph vs CrewAI vs AutoGen](https://o-mega.ai/articles/langgraph-vs-crewai-vs-autogen-top-10-agent-frameworks-2026)
55. [AI Agent Orchestration Analysis](https://medium.com/@josefsosa/ai-agent-orchestration-enterprise-framework-evolution-and-technical-performance-analysis-4463b2c3477d)
56. [Evaluating LLM-based Agents](https://samiranama.com/posts/Evaluating-LLM-based-Agents-Metrics,-Benchmarks,-and-Best-Practices/)

### Best Practices & Evaluation
57. [LLM Inference Optimization (Clarifai)](https://www.clarifai.com/blog/llm-inference-optimization/)
58. [Pitfalls of Evaluating LLMs (arXiv:2507.00460)](https://arxiv.org/html/2507.00460)
59. [2025 Year in Review (Goodeye Labs)](https://www.goodeyelabs.com/insights/llm-evaluation-2025-review)
60. [LLM Evaluation Benchmarks 2025](https://responsibleailabs.ai/knowledge-hub/articles/llm-evaluation-benchmarks-2025)
61. [LLM Evaluation Frameworks 2025 vs 2026](https://www.mlaidigital.com/blogs/llm-evaluation-frameworks-2025-vs-2026-what-matters-now-2026)
62. [Benchmarking LLMs with No Gold Standard (ICLR 2025)](https://arxiv.org/html/2411.07127v1)
63. [vLLM Performance Dashboard](https://docs.vllm.ai/en/latest/benchmarking/dashboard/)
64. [vLLM 2024 Retrospective](https://blog.vllm.ai/2025/01/10/vllm-2024-wrapped-2025-vision.html)
65. [vLLM Performance Tuning (Google Cloud)](https://cloud.google.com/blog/topics/developers-practitioners/vllm-performance-tuning-the-ultimate-guide-to-xpu-inference-configuration)
66. [Benchmarking vLLM (Medium)](https://medium.com/@kimdoil1211/benchmarking-vllm-inference-performance-measuring-latency-throughput-and-more-1dba830c5444)
67. [GuideLLM GitHub](https://github.com/vllm-project/guidellm)
68. [vLLM Continuous Batching (2025)](https://www.johal.in/vllm-continuous-batching-high-throughput-serving-for-long-contexts-2025/)
69. [BucketServe Paper (arXiv:2507.17120)](https://arxiv.org/html/2507.17120v1)
70. [vLLM vs TGI Comparison (arXiv:2511.17593)](https://arxiv.org/html/2511.17593v1)
71. [Survey of Efficient LLM Serving (ACL 2025)](https://aclanthology.org/2025.inlg-main.32.pdf)

### Speculative Decoding
72. [UC Berkeley Efficient LLM System (EECS-2025-224)](https://www2.eecs.berkeley.edu/Pubs/TechRpts/2025/EECS-2025-224.html)
73. [Unlocking Efficiency Survey (ACL 2024)](https://aclanthology.org/2024.findings-acl.456/)
74. [Mirror Speculative Decoding (Apple)](https://machinelearning.apple.com/research/mirror)
75. [Google: Looking Back at Speculative Decoding](https://research.google/blog/looking-back-at-speculative-decoding/)
76. [DART Paper (arXiv:2601.19278)](https://arxiv.org/html/2601.19278v1)

**Total References**: 76 papers, systems, and tools from 2023-2026
