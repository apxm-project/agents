//! The permission decision vocabulary and the layer stack that resolves it.
//!
//! One three-valued decision — `allow`, `ask`, `deny` — is the machine's whole
//! permission vocabulary. It lives here, beside the capability id catalogue,
//! because every consumer resolves through this crate: the capability
//! contracts, the Agent Program graph and its evidence, the admission
//! envelope, the runtime chokepoint, and both authoring frontends. None of
//! them may spell a decision itself.
//!
//! A decision is a *request or a ruling*, never a grant. Authority is a
//! property of the machine: resolving a decision here confers nothing, and the
//! layer stack exists precisely so a later layer can only take authority away.

use std::collections::BTreeMap;
use std::fmt;

use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};

/// Wire value of the permissive decision.
pub const ALLOW: &str = "allow";
/// Wire value of the decision that defers to an approval gate.
pub const ASK: &str = "ask";
/// Wire value of the refusing decision.
pub const DENY: &str = "deny";

/// Key naming the decision inside the object wire form.
pub const DECISION_KEY: &str = "decision";
/// Key naming the optional reason inside the object wire form.
pub const REASON_KEY: &str = "reason";

/// The closed permission decision, optionally carrying why it was made.
///
/// Wire form is the bare decision string when there is no reason and the
/// object `{"decision": …, "reason": …}` when there is one, so a reasonless
/// decision reads and writes exactly as the three-value string vocabulary it
/// has always been. The owner schemas publish that form as `PermissionDecision`
/// and both authoring frontends project it under the same name; all three are
/// this one type.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PermissionDecision {
    /// Proceed without a gate.
    Allow { reason: Option<String> },
    /// Proceed only through an approval gate.
    Ask { reason: Option<String> },
    /// Refuse.
    Deny { reason: Option<String> },
}

/// A decision string outside the closed vocabulary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownPermissionDecision(pub String);

impl fmt::Display for UnknownPermissionDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown permission decision {:?}: expected one of {ALLOW}, {ASK}, {DENY}",
            self.0
        )
    }
}

impl std::error::Error for UnknownPermissionDecision {}

impl PermissionDecision {
    /// Every decision in the closed vocabulary, ordered from most to least
    /// permissive.
    pub const DECISIONS: [&'static str; 3] = [ALLOW, ASK, DENY];

    /// The permissive decision, carrying no reason.
    #[must_use]
    pub const fn allow() -> Self {
        Self::Allow { reason: None }
    }

    /// The gated decision and why the gate is there.
    #[must_use]
    pub fn ask(reason: impl Into<String>) -> Self {
        Self::Ask {
            reason: Some(reason.into()),
        }
    }

    /// The refusing decision and why it refuses.
    #[must_use]
    pub fn deny(reason: impl Into<String>) -> Self {
        Self::Deny {
            reason: Some(reason.into()),
        }
    }

    /// Build a decision from its wire string and an optional reason.
    ///
    /// # Errors
    ///
    /// Returns [`UnknownPermissionDecision`] when the string is outside the
    /// closed vocabulary.
    pub fn from_parts(
        decision: &str,
        reason: Option<String>,
    ) -> Result<Self, UnknownPermissionDecision> {
        match decision {
            ALLOW => Ok(Self::Allow { reason }),
            ASK => Ok(Self::Ask { reason }),
            DENY => Ok(Self::Deny { reason }),
            other => Err(UnknownPermissionDecision(other.to_string())),
        }
    }

    /// The same decision carrying `reason` instead of whatever it held.
    #[must_use]
    pub fn with_reason(&self, reason: Option<String>) -> Self {
        match self {
            Self::Allow { .. } => Self::Allow { reason },
            Self::Ask { .. } => Self::Ask { reason },
            Self::Deny { .. } => Self::Deny { reason },
        }
    }

    /// Wire string for this decision. The single source of the label used by
    /// serde, metrics, diagnostics, and both authoring frontends.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Allow { .. } => ALLOW,
            Self::Ask { .. } => ASK,
            Self::Deny { .. } => DENY,
        }
    }

    /// Why the decision was made, when it says.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Allow { reason } | Self::Ask { reason } | Self::Deny { reason } => {
                reason.as_deref()
            }
        }
    }

    /// Whether this decision permits an effect without a gate.
    #[must_use]
    pub const fn is_allow(&self) -> bool {
        matches!(self, Self::Allow { .. })
    }

    /// Position on the tighten-only lattice: `Allow` (0) is the most
    /// permissive, `Deny` (2) the least.
    #[must_use]
    pub const fn restriction(&self) -> u8 {
        match self {
            Self::Allow { .. } => 0,
            Self::Ask { .. } => 1,
            Self::Deny { .. } => 2,
        }
    }

    /// Whether moving from `self` to `other` only removes authority.
    ///
    /// `Allow -> Ask -> Deny` is legal in that direction and at rest; every
    /// step back toward `Allow` would manufacture authority and is refused.
    #[must_use]
    pub const fn tightens_to(&self, other: &Self) -> bool {
        other.restriction() >= self.restriction()
    }
}

impl fmt::Display for PermissionDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.reason() {
            None => f.write_str(self.as_str()),
            Some(reason) => write!(f, "{} ({reason})", self.as_str()),
        }
    }
}

impl Serialize for PermissionDecision {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.reason() {
            None => serializer.serialize_str(self.as_str()),
            Some(reason) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry(DECISION_KEY, self.as_str())?;
                map.serialize_entry(REASON_KEY, reason)?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for PermissionDecision {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(PermissionDecisionVisitor)
    }
}

struct PermissionDecisionVisitor;

impl<'de> Visitor<'de> for PermissionDecisionVisitor {
    type Value = PermissionDecision;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "one of {ALLOW}, {ASK}, {DENY}, or an object carrying that decision and a reason"
        )
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        PermissionDecision::from_parts(value, None).map_err(de::Error::custom)
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
        let mut decision: Option<String> = None;
        let mut reason: Option<String> = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                DECISION_KEY => {
                    if decision.replace(map.next_value()?).is_some() {
                        return Err(de::Error::duplicate_field(DECISION_KEY));
                    }
                }
                REASON_KEY => {
                    if reason.replace(map.next_value()?).is_some() {
                        return Err(de::Error::duplicate_field(REASON_KEY));
                    }
                }
                other => {
                    return Err(de::Error::unknown_field(other, &[DECISION_KEY, REASON_KEY]));
                }
            }
        }
        let decision = decision.ok_or_else(|| de::Error::missing_field(DECISION_KEY))?;
        PermissionDecision::from_parts(&decision, reason).map_err(de::Error::custom)
    }
}

/// The closed set of layers that may state a permission decision, ordered by
/// precedence: a later layer overrides an earlier one, and may only tighten.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionLayer {
    /// What the Agent Program's own source requested.
    Code,
    /// What the package that ships the program states.
    Package,
    /// What the deployment profile running the package states.
    Deployment,
}

impl PermissionLayer {
    /// Every layer, in ascending precedence.
    pub const ALL: [Self; 3] = [Self::Code, Self::Package, Self::Deployment];

    /// The layer's precedence. Higher wins; the values are the published
    /// stack positions, not array indices.
    #[must_use]
    pub const fn precedence(self) -> u8 {
        match self {
            Self::Code => 10,
            Self::Package => 20,
            Self::Deployment => 30,
        }
    }

    /// Wire string for this layer.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Package => "package",
            Self::Deployment => "deployment",
        }
    }
}

impl fmt::Display for PermissionLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One capability's winning decision and the layer that produced it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedPermission {
    pub decision: PermissionDecision,
    pub layer: PermissionLayer,
}

/// What one layer states, keyed by capability reference.
pub type LayerDecisions = BTreeMap<String, PermissionDecision>;

/// Why a layer stack could not be resolved. Every variant fails closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionResolutionError {
    /// A later layer tried to hand back authority an earlier layer withheld.
    Widening {
        capability_ref: String,
        held: PermissionDecision,
        held_layer: PermissionLayer,
        attempted: PermissionDecision,
        attempted_layer: PermissionLayer,
    },
    /// A layer above the code layer named a capability the program never
    /// requested, so there is no request for it to narrow.
    UnrequestedOverride {
        capability_ref: String,
        attempted: PermissionDecision,
        attempted_layer: PermissionLayer,
    },
}

impl fmt::Display for PermissionResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Widening {
                capability_ref,
                held,
                held_layer,
                attempted,
                attempted_layer,
            } => write!(
                f,
                "capability '{capability_ref}' is {held} at the {held_layer} layer; the \
                 {attempted_layer} layer may only tighten it, not widen it to {attempted}"
            ),
            Self::UnrequestedOverride {
                capability_ref,
                attempted,
                attempted_layer,
            } => write!(
                f,
                "the {attempted_layer} layer decides {attempted} for capability \
                 '{capability_ref}', which the program never requested"
            ),
        }
    }
}

impl std::error::Error for PermissionResolutionError {}

/// The resolved decision for every requested capability, with the layer that
/// won each one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PermissionResolution {
    entries: BTreeMap<String, ResolvedPermission>,
}

impl PermissionResolution {
    /// Resolve the layer stack.
    ///
    /// Layers are applied in the closed precedence order of
    /// [`PermissionLayer`] rather than in the caller's iteration order, so the
    /// stack cannot be reordered by whoever assembles it. The code layer
    /// establishes which capabilities are under discussion; every layer above
    /// it may only narrow one of those, never widen it and never introduce a
    /// capability of its own.
    ///
    /// # Errors
    ///
    /// Returns [`PermissionResolutionError`] on the first widening or
    /// unrequested override, leaving nothing resolved.
    pub fn resolve(
        layers: &BTreeMap<PermissionLayer, LayerDecisions>,
    ) -> Result<Self, PermissionResolutionError> {
        let mut entries: BTreeMap<String, ResolvedPermission> = BTreeMap::new();
        for layer in PermissionLayer::ALL {
            let Some(decisions) = layers.get(&layer) else {
                continue;
            };
            for (capability_ref, decision) in decisions {
                match entries.get(capability_ref) {
                    None if layer == PermissionLayer::Code => {}
                    None => {
                        return Err(PermissionResolutionError::UnrequestedOverride {
                            capability_ref: capability_ref.clone(),
                            attempted: decision.clone(),
                            attempted_layer: layer,
                        });
                    }
                    Some(held) if !held.decision.tightens_to(decision) => {
                        return Err(PermissionResolutionError::Widening {
                            capability_ref: capability_ref.clone(),
                            held: held.decision.clone(),
                            held_layer: held.layer,
                            attempted: decision.clone(),
                            attempted_layer: layer,
                        });
                    }
                    Some(_) => {}
                }
                entries.insert(
                    capability_ref.clone(),
                    ResolvedPermission {
                        decision: decision.clone(),
                        layer,
                    },
                );
            }
        }
        Ok(Self { entries })
    }

    /// Resolve the two layers that have a producer today: what the program's
    /// own source requested, narrowed by what the package shipping it states.
    ///
    /// This is the whole shipped stack, and every caller that resolves a
    /// capability's authority goes through it rather than assembling a layer
    /// map of its own — the stacking rule has exactly one implementation,
    /// [`Self::resolve`], and exactly one shipped arrangement, this one.
    ///
    /// **[`PermissionLayer::Deployment`] (precedence 30) is unwired.** Nothing
    /// in this tree states a deployment-profile decision, so no third layer is
    /// stacked here. The layer is not vestigial — [`Self::resolve`] applies it
    /// the moment a caller supplies one, and the tighten-only rule already
    /// covers it — but its absence from this constructor means *no decision was
    /// stated*, never that the deployment allows what the layers below decided.
    ///
    /// # Errors
    ///
    /// Returns [`PermissionResolutionError`] on the first widening or
    /// unrequested override, exactly as [`Self::resolve`] does.
    pub fn resolve_code_over_package(
        code: LayerDecisions,
        package: LayerDecisions,
    ) -> Result<Self, PermissionResolutionError> {
        Self::resolve(&BTreeMap::from([
            (PermissionLayer::Code, code),
            (PermissionLayer::Package, package),
        ]))
    }

    /// The winning decision and its layer for one capability.
    #[must_use]
    pub fn get(&self, capability_ref: &str) -> Option<&ResolvedPermission> {
        self.entries.get(capability_ref)
    }

    /// The layer that produced the winning decision for one capability.
    #[must_use]
    pub fn origin(&self, capability_ref: &str) -> Option<PermissionLayer> {
        self.get(capability_ref).map(|resolved| resolved.layer)
    }

    /// Every resolved capability, in capability-reference order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &ResolvedPermission)> {
        self.entries
            .iter()
            .map(|(capability_ref, resolved)| (capability_ref.as_str(), resolved))
    }

    /// Whether any capability was resolved.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many capabilities were resolved.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(
        layers: impl IntoIterator<Item = (PermissionLayer, LayerDecisions)>,
    ) -> BTreeMap<PermissionLayer, LayerDecisions> {
        layers.into_iter().collect()
    }

    fn decisions(
        entries: impl IntoIterator<Item = (&'static str, PermissionDecision)>,
    ) -> LayerDecisions {
        entries
            .into_iter()
            .map(|(name, decision)| (name.to_string(), decision))
            .collect()
    }

    /// Every ordered pair on the lattice, so a legal transition cannot become
    /// illegal (or an illegal one legal) without this failing.
    #[test]
    fn the_lattice_admits_exactly_the_tightening_transitions() {
        let ladder = [
            PermissionDecision::allow(),
            PermissionDecision::ask("gated"),
            PermissionDecision::deny("refused"),
        ];
        for (from_index, from) in ladder.iter().enumerate() {
            for (to_index, to) in ladder.iter().enumerate() {
                assert_eq!(
                    from.tightens_to(to),
                    to_index >= from_index,
                    "{from} -> {to} is misclassified"
                );
            }
        }
    }

    #[test]
    fn a_tightening_stack_resolves_and_reports_the_winning_layer_per_key() {
        let resolution = PermissionResolution::resolve(&stack([
            (
                PermissionLayer::Code,
                decisions([
                    ("cap.read", PermissionDecision::allow()),
                    ("cap.write", PermissionDecision::allow()),
                    ("cap.send", PermissionDecision::ask("confirm each send")),
                ]),
            ),
            (
                PermissionLayer::Package,
                decisions([("cap.write", PermissionDecision::ask("writes the host"))]),
            ),
            (
                PermissionLayer::Deployment,
                decisions([("cap.send", PermissionDecision::deny("no egress"))]),
            ),
        ]))
        .expect("a tightening stack resolves");

        assert_eq!(resolution.len(), 3);
        assert_eq!(
            resolution.origin("cap.read"),
            Some(PermissionLayer::Code),
            "an untouched request keeps the code layer as its origin"
        );
        assert_eq!(
            resolution.origin("cap.write"),
            Some(PermissionLayer::Package)
        );
        assert_eq!(
            resolution.origin("cap.send"),
            Some(PermissionLayer::Deployment)
        );
        assert_eq!(
            resolution.get("cap.send").map(|r| &r.decision),
            Some(&PermissionDecision::deny("no egress")),
            "the winning decision carries the reason the winning layer gave"
        );
    }

    #[test]
    fn a_widening_override_fails_closed() {
        for (held, attempted) in [
            (
                PermissionDecision::deny("no egress"),
                PermissionDecision::allow(),
            ),
            (
                PermissionDecision::deny("no egress"),
                PermissionDecision::ask("confirm"),
            ),
            (
                PermissionDecision::ask("confirm"),
                PermissionDecision::allow(),
            ),
        ] {
            let error = PermissionResolution::resolve(&stack([
                (
                    PermissionLayer::Code,
                    decisions([("cap.send", held.clone())]),
                ),
                (
                    PermissionLayer::Package,
                    decisions([("cap.send", attempted.clone())]),
                ),
            ]))
            .expect_err("a widening override is refused");
            assert_eq!(
                error,
                PermissionResolutionError::Widening {
                    capability_ref: "cap.send".to_string(),
                    held: held.clone(),
                    held_layer: PermissionLayer::Code,
                    attempted: attempted.clone(),
                    attempted_layer: PermissionLayer::Package,
                }
            );
        }
    }

    #[test]
    fn a_deployment_layer_cannot_widen_what_the_package_layer_tightened() {
        let error = PermissionResolution::resolve(&stack([
            (
                PermissionLayer::Code,
                decisions([("cap.write", PermissionDecision::allow())]),
            ),
            (
                PermissionLayer::Package,
                decisions([(
                    "cap.write",
                    PermissionDecision::deny("never in this package"),
                )]),
            ),
            (
                PermissionLayer::Deployment,
                decisions([("cap.write", PermissionDecision::allow())]),
            ),
        ]))
        .expect_err("the highest-precedence layer is still tighten-only");
        assert!(
            matches!(
                error,
                PermissionResolutionError::Widening {
                    held_layer: PermissionLayer::Package,
                    attempted_layer: PermissionLayer::Deployment,
                    ..
                }
            ),
            "{error}"
        );
    }

    #[test]
    fn an_override_for_a_capability_the_program_never_requested_fails_closed() {
        let error = PermissionResolution::resolve(&stack([
            (
                PermissionLayer::Code,
                decisions([("cap.read", PermissionDecision::allow())]),
            ),
            (
                PermissionLayer::Package,
                decisions([("cap.exfiltrate", PermissionDecision::allow())]),
            ),
        ]))
        .expect_err("an override cannot introduce a capability");
        assert_eq!(
            error,
            PermissionResolutionError::UnrequestedOverride {
                capability_ref: "cap.exfiltrate".to_string(),
                attempted: PermissionDecision::allow(),
                attempted_layer: PermissionLayer::Package,
            }
        );
    }

    /// The shipped constructor must be the two-layer stack applied through the
    /// same rule, not a second implementation of it.
    #[test]
    fn the_shipped_constructor_stacks_package_over_code_through_the_one_rule() {
        let code = decisions([
            ("cap.read", PermissionDecision::allow()),
            ("cap.write", PermissionDecision::allow()),
        ]);
        let package = decisions([("cap.write", PermissionDecision::deny("read-only surface"))]);

        let resolved =
            PermissionResolution::resolve_code_over_package(code.clone(), package.clone())
                .expect("a tightening stack resolves");
        assert_eq!(
            resolved,
            PermissionResolution::resolve(&stack([
                (PermissionLayer::Code, code.clone()),
                (PermissionLayer::Package, package.clone()),
            ]))
            .expect("the same stack through the general rule"),
            "the constructor is the general rule with the two shipped layers, nothing else"
        );
        assert_eq!(resolved.origin("cap.read"), Some(PermissionLayer::Code));
        assert_eq!(resolved.origin("cap.write"), Some(PermissionLayer::Package));

        let widened = decisions([("cap.write", PermissionDecision::allow())]);
        assert!(
            PermissionResolution::resolve_code_over_package(package, widened).is_err(),
            "the constructor inherits tighten-only, it does not relax it"
        );
    }

    #[test]
    fn a_reasonless_decision_is_wire_identical_to_its_bare_string() {
        for decision in PermissionDecision::DECISIONS {
            let effect = PermissionDecision::from_parts(decision, None).expect("closed vocabulary");
            let encoded = serde_json::to_value(&effect).expect("encode");
            assert_eq!(encoded, serde_json::json!(decision));
            let decoded: PermissionDecision = serde_json::from_value(encoded).expect("decode");
            assert_eq!(decoded, effect);
        }
    }

    #[test]
    fn a_reason_travels_with_the_decision_it_explains() {
        let effect = PermissionDecision::ask("Writes files on the host.");
        let encoded = serde_json::to_value(&effect).expect("encode");
        assert_eq!(
            encoded,
            serde_json::json!({"decision": "ask", "reason": "Writes files on the host."})
        );
        let decoded: PermissionDecision = serde_json::from_value(encoded).expect("decode");
        assert_eq!(decoded, effect);
        assert_eq!(decoded.reason(), Some("Writes files on the host."));
    }

    #[test]
    fn a_decision_outside_the_closed_vocabulary_is_refused() {
        for candidate in [
            serde_json::json!("require_approval"),
            serde_json::json!("search.read"),
            serde_json::json!({"decision": "maybe"}),
            serde_json::json!({"reason": "no decision"}),
            serde_json::json!({"decision": "allow", "grant": "root"}),
        ] {
            assert!(
                serde_json::from_value::<PermissionDecision>(candidate.clone()).is_err(),
                "{candidate} decoded into a permission decision"
            );
        }
    }
}
