//! Owner consent: the typed request an Agent program yields to its human owner
//! and the closed answer envelope that resumes it.
//!
//! An owner request rides the existing structural yield. The program yields a
//! closed request value — its prompt, what an answer looks like (declared
//! choices or a finite JSON schema), and how long the owner has — and the host
//! reads that value from the committed yield output instead of opaque bytes.
//! The next admitted input on the same instance is the answer envelope, which
//! the runtime validates against the request before it binds the resume SSA
//! value. Nothing here is a new AIS operation, an escalation chain, or a grant
//! of Capability authority: an answer only lets the program continue.
//!
//! Two representations of one request exist and this module owns both. The
//! authored form is the frontend graph's pure `ValueExpression` for the yield's
//! `output` operand, where the prompt may reference prior SSA values. The
//! committed form is the JSON the runtime materialized and the host reads,
//! which additionally carries its `schema_version` and the compiler-computed
//! `schema_digest` of the answer schema so a host can pin exactly what it
//! asked the owner.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::frontend_graph::{ValueExpression, ValueField};
use crate::input_schema::EntrypointInputSchema;

/// The `type_ref` of the yielded request value. Only `ask_owner` produces it.
pub const OWNER_REQUEST_TYPE_REF: &str = "OwnerRequest";
/// The `type_ref` of the resume value an owner request binds.
pub const OWNER_ANSWER_TYPE_REF: &str = "OwnerAnswer";
/// The committed request document's own schema id. The compiler stamps it
/// while lowering, so a host reading a yield output can check the document
/// against the contract it claims.
pub const OWNER_REQUEST_SCHEMA_VERSION: &str = "apxm.owner-request.v1";
/// The longest an owner request may stay open: thirty days.
pub const MAX_OWNER_REQUEST_EXPIRY_SECONDS: u64 = 30 * 24 * 60 * 60;
/// The most choices one request may offer.
pub const MAX_OWNER_CHOICES: usize = 32;
/// The longest prompt, choice id, or choice label in UTF-8 bytes.
pub const MAX_OWNER_TEXT_BYTES: usize = 4096;

/// One selectable answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerChoice {
    pub id: String,
    pub label: String,
}

/// What an answer to one request looks like.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum OwnerAnswerSchema {
    /// The owner picks exactly one declared choice; the answer is its id.
    Choice { choices: Vec<OwnerChoice> },
    /// The owner supplies a value of the declared finite JSON type.
    Typed { schema: EntrypointInputSchema },
}

/// The committed request value the host reads from a yield output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerRequest {
    pub schema_version: String,
    pub prompt: String,
    pub answer: OwnerAnswerSchema,
    pub expires_in_seconds: u64,
    /// Content address of `answer`; the compiler stamps it while lowering.
    pub schema_digest: String,
}

/// The closed envelope an owner request resumes with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum OwnerAnswer {
    Answered { answer: Value },
    Declined,
    Expired,
}

impl OwnerAnswerSchema {
    /// Reject empty, oversized, or ambiguous answer shapes.
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Choice { choices } => {
                if choices.is_empty() {
                    return Err("owner request choices must not be empty");
                }
                if choices.len() > MAX_OWNER_CHOICES {
                    return Err("owner request offers too many choices");
                }
                let mut ids = BTreeSet::new();
                for choice in choices {
                    validate_text(&choice.id, "owner choice id")?;
                    validate_text(&choice.label, "owner choice label")?;
                    if !ids.insert(choice.id.as_str()) {
                        return Err("owner choice ids must be unique");
                    }
                }
                Ok(())
            }
            Self::Typed { schema } => schema.validate(),
        }
    }

    /// Content address of the validated answer schema with sorted object keys.
    pub fn canonical_digest(&self) -> Result<String, &'static str> {
        self.validate()?;
        let value = serde_json::to_value(self).map_err(|_| "answer schema serialization failed")?;
        let bytes =
            serde_json::to_vec(&sorted(value)).map_err(|_| "answer schema serialization failed")?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }

    /// Check one answered value against this schema without coercion.
    pub fn validate_answer(&self, answer: &Value) -> Result<(), &'static str> {
        match self {
            Self::Choice { choices } => {
                let id = answer
                    .as_str()
                    .ok_or("an owner choice answer is the chosen choice id")?;
                if choices.iter().any(|choice| choice.id == id) {
                    Ok(())
                } else {
                    Err("owner answer names no declared choice")
                }
            }
            Self::Typed { schema } => schema.validate_value(answer),
        }
    }
}

impl OwnerRequest {
    /// Decode the committed request value the runtime materialized.
    pub fn decode(value: &Value) -> Result<Self, &'static str> {
        let request: Self =
            serde_json::from_value(value.clone()).map_err(|_| "owner request is malformed")?;
        request.validate()?;
        Ok(request)
    }

    /// Reject a request outside the closed shape or with a forged digest.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != OWNER_REQUEST_SCHEMA_VERSION {
            return Err("owner request schema_version must be apxm.owner-request.v1");
        }
        validate_text(&self.prompt, "owner request prompt")?;
        validate_expiry(self.expires_in_seconds)?;
        self.answer.validate()?;
        if self.schema_digest != self.answer.canonical_digest()? {
            return Err("owner request schema digest does not match its answer schema");
        }
        Ok(())
    }
}

/// Validate the delivered answer envelope against the committed request.
///
/// Every outcome is explicit: an answered envelope must carry a value the
/// request's schema admits; declined and expired carry nothing. The runtime has
/// no clock of its own here — the host decides when a request expired and says
/// so through the envelope.
pub fn validate_owner_answer(
    request: &Value,
    delivered: &Value,
) -> Result<OwnerAnswer, &'static str> {
    let request = OwnerRequest::decode(request)?;
    let envelope = decode_owner_answer(delivered)?;
    if let OwnerAnswer::Answered { answer } = &envelope {
        request.answer.validate_answer(answer)?;
    }
    Ok(envelope)
}

/// Decode the answer envelope by hand: an internally tagged serde enum lets a
/// unit variant carry stray fields, and a declined answer that also carries a
/// value is exactly the ambiguity the closed envelope exists to refuse.
fn decode_owner_answer(delivered: &Value) -> Result<OwnerAnswer, &'static str> {
    let object = delivered
        .as_object()
        .ok_or("owner answer is not a closed outcome envelope")?;
    let outcome = object
        .get("outcome")
        .and_then(Value::as_str)
        .ok_or("owner answer states its outcome")?;
    match outcome {
        "answered" => match (object.len(), object.get("answer")) {
            (2, Some(answer)) => Ok(OwnerAnswer::Answered {
                answer: answer.clone(),
            }),
            _ => Err("an answered owner envelope carries exactly its answer"),
        },
        "declined" | "expired" if object.len() == 1 => Ok(if outcome == "declined" {
            OwnerAnswer::Declined
        } else {
            OwnerAnswer::Expired
        }),
        "declined" | "expired" => Err("a declined or expired owner envelope carries no answer"),
        _ => Err("owner answer outcome is not answered, declined, or expired"),
    }
}

/// Verify the authored request expression the frontend captured for a yield.
///
/// The prompt may be a string literal or a reference to a prior value; the
/// answer shape and the expiry must be literal so the compiler can address the
/// schema before anything runs. The graph never carries `schema_version` or
/// `schema_digest`: the compiler stamps both while lowering, so an authored
/// one is a forgery.
pub fn verify_owner_request_expression(expression: &ValueExpression) -> Result<(), &'static str> {
    let ValueExpression::Object { fields } = expression else {
        return Err("owner request must be an object expression");
    };
    let mut seen = BTreeSet::new();
    for field in fields {
        if !seen.insert(field.name.as_str()) {
            return Err("owner request repeats a field");
        }
    }
    let field = |name: &str| {
        fields
            .iter()
            .find(|field| field.name == name)
            .map(|field| &field.value)
    };
    match field("prompt") {
        Some(ValueExpression::String { value }) => validate_text(value, "owner request prompt")?,
        Some(
            ValueExpression::Ssa { .. }
            | ValueExpression::Projection { .. }
            | ValueExpression::Context { .. },
        ) => {}
        _ => return Err("owner request prompt must be a string or a prior value"),
    }
    match field("expires_in_seconds") {
        Some(ValueExpression::Integer { value }) => {
            validate_expiry(
                u64::try_from(*value).map_err(|_| "owner request expiry must be positive")?,
            )?;
        }
        _ => return Err("owner request expires_in_seconds must be an integer literal"),
    }
    let answer = field("answer").ok_or("owner request declares its answer shape")?;
    let literal = literal_json(answer).ok_or("owner request answer shape must be literal")?;
    let schema: OwnerAnswerSchema =
        serde_json::from_value(literal).map_err(|_| "owner request answer shape is malformed")?;
    schema.validate()?;
    if seen.len() != 3 {
        return Err("owner request admits only prompt, answer, and expires_in_seconds");
    }
    Ok(())
}

/// The lowered request expression: the verified authored fields plus the
/// document's `schema_version` and the compiler-computed `schema_digest` of
/// the answer shape.
pub fn owner_request_expression_with_digest(
    expression: &ValueExpression,
) -> Result<ValueExpression, &'static str> {
    verify_owner_request_expression(expression)?;
    let ValueExpression::Object { fields } = expression else {
        unreachable!("verified owner request is an object expression");
    };
    let answer = fields
        .iter()
        .find(|field| field.name == "answer")
        .and_then(|field| literal_json(&field.value))
        .ok_or("owner request answer shape must be literal")?;
    let schema: OwnerAnswerSchema =
        serde_json::from_value(answer).map_err(|_| "owner request answer shape is malformed")?;
    let mut fields = fields.clone();
    fields.push(ValueField {
        name: "schema_version".to_owned(),
        value: ValueExpression::String {
            value: OWNER_REQUEST_SCHEMA_VERSION.to_owned(),
        },
    });
    fields.push(ValueField {
        name: "schema_digest".to_owned(),
        value: ValueExpression::String {
            value: schema.canonical_digest()?,
        },
    });
    Ok(ValueExpression::Object { fields })
}

/// A pure literal expression as JSON; `None` when it references anything.
pub fn literal_json(expression: &ValueExpression) -> Option<Value> {
    Some(match expression {
        ValueExpression::String { value } => Value::String(value.clone()),
        ValueExpression::Integer { value } => Value::from(*value),
        ValueExpression::Boolean { value } => Value::Bool(*value),
        ValueExpression::Null => Value::Null,
        ValueExpression::Array { items } => {
            Value::Array(items.iter().map(literal_json).collect::<Option<Vec<_>>>()?)
        }
        ValueExpression::Object { fields } => {
            let mut object = serde_json::Map::new();
            for field in fields {
                if object
                    .insert(field.name.clone(), literal_json(&field.value)?)
                    .is_some()
                {
                    return None;
                }
            }
            Value::Object(object)
        }
        ValueExpression::Ssa { .. }
        | ValueExpression::Projection { .. }
        | ValueExpression::Context { .. } => return None,
    })
}

fn validate_text(text: &str, what: &'static str) -> Result<(), &'static str> {
    if text.is_empty() {
        return Err(match what {
            "owner request prompt" => "owner request prompt must not be empty",
            "owner choice id" => "owner choice id must not be empty",
            _ => "owner choice label must not be empty",
        });
    }
    if text.len() > MAX_OWNER_TEXT_BYTES {
        return Err(match what {
            "owner request prompt" => "owner request prompt is too long",
            "owner choice id" => "owner choice id is too long",
            _ => "owner choice label is too long",
        });
    }
    Ok(())
}

fn validate_expiry(seconds: u64) -> Result<(), &'static str> {
    if seconds == 0 {
        return Err("owner request expiry must be positive");
    }
    if seconds > MAX_OWNER_REQUEST_EXPIRY_SECONDS {
        return Err("owner request expiry exceeds thirty days");
    }
    Ok(())
}

fn sorted(value: Value) -> Value {
    match value {
        Value::Object(fields) => {
            let fields: BTreeMap<_, _> = fields.into_iter().collect();
            Value::Object(
                fields
                    .into_iter()
                    .map(|(key, value)| (key, sorted(value)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn choice_request() -> ValueExpression {
        serde_json::from_value(json!({
            "kind": "object",
            "fields": [
                {"name": "prompt", "value": {"kind": "string", "value": "Send the quote?"}},
                {"name": "answer", "value": {"kind": "object", "fields": [
                    {"name": "mode", "value": {"kind": "string", "value": "choice"}},
                    {"name": "choices", "value": {"kind": "array", "items": [
                        {"kind": "object", "fields": [
                            {"name": "id", "value": {"kind": "string", "value": "send"}},
                            {"name": "label", "value": {"kind": "string", "value": "Send it"}}
                        ]},
                        {"kind": "object", "fields": [
                            {"name": "id", "value": {"kind": "string", "value": "hold"}},
                            {"name": "label", "value": {"kind": "string", "value": "Hold"}}
                        ]}
                    ]}}
                ]}},
                {"name": "expires_in_seconds", "value": {"kind": "integer", "value": 3600}}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn a_choice_request_verifies_and_lowers_with_a_stable_digest() {
        let expression = choice_request();
        verify_owner_request_expression(&expression).unwrap();
        let lowered = owner_request_expression_with_digest(&expression).unwrap();
        let committed = literal_json(&lowered).unwrap();
        let request = OwnerRequest::decode(&committed).unwrap();
        let digest = request.schema_digest.clone();
        assert_eq!(request.schema_version, OWNER_REQUEST_SCHEMA_VERSION);
        assert!(digest.starts_with("sha256:") && digest.len() == 71);
        assert_eq!(request.answer.canonical_digest().unwrap(), digest);
        // Field order never changes the digest.
        let reordered = json!({"choices": [{"label": "Send it", "id": "send"}, {"id": "hold", "label": "Hold"}], "mode": "choice"});
        let reordered: OwnerAnswerSchema = serde_json::from_value(reordered).unwrap();
        assert_eq!(reordered.canonical_digest().unwrap(), digest);
    }

    #[test]
    fn a_dynamic_prompt_is_admitted_but_a_dynamic_answer_shape_is_not() {
        let ValueExpression::Object { mut fields } = choice_request() else {
            unreachable!()
        };
        fields[0].value = ValueExpression::Ssa {
            value_id: "value.prompt".into(),
        };
        verify_owner_request_expression(&ValueExpression::Object {
            fields: fields.clone(),
        })
        .unwrap();
        fields[1].value = ValueExpression::Ssa {
            value_id: "value.answer".into(),
        };
        assert!(verify_owner_request_expression(&ValueExpression::Object { fields }).is_err());
    }

    #[test]
    fn forged_digests_and_extra_fields_are_refused() {
        let ValueExpression::Object { mut fields } = choice_request() else {
            unreachable!()
        };
        fields.push(ValueField {
            name: "schema_digest".into(),
            value: ValueExpression::String {
                value: "sha256:00".into(),
            },
        });
        assert!(verify_owner_request_expression(&ValueExpression::Object { fields }).is_err());
        let lowered = owner_request_expression_with_digest(&choice_request()).unwrap();
        let mut committed = literal_json(&lowered).unwrap();
        committed["schema_digest"] =
            json!("sha256:0000000000000000000000000000000000000000000000000000000000000000");
        assert!(OwnerRequest::decode(&committed).is_err());
    }

    #[test]
    fn answers_are_validated_against_the_request() {
        let committed =
            literal_json(&owner_request_expression_with_digest(&choice_request()).unwrap())
                .unwrap();
        assert_eq!(
            validate_owner_answer(
                &committed,
                &json!({"outcome": "answered", "answer": "send"})
            )
            .unwrap(),
            OwnerAnswer::Answered {
                answer: json!("send")
            }
        );
        assert_eq!(
            validate_owner_answer(&committed, &json!({"outcome": "declined"})).unwrap(),
            OwnerAnswer::Declined
        );
        assert_eq!(
            validate_owner_answer(&committed, &json!({"outcome": "expired"})).unwrap(),
            OwnerAnswer::Expired
        );
        for rejected in [
            json!({"outcome": "answered", "answer": "burn"}),
            json!({"outcome": "answered"}),
            json!({"outcome": "declined", "answer": "send"}),
            json!({"outcome": "later"}),
            json!("send"),
            json!({"message": "send"}),
        ] {
            assert!(
                validate_owner_answer(&committed, &rejected).is_err(),
                "{rejected}"
            );
        }
    }

    #[test]
    fn typed_answers_use_the_finite_input_schema() {
        let schema: OwnerAnswerSchema = serde_json::from_value(json!({"mode": "typed", "schema": {"type": "object", "properties": {"amount": {"type": "integer"}}, "required": ["amount"], "additionalProperties": false}})).unwrap();
        let typed = json!({
            "schema_version": OWNER_REQUEST_SCHEMA_VERSION,
            "prompt": "How much?",
            "answer": schema,
            "expires_in_seconds": 60,
            "schema_digest": schema.canonical_digest().unwrap()
        });
        assert!(
            validate_owner_answer(
                &typed,
                &json!({"outcome": "answered", "answer": {"amount": 3}})
            )
            .is_ok()
        );
        assert!(
            validate_owner_answer(
                &typed,
                &json!({"outcome": "answered", "answer": {"amount": "3"}})
            )
            .is_err()
        );
        assert!(
            validate_owner_answer(
                &typed,
                &json!({"outcome": "answered", "answer": {"amount": 3, "extra": true}})
            )
            .is_err()
        );
    }

    #[test]
    fn bounds_are_closed() {
        let mut request = OwnerRequest::decode(
            &literal_json(&owner_request_expression_with_digest(&choice_request()).unwrap())
                .unwrap(),
        )
        .unwrap();
        request.expires_in_seconds = 0;
        assert!(request.validate().is_err());
        request.expires_in_seconds = MAX_OWNER_REQUEST_EXPIRY_SECONDS + 1;
        assert!(request.validate().is_err());
        request.expires_in_seconds = 1;
        request.prompt.clear();
        assert!(request.validate().is_err());
        request.prompt = "ok".into();
        request.answer = OwnerAnswerSchema::Choice {
            choices: vec![
                OwnerChoice {
                    id: "a".into(),
                    label: "A".into(),
                },
                OwnerChoice {
                    id: "a".into(),
                    label: "B".into(),
                },
            ],
        };
        assert!(request.validate().is_err());
        request.answer = OwnerAnswerSchema::Choice {
            choices: Vec::new(),
        };
        assert!(request.validate().is_err());
    }
}
