# Token Estimation in Production LLM Systems

**Research Date**: 2026-04-08
**Purpose**: Investigate how production systems estimate tokens to inform APXM's fusion and budget decisions

## Executive Summary

Production LLM systems use a **hybrid approach**: exact tokenizers when available (tiktoken for OpenAI, SentencePiece for Llama, API calls for Claude), falling back to character-based approximation (~4 chars/token or ~1.3 tokens/word) when model-specific tokenizers aren't available.

**Recommendation for APXM**: Implement a hybrid token estimator with:
1. **Compile-time**: Fast approximation (4 chars/token) for fusion decisions
2. **Runtime**: Optional exact tokenization via model-specific libraries
3. **Rust crates**: `tiktoken-rs` (OpenAI), HuggingFace `tokenizers` (cross-model), approximation fallback

---

## 1. Production System Analysis

### 1.1 DSPy Token Tracking

**Approach**: Post-hoc tracking via LLM API responses
**Implementation**:
- LM objects maintain history with token usage (prompt tokens, completion tokens, total tokens)
- Configure with `track_usage=True` to monitor consumption across program calls
- Tracks cached tokens and reasoning tokens (for o1/o4 models)
- **No compile-time estimation** — DSPy relies on actual API responses

**Key Finding**: DSPy does NOT use tiktoken for pre-estimation; it tracks actual usage after API calls.

**Sources**:
- [DSPy Language Models Documentation](https://dspy.ai/learn/programming/language_models/)
- [DSPy Improved LLM Usage Reporting Issue](https://github.com/stanfordnlp/dspy/issues/685)

---

### 1.2 OpenAI tiktoken

**Approach**: Model-specific BPE tokenizer (exact count)
**How It Works**:
1. Text → `.encode()` → list of token integers (e.g., "tiktoken is great!" → `[83, 8251, 2488, 382, 2212, 0]`)
2. Count tokens: `len(tokens)`
3. Different models use different encodings:
   - `cl100k_base`: GPT-3.5, GPT-4, text-embedding-3-*
   - `o200k_base`: Newer models with 200k vocabulary (better non-Latin support)

**Python Example**:
```python
import tiktoken
enc = tiktoken.encoding_for_model('gpt-4o')
tokens = enc.encode('hello world')
print(len(tokens))  # 2
```

**Rust Equivalent**: `tiktoken-rs` crate (version 0.9.1)
```rust
use tiktoken_rs::p50k_base;
let bpe = p50k_base().unwrap();
let tokens = bpe.encode_with_special_tokens("This is an example");
println!("Token count: {}", tokens.len());
```

**Key Finding**: OpenAI provides exact tokenizers for all their models. Tiktoken is the gold standard for OpenAI models.

**Sources**:
- [OpenAI Cookbook: How to Count Tokens with Tiktoken](https://github.com/openai/openai-cookbook/blob/main/examples/How_to_count_tokens_with_tiktoken.ipynb)
- [OpenAI Developers: How to Count Tokens](https://developers.openai.com/cookbook/examples/how_to_count_tokens_with_tiktoken)
- [tiktoken-rs Crate Documentation](https://crates.io/crates/tiktoken-rs)
- [tiktoken-rs GitHub Repository](https://github.com/zurawiki/tiktoken-rs)

---

### 1.3 Claude (Anthropic) Token Counting

**Approach**: API-based exact count (no local tokenizer for Claude 3+)
**Implementation**:
- **Token Count API** (`POST /messages/count_tokens`): Free to use, rate-limited
- Counts tokens in a Message including tools, images, documents WITHOUT creating it
- **Local approximation**: Anthropic recommends **1 token ≈ 3.5 English characters** (when API unavailable)
- Token counts may include system-added tokens (not billed)

**Important**: Anthropic does NOT ship a local tokenizer for Claude 3+ models. Pre-Claude-3 tokenizers exist but are outdated.

**Third-party tools**:
- [claudetokenizer.com](https://www.claudetokenizer.com/)
- [Jellyfishboy/claude-tokenizer (Rust)](https://github.com/Jellyfishboy/claude-tokenizer) — community implementation

**Key Finding**: For Claude, use the Token Count API for exact counts or fall back to 3.5 chars/token.

**Sources**:
- [Claude Token Counting Documentation](https://platform.claude.com/docs/en/build-with-claude/token-counting)
- [Claude API: Count Tokens Reference](https://docs.anthropic.com/en/api/messages-count-tokens)
- [Counting Claude Tokens Without a Tokenizer](https://blog.gopenai.com/counting-claude-tokens-without-a-tokenizer-e767f2b6e632)
- [Jellyfishboy/claude-tokenizer GitHub](https://github.com/Jellyfishboy/claude-tokenizer)

---

### 1.4 LangChain Token Estimation

**Approach**: Universal token counting callback (model-agnostic abstraction)
**Implementation**:
- **Universal token counting callback**: Tracks token usage across all major LLM providers
- Uses model-specific tokenizers under the hood (tiktoken for OpenAI, etc.)
- **LangSmith integration**: Automatically captures prompt/completion/total tokens + estimated costs
- **Limitation**: Reasoning models (o1, o4) generate "reasoning tokens" during multi-step reasoning that can't be estimated from input/output strings alone

**Key Finding**: LangChain abstracts over model-specific tokenizers, providing a unified interface. Production systems integrate with LangSmith for monitoring.

**Sources**:
- [LangChain Token Usage Tracking](https://python.langchain.com/docs/how_to/chat_token_usage_tracking/)
- [Universal Token Counting Callback Changelog](https://changelog.langchain.com/announcements/universal-token-counting-callback-for-langchain-python)
- [How to Setup Token Usage Tracking in LangChain](https://medium.com/@meta_heuristic/how-to-setup-token-usage-tracking-in-langchain-b413b67c70d9)

---

### 1.5 LlamaIndex Token Estimation

**Approach**: Token predictors + callback-based tracking
**Implementation**:
- **TokenCountingHandler callback**: Configurable tokenizer (e.g., tiktoken for specific models)
- **MockLLM/MockEmbedding**: Predict token usage BEFORE making actual API calls
- Deprecated approach used static gpt-2 tokenizer (inaccurate); modern approach uses model-specific tokenizers
- Lower overhead than LangChain (~1.6K vs ~2.4K tokens) = cost savings at scale

**Key Finding**: LlamaIndex provides both pre-estimation (via token predictors) and post-tracking (via callbacks).

**Sources**:
- [LlamaIndex Cost Analysis Documentation](https://docs.llamaindex.ai/en/stable/understanding/evaluating/cost_analysis/)
- [LlamaIndex Token Counting Migration Guide](https://docs.llamaindex.ai/en/stable/module_guides/observability/callbacks/token_counting_migration/)
- [Intercept OpenAI Tokens in LlamaIndex](https://filippotoso.medium.com/intercept-openai-tokens-count-in-llamaindex-and-langchain-applications-375a1ad6f732)

---

### 1.6 vLLM Tokenization

**Approach**: Model's actual tokenizer (exact count via tokenizer.json)
**Implementation**:
- Dedicated routes to retrieve exact token count for a given prompt using loaded model's `tokenizer.json`
- Workflow: tokenize → schedule/batch → prefill/decode → detokenize
- **Metrics exposed**:
  - `vllm:request_generation_tokens` (histogram)
  - `vllm:request_prefill_time_seconds` (histogram)
- **Chunked prefill**: Process large prefills in chunks, batch with decode requests

**Key Finding**: vLLM uses the model's native tokenizer for exact counts. No approximation.

**Sources**:
- [vLLM Metrics Documentation](https://docs.vllm.ai/en/stable/design/metrics/)
- [Inside vLLM: Anatomy of a High-Throughput LLM Inference System](https://blog.vllm.ai/2025/09/05/anatomy-of-vllm.html)
- [vLLM Optimization and Tuning](https://docs.vllm.ai/en/stable/configuration/optimization/)

---

## 2. Token Estimation Approaches (Categorized)

### Approach A: Exact Tokenizer (Model-Specific)

**Description**: Use the model's actual tokenizer (tiktoken, SentencePiece, HuggingFace Tokenizers)

**Pros**:
- Exact token count (100% accurate for billing/context limits)
- Handles special tokens, formatting, message roles correctly

**Cons**:
- Requires model-specific tokenizer (heavy dependency)
- Different tokenizer per model family:
  - OpenAI: tiktoken (BPE)
  - Llama: SentencePiece (BPE/Unigram)
  - Claude 3+: No local tokenizer (API-only)
  - Gemini/PaLM: SentencePiece
- Token counts can differ by 20% between models (e.g., OpenAI vs Llama)

**Production Use**:
- OpenAI API (tiktoken)
- vLLM (model's tokenizer.json)
- LangChain/LlamaIndex (via callbacks)

---

### Approach B: Model-Agnostic Approximation

**Description**: Character-based heuristics (no tokenizer required)

**Rules of Thumb**:
- **OpenAI**: 1 token ≈ 4 characters (English)
- **Anthropic**: 1 token ≈ 3.5 characters (English)
- **Word-based**: 1 token ≈ 0.75 words (or 1.3 tokens/word)
- **Page estimate**: 500 words ≈ 650-700 tokens

**Pros**:
- Fast (no tokenizer loading)
- Zero dependencies
- Model-agnostic (works across providers)

**Cons**:
- Inaccurate (±10-20% variance)
- Varies by language (worse for non-Latin scripts)
- Doesn't account for special tokens, formatting overhead

**Production Use**:
- Quick budget estimation
- Planning/testing (not billing)
- Fallback when tokenizer unavailable

---

### Approach C: Cached Tokenization

**Description**: Tokenize once, cache the result

**Pros**:
- Exact count after first call
- Fast on cache hit

**Cons**:
- Still need tokenizer available
- Cache invalidation complexity
- Memory overhead for large prompt libraries

**Production Use**:
- LlamaIndex (via `llm-tokenizer` crate with caching support)
- Prompt template libraries

---

### Approach D: Hybrid (Recommended for Production)

**Description**: Use exact tokenizer if available, fall back to approximation

**Implementation Strategy**:
1. **Tier 1**: Exact tokenizer for known models (tiktoken for GPT, SentencePiece for Llama)
2. **Tier 2**: API-based count (Claude Token Count API)
3. **Tier 3**: Character approximation (4 chars/token for OpenAI-like, 3.5 for Claude-like)

**Pros**:
- Best of both worlds (exact when possible, fast fallback)
- Graceful degradation
- Supports multi-model workflows

**Cons**:
- More complex implementation
- Need to maintain model → tokenizer mapping

**Production Use**:
- LangChain (universal callback with model dispatch)
- LlamaIndex (configurable tokenizer)
- Most production frameworks

---

## 3. Character Approximation Best Practices

### Empirical Rules

| Provider   | Chars/Token | Tokens/Word | Source                          |
|------------|-------------|-------------|---------------------------------|
| OpenAI     | 4           | 1.3         | OpenAI documentation            |
| Anthropic  | 3.5         | ~1.4        | Anthropic documentation         |
| Generic    | 4           | 1.3         | Industry standard               |

### Production Guidelines

1. **Test before production**: Always validate with real tokenizer to calibrate approximation
2. **Account for hidden tokens**: System prompts, role markers, formatting add 5-15% overhead
3. **Language variance**: Non-Latin scripts can have 2-3x higher token density
4. **Don't replace billing**: Use approximation for planning, exact tokenizers for billing

**Sources**:
- [Understanding LLM Billing: From Characters to Tokens](https://www.edenai.co/post/understanding-llm-billing-from-characters-to-tokens)
- [Calculating LLM Token Counts: A Practical Guide](https://winder.ai/calculating-token-counts-llm-context-windows-practical-guide/)
- [LLM Cost Estimation Guide](https://medium.com/@alphaiterations/llm-cost-estimation-guide-from-token-usage-to-total-spend-fba348d62824)

---

## 4. Rust Tokenizer Ecosystem

### Available Crates (2026)

| Crate                | Version | Description                                      | Use Case              |
|----------------------|---------|--------------------------------------------------|-----------------------|
| `tiktoken-rs`        | 0.9.1   | Pure-Rust tiktoken BPE (OpenAI models)           | GPT-4, GPT-3.5        |
| `tokenizers`         | 0.22.2  | HuggingFace tokenizers (BPE, WordPiece, Unigram) | Cross-model support   |
| `rust_tokenizers`    | 3.1+    | BPE, WordPiece, SentencePiece for BERT/GPT/XLNet | Multi-model NLP       |
| `sentencepiece`      | Latest  | Rust bindings to Google's SentencePiece          | Llama, PaLM, T5       |
| `llm-tokenizer`      | 1.3.1   | LLM tokenizer with caching + chat template       | Production workflows  |

### Recommended: `tiktoken-rs`

**Installation**:
```toml
[dependencies]
tiktoken-rs = "0.9.1"
```

**Basic Usage**:
```rust
use tiktoken_rs::p50k_base;

fn count_tokens(text: &str) -> usize {
    let bpe = p50k_base().unwrap();
    let tokens = bpe.encode_with_special_tokens(text);
    tokens.len()
}
```

**Chat Completion Example**:
```rust
use tiktoken_rs::get_chat_completion_max_tokens;
use async_openai::types::{ChatCompletionRequestMessage, Role};

let messages = vec![
    ChatCompletionRequestMessage {
        content: Some("You are a helpful assistant.".to_string()),
        role: Role::System,
        ..Default::default()
    },
    ChatCompletionRequestMessage {
        content: Some("Hello!".to_string()),
        role: Role::User,
        ..Default::default()
    },
];

let max_tokens = get_chat_completion_max_tokens("gpt-4o", &messages)?;
```

**Sources**:
- [tiktoken-rs Crate](https://crates.io/crates/tiktoken-rs)
- [tiktoken-rs Documentation](https://docs.rs/tiktoken-rs/latest/tiktoken_rs/)
- [tiktoken-rs GitHub](https://github.com/zurawiki/tiktoken-rs)

---

### Recommended: HuggingFace `tokenizers`

**Why**: Cross-model support (BPE, WordPiece, SentencePiece), load pretrained tokenizers

**Installation**:
```toml
[dependencies]
tokenizers = "0.22.2"
```

**Usage**:
```rust
use tokenizers::Tokenizer;

fn count_tokens_huggingface(text: &str, model: &str) -> Result<usize, Box<dyn std::error::Error>> {
    let tokenizer = Tokenizer::from_pretrained(model, None)?;
    let encoding = tokenizer.encode(text, false)?;
    Ok(encoding.get_ids().len())
}
```

**Performance**: Core is written in Rust — tokenizes 1GB of text in <20 seconds on server CPU.

**Sources**:
- [HuggingFace Tokenizers Documentation](https://huggingface.co/docs/tokenizers/index)
- [tokenizers Crate](https://crates.io/crates/tokenizers)
- [HuggingFace Tokenizers GitHub](https://github.com/huggingface/tokenizers)

---

### Alternative: `rust_tokenizers`

**Why**: Comprehensive support (BPE, WordPiece, SentencePiece) without HuggingFace dependency

**Installation**:
```toml
[dependencies]
rust_tokenizers = "8.1"
```

**Models Supported**: BERT, RoBERTa, GPT-2, XLNet, Llama (SentencePiece)

**Sources**:
- [rust-tokenizers GitHub](https://github.com/guillaume-be/rust-tokenizers)
- [Rust SentencePiece Implementation Article](https://guillaume-be.github.io/2020-05-30/sentence_piece)
- [rust_tokenizers Crate](https://crates.io/crates/rust_tokenizers)

---

## 5. Llama-Specific Considerations

### Tokenizer

Llama models (1, 2, 3.x) use **SentencePiece BPE** tokenizer.

**Important**: Token counts differ by ~20% between OpenAI (tiktoken) and Llama (SentencePiece). Always use Llama-specific tokenizers for accurate counts.

### Llama 3 Exception

Llama 3.x uses **tiktoken with `o200k_base` encoding** (same as GPT-4o), but with custom special tokens. Count may differ by 1-3 tokens per message.

### Tools

- **Python**: `llama-tokenizer-js` (works in browser + Node)
- **Rust**: `sentencepiece` crate or HuggingFace `tokenizers` with Llama checkpoint

**Sources**:
- [llama-tokenizer-js npm](https://www.npmjs.com/package/llama-tokenizer-js)
- [In-depth Understanding of Llama Tokenizer](https://medium.com/@manyi.yim/in-depth-understanding-of-llama-tokenizer-d91777025dab)
- [Llama Tokenizer GitHub](https://github.com/meta-llama/llama/blob/main/llama/tokenizer.py)

---

## 6. Recommendation for APXM

### Requirements

APXM needs token estimation for:
1. **Compile-time fusion decisions**: Should we fuse `ASK` nodes into a `Chain`?
2. **Context budget validation**: Will a `Chain` fit within model's context window?
3. **Multi-model support**: GPT-4, Claude, Llama, Qwen, Gemini
4. **Fast estimates**: Compiler can't afford heavy tokenization during optimization passes

### Proposed Architecture

**Phase 1: Character-Based Approximation (Immediate)**

```rust
// apxm-core/src/token_estimator.rs

pub struct TokenEstimator {
    chars_per_token: f64,
}

impl TokenEstimator {
    pub fn for_model(model: &str) -> Self {
        let chars_per_token = if model.starts_with("claude") {
            3.5
        } else if model.starts_with("gpt") || model.starts_with("o1") {
            4.0
        } else {
            4.0  // Default to OpenAI-like
        };
        Self { chars_per_token }
    }

    pub fn estimate_tokens(&self, text: &str) -> usize {
        (text.len() as f64 / self.chars_per_token).ceil() as usize
    }

    pub fn estimate_tokens_with_overhead(&self, text: &str, system_prompt: Option<&str>) -> usize {
        let base = self.estimate_tokens(text);
        let system = system_prompt.map(|s| self.estimate_tokens(s)).unwrap_or(0);
        // Add 10% overhead for message formatting, role markers
        ((base + system) as f64 * 1.1).ceil() as usize
    }
}
```

**Usage in fusion heuristics**:
```rust
// apxm-compiler/src/passes/heuristics.rs

fn should_fuse_into_chain(&self, nodes: &[Node]) -> bool {
    let estimator = TokenEstimator::for_model("gpt-4o");
    let total_tokens: usize = nodes.iter()
        .map(|n| estimator.estimate_tokens(&n.prompt))
        .sum();

    total_tokens < self.context_budget  // e.g., 128K for GPT-4
}
```

**Pros**:
- Zero dependencies
- Fast (<1μs per estimation)
- Good enough for fusion decisions (±15% error acceptable)

**Cons**:
- Not accurate for billing
- Doesn't handle special tokens

---

**Phase 2: Optional Exact Tokenization (Future)**

Add `tiktoken-rs` as optional dependency:

```toml
[dependencies]
tiktoken-rs = { version = "0.9", optional = true }

[features]
exact-tokens = ["tiktoken-rs"]
```

```rust
#[cfg(feature = "exact-tokens")]
pub struct ExactTokenEstimator {
    tiktoken_bpe: tiktoken_rs::CoreBPE,
}

#[cfg(feature = "exact-tokens")]
impl ExactTokenEstimator {
    pub fn for_model(model: &str) -> Result<Self> {
        let bpe = if model.starts_with("gpt-4") {
            tiktoken_rs::cl100k_base()?
        } else if model.starts_with("gpt-3.5") {
            tiktoken_rs::cl100k_base()?
        } else {
            return Err("Unsupported model for exact tokenization");
        };
        Ok(Self { tiktoken_bpe: bpe })
    }

    pub fn count_tokens(&self, text: &str) -> usize {
        self.tiktoken_bpe.encode_with_special_tokens(text).len()
    }
}
```

**When to use**:
- Runtime validation (before executing workflow)
- Detailed diagnostics (`apxm analyze --exact-tokens`)
- Budget verification for production workflows

---

**Phase 3: Multi-Model Tokenizers (Future)**

Add HuggingFace `tokenizers` for cross-model support:

```toml
[dependencies]
tokenizers = { version = "0.22", optional = true }

[features]
multi-model-tokens = ["tokenizers"]
```

```rust
pub enum TokenizerBackend {
    Approximation(TokenEstimator),
    #[cfg(feature = "exact-tokens")]
    Tiktoken(ExactTokenEstimator),
    #[cfg(feature = "multi-model-tokens")]
    HuggingFace(tokenizers::Tokenizer),
}

impl TokenizerBackend {
    pub fn for_model(model: &str) -> Self {
        #[cfg(feature = "multi-model-tokens")]
        if let Ok(tokenizer) = tokenizers::Tokenizer::from_pretrained(model, None) {
            return Self::HuggingFace(tokenizer);
        }

        #[cfg(feature = "exact-tokens")]
        if model.starts_with("gpt") {
            if let Ok(exact) = ExactTokenEstimator::for_model(model) {
                return Self::Tiktoken(exact);
            }
        }

        Self::Approximation(TokenEstimator::for_model(model))
    }
}
```

---

### Implementation Plan

**Step 1** (Immediate): Add character-based `TokenEstimator` to `apxm-core`
- No new dependencies
- Use in fusion heuristics (`should_fuse_into_chain()`)
- Document accuracy limitations in comments

**Step 2** (v0.2): Add `tiktoken-rs` as optional feature
- CLI flag: `apxm analyze --exact-tokens`
- Use for budget validation warnings
- Test against actual API responses

**Step 3** (v0.3+): Add HuggingFace `tokenizers` for Llama/Gemini
- Support model checkpoints: `llama-3.1-8b`, `gemma-2-9b`
- Load tokenizer from HuggingFace Hub
- Cache tokenizer files in `~/.apxm/tokenizers/`

**Step 4** (Future): Claude Token Count API integration
- For Claude models, optionally call `POST /messages/count_tokens`
- Requires API key + network request
- Cache results per (model, prompt hash)

---

## 7. Key Takeaways

### For Compile-Time Fusion

**Use character approximation**:
- 4 chars/token for OpenAI-like models
- 3.5 chars/token for Claude-like models
- Add 10% overhead for message formatting
- Accept ±15% variance (acceptable for optimization decisions)

### For Runtime Validation

**Use exact tokenizers**:
- `tiktoken-rs` for GPT models
- HuggingFace `tokenizers` for Llama/Gemini
- Claude Token Count API for Claude models
- Verify before executing expensive workflows

### For Multi-Model Workflows

**Implement hybrid strategy**:
1. Try exact tokenizer (model-specific)
2. Fall back to approximation
3. Cache results when possible
4. Document accuracy per model in diagnostics

---

## 8. Testing Validation

### Verify tiktoken-rs

```bash
# Already confirmed installed:
python3 -c "import tiktoken; print(tiktoken.encoding_for_model('gpt-4o').encode('hello world'))"
# Output: [24912, 2375] (2 tokens)
```

### Rust Crate Availability

```bash
cargo search tiktoken | head -5
# tiktoken = "3.1.2" — Pure-Rust implementation ✅
# tiktoken-rs = "0.9.1" — Rust bindings ✅

cargo search tokenizers | head -5
# tokenizers = "0.22.2" — HuggingFace tokenizers ✅
```

### Benchmark Test

```rust
// Confirm character approximation accuracy
#[test]
fn test_approximation_accuracy() {
    let text = "This is a test prompt for token estimation.";

    // Exact (via tiktoken-rs)
    let bpe = tiktoken_rs::p50k_base().unwrap();
    let exact = bpe.encode_with_special_tokens(text).len();

    // Approximation
    let approx = (text.len() as f64 / 4.0).ceil() as usize;

    let error = ((exact as f64 - approx as f64) / exact as f64).abs();
    assert!(error < 0.2, "Error: {:.2}%", error * 100.0);  // Allow 20% variance
}
```

---

## Sources

### DSPy
- [DSPy Language Models Documentation](https://dspy.ai/learn/programming/language_models/)
- [DSPy GitHub Repository](https://github.com/stanfordnlp/dspy)

### OpenAI tiktoken
- [OpenAI Cookbook: How to Count Tokens with Tiktoken](https://github.com/openai/openai-cookbook/blob/main/examples/How_to_count_tokens_with_tiktoken.ipynb)
- [OpenAI Developers: How to Count Tokens](https://developers.openai.com/cookbook/examples/how_to_count_tokens_with_tiktoken)
- [OpenAI Tokenizer Guide](https://www.aitoolskit.io/learn/openai-tokenizer-tiktoken-guide)

### Claude (Anthropic)
- [Claude Token Counting Documentation](https://platform.claude.com/docs/en/build-with-claude/token-counting)
- [Claude API: Count Tokens Reference](https://docs.anthropic.com/en/api/messages-count-tokens)
- [Counting Claude Tokens Without a Tokenizer](https://blog.gopenai.com/counting-claude-tokens-without-a-tokenizer-e767f2b6e632)

### LangChain
- [LangChain Token Usage Tracking](https://python.langchain.com/docs/how_to/chat_token_usage_tracking/)
- [Universal Token Counting Callback](https://changelog.langchain.com/announcements/universal-token-counting-callback-for-langchain-python)

### LlamaIndex
- [LlamaIndex Cost Analysis](https://docs.llamaindex.ai/en/stable/understanding/evaluating/cost_analysis/)
- [Token Counting Migration Guide](https://docs.llamaindex.ai/en/stable/module_guides/observability/callbacks/token_counting_migration/)

### vLLM
- [vLLM Metrics Documentation](https://docs.vllm.ai/en/stable/design/metrics/)
- [Inside vLLM: Anatomy of a High-Throughput LLM Inference System](https://blog.vllm.ai/2025/09/05/anatomy-of-vllm.html)

### Rust Tokenizers
- [tiktoken-rs Crate](https://crates.io/crates/tiktoken-rs)
- [tiktoken-rs GitHub](https://github.com/zurawiki/tiktoken-rs)
- [HuggingFace Tokenizers](https://huggingface.co/docs/tokenizers/index)
- [rust-tokenizers GitHub](https://github.com/guillaume-be/rust-tokenizers)

### Best Practices
- [Understanding LLM Billing: From Characters to Tokens](https://www.edenai.co/post/understanding-llm-billing-from-characters-to-tokens)
- [Calculating LLM Token Counts: A Practical Guide](https://winder.ai/calculating-token-counts-llm-context-windows-practical-guide/)
- [LLM Cost Estimation Guide](https://medium.com/@alphaiterations/llm-cost-estimation-guide-from-token-usage-to-total-spend-fba348d62824)
