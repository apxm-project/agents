//! Hook ABI vectors: typed callbacks over the scoped Agent Facade thread context
//! explicitly and either observe or return a statically declared replacement.

use apxm_kernel::{Hook, HookReturn, apply_hooks};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    iterations: u32,
}

#[test]
fn observe_only_hook_leaves_context_and_result_unchanged() {
    let hooks: Vec<Hook<Context, String>> = vec![Box::new(|_facade, _result| HookReturn::Observe)];
    let effect = apply_hooks(Context { iterations: 1 }, "answer".to_string(), &hooks);
    assert_eq!(effect.context, Context { iterations: 1 });
    assert_eq!(effect.result, "answer");
    assert!(!effect.replaced);
}

#[test]
fn hook_changes_context_only_by_assigning_agent_context() {
    let hooks: Vec<Hook<Context, String>> = vec![Box::new(|facade, _result| {
        let next = Context {
            iterations: facade.context().iterations + 1,
        };
        facade.set_context(next);
        HookReturn::Observe
    })];
    let effect = apply_hooks(Context { iterations: 1 }, "answer".to_string(), &hooks);
    assert_eq!(effect.context, Context { iterations: 2 });
    assert_eq!(
        effect.result, "answer",
        "context change does not alter the result"
    );
    assert!(!effect.replaced);
}

#[test]
fn replacement_hook_replaces_the_declared_result() {
    let hooks: Vec<Hook<Context, String>> = vec![Box::new(|_facade, _result| {
        HookReturn::Replace("redacted".to_string())
    })];
    let effect = apply_hooks(Context { iterations: 0 }, "secret".to_string(), &hooks);
    assert_eq!(effect.result, "redacted");
    assert!(effect.replaced);
}

#[test]
fn hooks_apply_in_declaration_order() {
    let hooks: Vec<Hook<Context, String>> = vec![
        Box::new(|facade, _result| {
            facade.set_context(Context { iterations: 10 });
            HookReturn::Replace("first".to_string())
        }),
        Box::new(|facade, result| {
            // Sees the first hook's context and result, then replaces again.
            assert_eq!(facade.context().iterations, 10);
            assert_eq!(result, "first");
            HookReturn::Replace("second".to_string())
        }),
    ];
    let effect = apply_hooks(Context { iterations: 0 }, "start".to_string(), &hooks);
    assert_eq!(effect.result, "second");
    assert_eq!(effect.context, Context { iterations: 10 });
    assert!(effect.replaced);
}
