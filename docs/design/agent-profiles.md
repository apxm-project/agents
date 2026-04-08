# Agent Profile Design

> **Status: Design Proposal -- Not Yet Implemented**
>
> This document describes a planned capability-scoping system for agent nodes.
> The `AgentProfile` struct and registry do not exist in the codebase yet.
> See [Implementation Path](#implementation-path) for the rollout plan.

*How each node knows what it can do.*

> **See also:**
> - [Agent ontology](agent-ontology.md) -- foundational (B, G, C) model
> - [AAM theory](../pxm/aam.md) -- ScopePolicy and ScopeSpec definitions
> - [Multi-agent guide](../guides/multi-agent.md) -- practical multi-agent patterns
> - [Sessions](../implementation/runtime/sessions.md) -- per-node workspace layout

---

## The Problem

Currently, node capability assignment is implicit:
- `skill_names` is passed as a list of strings
- Context assembler just says "see skills/{name}/SKILL.md"
- No enforcement -- the LLM sees the skills, might use them, might not
- No filtering -- every node sees all skills
- No formal declaration of what a node is *allowed* to do

This breaks the hierarchical AAM model. If we want `ScopePolicy::Filter(["code_review", "refactor"])`, we need to know *which* capabilities each node should have.

---

## The Design: AgentProfile

An **AgentProfile** is a named, reusable specification of what an agent node can do:

```rust
/// Defines what an agent node is allowed to do.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    /// Unique name (e.g., "architect", "coder", "reviewer")
    pub name: String,

    /// Human description of this profile's role
    pub description: String,

    /// Which primitive ops this profile can invoke
    pub allowed_ops: Vec<AISOperationType>,

    /// Which skills (by name) this profile can invoke
    pub allowed_skills: Vec<String>,

    /// Which belief keys this profile can read
    pub belief_read_scope: BeliefScope,

    /// Which belief keys this profile can write
    pub belief_write_scope: BeliefScope,

    /// Which tools (external APIs) this profile can use
    pub allowed_tools: Vec<String>,

    /// Constraints injected into context (security/compliance rules)
    pub constraints: Vec<String>,

    /// Model preferences (can override global default)
    pub model_preference: Option<ModelId>,

    /// Token budget limit for this profile
    pub token_budget: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BeliefScope {
    /// Can access all beliefs
    All,
    /// Can only access listed keys
    Keys(Vec<String>),
    /// Can only access beliefs matching prefix
    Prefix(String),
    /// No belief access
    None,
}
```

---

## Profile Examples

### Architect Profile
```yaml
name: architect
description: "System design and high-level planning"
allowed_ops:
  - THINK
  - REASON
  - PLAN
  - COMMUNICATE
  - VERIFY
allowed_skills:
  - system_design
  - api_design
  - decompose_task
belief_read_scope: all
belief_write_scope:
  keys: [architecture, interfaces, design_decisions]
allowed_tools: []  # No external tools
constraints:
  - "Stay at design/interface level"
  - "Do not write implementation code"
  - "Focus on structure, not details"
model_preference: "claude-opus-4"
token_budget: 32000
```

### Coder Profile
```yaml
name: coder
description: "Implementation and code generation"
allowed_ops:
  - ASK
  - THINK
  - EXC
  - INV
  - VERIFY
allowed_skills:
  - code_gen
  - refactor
  - write_tests
  - debug
belief_read_scope:
  keys: [architecture, requirements, code_context]
belief_write_scope:
  keys: [implementation, test_results]
allowed_tools:
  - file_read
  - file_write
  - shell
  - git
constraints:
  - "Follow the provided architecture"
  - "Write tests for new code"
  - "Do not modify interfaces without explicit approval"
model_preference: "claude-sonnet-4"
token_budget: 16000
```

### Reviewer Profile
```yaml
name: reviewer
description: "Code review and quality assurance"
allowed_ops:
  - ASK
  - REASON
  - VERIFY
  - REFLECT
allowed_skills:
  - code_review
  - security_audit
  - performance_review
belief_read_scope: all
belief_write_scope:
  keys: [review_results, issues_found, approval_status]
allowed_tools:
  - file_read
  - git_diff
constraints:
  - "Cannot modify code, only review"
  - "Must cite specific line numbers for issues"
  - "Focus on correctness, security, and maintainability"
model_preference: "claude-sonnet-4"
token_budget: 24000
```

---

## How It Connects

### 1. Graph Declaration

Nodes declare their profile in the graph:

```json
{
  "id": 3,
  "name": "implement_feature",
  "op": "SPAWN_AGENT",
  "attributes": {
    "profile": "coder",
    "task_spec": "Implement the auth module per {{architecture}}"
  }
}
```

### 2. Profile Resolution

At compile time or runtime init, the profile is resolved:

```rust
impl AgentProfileRegistry {
    pub fn resolve(&self, profile_name: &str) -> Result<AgentProfile> {
        // Check built-in profiles
        if let Some(p) = self.builtin.get(profile_name) {
            return Ok(p.clone());
        }
        // Check user-defined profiles in .apxm/profiles/
        if let Some(p) = self.load_from_file(profile_name)? {
            return Ok(p);
        }
        // Fallback to default
        Ok(AgentProfile::default())
    }
}
```

### 3. ScopeSpec Generation

The profile generates a `ScopeSpec` for child AAM creation (see [AAM theory](../pxm/aam.md) for ScopeSpec semantics):

```rust
impl AgentProfile {
    pub fn to_scope_spec(&self) -> ScopeSpec {
        ScopeSpec {
            beliefs: match &self.belief_read_scope {
                BeliefScope::All => ScopePolicy::Inherit,
                BeliefScope::Keys(keys) => ScopePolicy::Filter(keys.clone()),
                BeliefScope::Prefix(p) => ScopePolicy::Filter(vec![format!("{}*", p)]),
                BeliefScope::None => ScopePolicy::Isolate,
            },
            capabilities: ScopePolicy::Filter(
                self.allowed_skills.iter()
                    .chain(self.allowed_tools.iter())
                    .cloned()
                    .collect()
            ),
            goals: ScopePolicy::Inherit,  // Usually inherit, can be overridden
        }
    }
}
```

### 4. Context Assembly

The ContextAssembler uses the profile to build the node's context:

```rust
impl ContextAssembler {
    pub fn assemble_with_profile(
        &self,
        node_id: u64,
        profile: &AgentProfile,
    ) -> io::Result<String> {
        let mut doc = String::new();

        // Role from profile
        doc.push_str(&format!("## Role: {}\n", profile.name));
        doc.push_str(&format!("{}\n\n", profile.description));

        // Only show allowed skills
        doc.push_str("## Available Capabilities\n");
        for skill in &profile.allowed_skills {
            doc.push_str(&format!("- {}\n", skill));
        }
        doc.push_str("\n");

        // Only show allowed tools
        if !profile.allowed_tools.is_empty() {
            doc.push_str("## Available Tools\n");
            for tool in &profile.allowed_tools {
                doc.push_str(&format!("- {}\n", tool));
            }
            doc.push_str("\n");
        }

        // Inject constraints
        doc.push_str("## Constraints\n");
        for c in &profile.constraints {
            doc.push_str(&format!("- {}\n", c));
        }

        // Token budget warning
        if let Some(budget) = profile.token_budget {
            doc.push_str(&format!("\nToken budget: {} tokens\n", budget));
        }

        Ok(doc)
    }
}
```

### 5. Runtime Enforcement

The scheduler/executor validates operations against the profile:

```rust
impl Executor {
    fn validate_op(&self, node: &AISNode, profile: &AgentProfile) -> Result<()> {
        // Check if op type is allowed
        if !profile.allowed_ops.contains(&node.op_type) {
            return Err(ExecutionError::DisallowedOp {
                op: node.op_type,
                profile: profile.name.clone(),
            });
        }

        // Check if invoked skill is allowed (for FLOW_CALL)
        if let Some(skill_name) = node.get_skill_target() {
            if !profile.allowed_skills.contains(&skill_name) {
                return Err(ExecutionError::DisallowedSkill {
                    skill: skill_name,
                    profile: profile.name.clone(),
                });
            }
        }

        Ok(())
    }
}
```

---

## File Structure

```
~/.apxm/profiles/
  +-- architect.yaml
  +-- coder.yaml
  +-- reviewer.yaml
  +-- custom/
      +-- security-auditor.yaml
      +-- data-analyst.yaml

project/.apxm/profiles/
  +-- project-specific-profile.yaml
```

Profile resolution order:
1. Project-local profiles
2. User profiles (`~/.apxm/profiles/`)
3. Built-in defaults

---

## Relationship to Existing Code

| Current | With AgentProfile |
|---------|------------------|
| `infer_role(profile, node_name)` | `profile.name` + `profile.description` |
| `role_description()` | `profile.description` |
| `profile_constraints()` | `profile.constraints` |
| `skill_names: &[String]` | `profile.allowed_skills` |
| No enforcement | `validate_op()` at execution |
| No belief filtering | `profile.belief_read_scope` -> `ScopeSpec` |

---

## Implementation Path

1. **Define `AgentProfile` struct** in `apxm-core/src/types/`
2. **Create `AgentProfileRegistry`** with built-in profiles + file loading
3. **Update ContextAssembler** to use profiles instead of inferred roles
4. **Wire profile -> ScopeSpec** conversion
5. **Add validation in executor** (optional enforcement mode)
6. **Add `--profile` to CLI** for per-node overrides

---

## Open Questions

1. **Inheritance**: Should profiles extend other profiles? (`coder extends base_agent`)
2. **Dynamic profiles**: Can a node's profile change mid-execution?
3. **Composite profiles**: Can a node have multiple profiles merged?
4. **Compile-time vs runtime**: Should profile violations be caught at compile time or runtime?

---

## Recommendation

Start with:
1. `AgentProfile` struct + 3 built-in profiles (architect, coder, reviewer)
2. Profile loading from YAML files
3. ContextAssembler integration
4. Soft validation (warn, don't block)

Then iterate on enforcement and belief scoping.
