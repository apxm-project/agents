//! Typed Hook callbacks over the scoped Agent Facade, lowered to the private
//! `HookEffect<C, T>` ABI.
//!
//! Hook source changes context only by assigning `agent.context` (here,
//! [`AgentFacade::set_context`]); its return is either observe-only or a
//! statically declared replacement result. Bindings are a statically compiled,
//! ordered slice — there is no hidden callback dispatcher and no mutable runtime
//! Hook registry. The runtime never mutates prompt context implicitly.

/// The scoped Agent Facade a Hook sees. It exposes exactly the current context;
/// the only mutation is an explicit `set_context`.
pub struct AgentFacade<C> {
    context: C,
}

impl<C> AgentFacade<C> {
    #[must_use]
    pub fn new(context: C) -> Self {
        Self { context }
    }

    #[must_use]
    pub fn context(&self) -> &C {
        &self.context
    }

    /// Assign `agent.context`. This is the only way Hook source changes context.
    pub fn set_context(&mut self, context: C) {
        self.context = context;
    }

    fn into_context(self) -> C {
        self.context
    }
}

/// What a Hook returns: observe only, or a statically declared replacement.
pub enum HookReturn<T> {
    Observe,
    Replace(T),
}

/// One compiled Hook: a typed callback over the scoped facade and the current
/// result value. It is an ordinary function pointer/closure, not a registry
/// entry.
pub type Hook<C, T> = Box<dyn Fn(&mut AgentFacade<C>, &T) -> HookReturn<T>>;

/// The private ABI a chain of Hooks lowers to: the threaded-out context, the
/// (possibly replaced) result, and whether any Hook replaced it.
pub struct HookEffect<C, T> {
    pub context: C,
    pub result: T,
    pub replaced: bool,
}

/// Apply a statically compiled, ordered Hook chain around one result value,
/// threading context explicitly. Context changes only through the facade;
/// the result changes only through an explicit `Replace`.
pub fn apply_hooks<C, T>(context: C, result: T, hooks: &[Hook<C, T>]) -> HookEffect<C, T> {
    let mut facade = AgentFacade::new(context);
    let mut result = result;
    let mut replaced = false;

    for hook in hooks {
        match hook(&mut facade, &result) {
            HookReturn::Observe => {}
            HookReturn::Replace(next) => {
                result = next;
                replaced = true;
            }
        }
    }

    HookEffect {
        context: facade.into_context(),
        result,
        replaced,
    }
}
