//! `schedule` — a native, durable, event-driven scheduling tool.
//!
//! `create` arms a wakeup (one-shot via `after_secs`/`at_ms`, or recurring via
//! `every_secs`), persisted in the [`ToolsStore`] so it survives a restart. The
//! tool returns immediately; firing is done by a background [`spawn_firer`] task
//! that, when a schedule comes due, delivers through the host-provided
//! [`CapabilityHost`] wake bridge and the host-provided `OnFire` hook, then
//! advances/retires the row. `apxm-server` uses the hook to enqueue prompt
//! wakeups into its existing CLAIM task queue.
//!
//! The tool never blocks waiting for the fire time — a 30s capability invoke
//! timeout forbids it and a tool must not hold a worker lane.

use std::collections::HashMap;
use std::sync::Arc;

use apxm_capability_iface::CapabilityHost;
use apxm_core::constants::capabilities::groups;
use apxm_core::types::values::{Number, Value};
use async_trait::async_trait;
use tokio::sync::Notify;
use uuid::Uuid;

use super::store::{ScheduleRow, ToolsStore, now_ms};
use crate::capability::{
    executor::{CapabilityExecutor, CapabilityResult},
    metadata::RuntimeCapability,
};

const CAP: &str = apxm_core::constants::capabilities::SCHEDULE;
const PAYLOAD_QUEUE: &str = apxm_core::constants::agent_tools::PAYLOAD_QUEUE;
const SCHEDULED_PROMPT_QUEUE: &str = apxm_core::constants::agent_tools::SCHEDULED_PROMPT_QUEUE;

/// Native event-driven scheduling capability.
pub struct ScheduleCapability {
    metadata: RuntimeCapability,
    store: ToolsStore,
    /// Notifies the firer to re-evaluate its next sleep when a schedule is armed.
    arm: Arc<Notify>,
}

impl ScheduleCapability {
    pub fn new(store: ToolsStore, arm: Arc<Notify>) -> Self {
        Self {
            metadata: RuntimeCapability::new(
                CAP,
                "Arm a durable wakeup: one-shot via 'after_secs' or 'at_ms', or \
                 recurring via 'every_secs'. Actions: create, list, get, cancel. \
                 Survives process restarts; fires through the runtime wake bridge.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["create", "list", "get", "cancel"] },
                        "schedule_id": { "type": "string" },
                        "when": {
                            "type": "object",
                            "properties": {
                                "after_secs": { "type": "integer", "minimum": 0 },
                                "at_ms": { "type": "integer", "minimum": 0 },
                                "every_secs": { "type": "integer", "minimum": 1 },
                                "cron": {
                                    "type": "string",
                                    "description": "5-field cron (min hour dom month dow), UTC"
                                }
                            }
                        },
                        "prompt": { "type": "string" },
                        (PAYLOAD_QUEUE): {
                            "type": "string",
                            "description": format!(
                                "Optional task queue for server prompt delivery; defaults to {SCHEDULED_PROMPT_QUEUE}"
                            )
                        },
                        "payload": {
                            "type": "object",
                            "description": format!(
                                "JSON payload delivered on fire. A payload.{PAYLOAD_QUEUE} string also selects the task queue."
                            )
                        }
                    },
                    "required": ["action"]
                }),
            )
            .with_returns("object")
            .with_groups(vec![
                groups::AGENT_MANAGEMENT.to_string(),
                groups::AGENT_MANAGEMENT.to_string(),
            ]),
            store,
            arm,
        }
    }

    fn err(&self, message: impl Into<String>) -> apxm_core::error::RuntimeError {
        apxm_core::error::RuntimeError::Capability {
            capability: CAP.to_string(),
            message: message.into(),
        }
    }

    fn do_create(&self, args: &HashMap<String, Value>) -> CapabilityResult<Value> {
        let when = args
            .get("when")
            .and_then(|v| v.as_object())
            .ok_or_else(|| self.err("'create' requires a 'when' object"))?;
        let after_secs = when.get("after_secs").and_then(|v| v.as_i64());
        let at_ms = when.get("at_ms").and_then(|v| v.as_i64());
        let every_secs = when.get("every_secs").and_then(|v| v.as_i64());
        let cron = when
            .get("cron")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let specified = [
            after_secs.is_some(),
            at_ms.is_some(),
            every_secs.is_some(),
            cron.is_some(),
        ]
        .iter()
        .filter(|b| **b)
        .count();
        if specified != 1 {
            return Err(self
                .err("'when' must specify exactly one of after_secs, at_ms, every_secs, or cron"));
        }

        let now = now_ms();
        let (kind, recurring, next_fire_ms, cron_stored) = if let Some(expr) = &cron {
            let next = cron::next_after(expr, now)
                .map_err(|e| self.err(format!("invalid cron expression: {e}")))?;
            ("cron", true, next, Some(expr.clone()))
        } else if let Some(secs) = every_secs {
            if secs < 1 {
                return Err(self.err("every_secs must be >= 1"));
            }
            ("recurring", true, now + secs * 1000, None)
        } else if let Some(secs) = after_secs {
            ("once", false, now + secs.max(0) * 1000, None)
        } else {
            ("once", false, at_ms.unwrap(), None)
        };

        let mut payload_json = match args.get("payload") {
            Some(v) => serde_json::to_value(v).unwrap_or_else(|_| serde_json::json!({})),
            None => serde_json::json!({}),
        };
        if let Some(queue) = args.get(PAYLOAD_QUEUE).and_then(|v| v.as_str()) {
            let obj = payload_json
                .as_object_mut()
                .ok_or_else(|| self.err("'payload' must be an object when 'queue' is set"))?;
            obj.insert(
                PAYLOAD_QUEUE.to_string(),
                serde_json::Value::String(queue.to_string()),
            );
        }
        let payload = serde_json::to_string(&payload_json).unwrap_or_else(|_| "{}".to_string());
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let row = ScheduleRow {
            id: Uuid::now_v7().to_string(),
            kind: kind.to_string(),
            every_secs,
            cron: cron_stored,
            next_fire_ms,
            recurring,
            prompt,
            payload,
            status: "armed".to_string(),
            created_at_ms: now,
            last_fired_ms: None,
        };
        self.store.upsert_schedule(&row).map_err(|e| self.err(e))?;
        // Nudge the firer so it re-evaluates its next sleep deadline.
        self.arm.notify_one();

        let mut obj = HashMap::new();
        obj.insert("schedule_id".to_string(), Value::String(row.id));
        obj.insert(
            "next_fire_ms".to_string(),
            Value::Number(Number::Integer(row.next_fire_ms)),
        );
        obj.insert("recurring".to_string(), Value::Bool(row.recurring));
        Ok(Value::Object(obj))
    }

    fn do_list(&self) -> CapabilityResult<Value> {
        let rows = self.store.list_schedules().map_err(|e| self.err(e))?;
        let schedules: Vec<Value> = rows.iter().map(schedule_value).collect();
        let mut obj = HashMap::new();
        obj.insert("schedules".to_string(), Value::Array(schedules));
        Ok(Value::Object(obj))
    }

    fn do_get(&self, args: &HashMap<String, Value>) -> CapabilityResult<Value> {
        let id = args
            .get("schedule_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| self.err("'get' requires 'schedule_id'"))?;
        match self.store.get_schedule(id).map_err(|e| self.err(e))? {
            Some(row) => Ok(schedule_value(&row)),
            None => Err(self.err(format!("schedule '{id}' not found"))),
        }
    }

    fn do_cancel(&self, args: &HashMap<String, Value>) -> CapabilityResult<Value> {
        let id = args
            .get("schedule_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| self.err("'cancel' requires 'schedule_id'"))?;
        let cancelled = self.store.cancel_schedule(id).map_err(|e| self.err(e))?;
        let mut obj = HashMap::new();
        obj.insert("cancelled".to_string(), Value::Bool(cancelled));
        Ok(Value::Object(obj))
    }
}

#[async_trait]
impl CapabilityExecutor for ScheduleCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| self.err("missing required 'action'"))?;
        match action {
            "create" => self.do_create(&args),
            "list" => self.do_list(),
            "get" => self.do_get(&args),
            "cancel" => self.do_cancel(&args),
            other => Err(self.err(format!("unknown action '{other}'"))),
        }
    }

    fn metadata(&self) -> &RuntimeCapability {
        &self.metadata
    }
}

fn schedule_value(row: &ScheduleRow) -> Value {
    let mut obj = HashMap::new();
    obj.insert("id".to_string(), Value::String(row.id.clone()));
    obj.insert("kind".to_string(), Value::String(row.kind.clone()));
    obj.insert(
        "next_fire_ms".to_string(),
        Value::Number(Number::Integer(row.next_fire_ms)),
    );
    obj.insert("recurring".to_string(), Value::Bool(row.recurring));
    obj.insert("status".to_string(), Value::String(row.status.clone()));
    obj.insert(
        "prompt".to_string(),
        match &row.prompt {
            Some(p) => Value::String(p.clone()),
            None => Value::Null,
        },
    );
    Value::Object(obj)
}

/// A delivered schedule fire, handed to the optional [`OnFire`] callback so a
/// host (the server) can route the prompt/payload into agent work or its
/// observability stream. Park-registry waiters are woken independently.
#[derive(Debug, Clone)]
pub struct FiredSchedule {
    pub id: String,
    pub kind: String,
    pub recurring: bool,
    pub prompt: Option<String>,
    pub payload: String,
}

/// Host-provided delivery hook invoked once per fired schedule.
pub type OnFire = Arc<dyn Fn(FiredSchedule) + Send + Sync>;

/// Compute the next fire time for a recurring row given the wall clock, avoiding
/// a burst of immediate re-fires when the firer was behind. Cron rows recompute
/// from their expression; fixed-interval rows step by `every_secs`.
fn advance_recurring(row: &ScheduleRow, now: i64) -> Option<i64> {
    if let Some(expr) = &row.cron {
        return cron::next_after(expr, now).ok();
    }
    let secs = row.every_secs?;
    let step = secs * 1000;
    if step <= 0 {
        return None;
    }
    let mut next = row.next_fire_ms + step;
    while next <= now {
        next += step;
    }
    Some(next)
}

/// Fire every schedule that is due as of now. Delivers each row's payload
/// through the [`CapabilityHost`]'s wake-notification bridge and the optional
/// `on_fire` hook, then retires one-shots and re-arms recurring rows. Returns
/// the number of schedules fired.
///
/// Takes `host: &dyn CapabilityHost` rather than calling
/// `crate::scheduler::park_registry::wake` directly — this is capability's
/// one touchpoint on the scheduler, narrowed to the trait `apxm-runtime`'s
/// scheduler implements (see [`crate::scheduler::park_registry::ParkRegistryHost`]).
pub fn fire_due(
    store: &ToolsStore,
    host: &dyn CapabilityHost,
    on_fire: Option<&OnFire>,
) -> usize {
    let now = now_ms();
    let due = store.due_schedules(now).unwrap_or_default();
    let mut fired = 0;
    for row in due {
        let value = parse_payload(&row.payload);
        let _woken = host.wake(&row.id, value);
        if let Some(cb) = on_fire {
            cb(FiredSchedule {
                id: row.id.clone(),
                kind: row.kind.clone(),
                recurring: row.recurring,
                prompt: row.prompt.clone(),
                payload: row.payload.clone(),
            });
        }
        let next = if row.recurring {
            advance_recurring(&row, now)
        } else {
            None
        };
        if store.record_fire(&row.id, now, next).is_ok() {
            fired += 1;
        }
    }
    fired
}

fn parse_payload(payload: &str) -> Value {
    serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .and_then(|j| Value::try_from(j).ok())
        .unwrap_or(Value::Null)
}

/// Spawn the background firer. It sleeps until the earliest armed schedule is
/// due (or until `arm` is notified that a new schedule was created), fires due
/// schedules, and repeats. Returns the task handle.
pub fn spawn_firer(
    store: ToolsStore,
    host: Arc<dyn CapabilityHost>,
    arm: Arc<Notify>,
    on_fire: Option<OnFire>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let next = store.next_armed_fire_ms().ok().flatten();
            match next {
                None => {
                    // Nothing armed — wait until something is.
                    arm.notified().await;
                }
                Some(ts) => {
                    let now = now_ms();
                    if ts <= now {
                        fire_due(&store, host.as_ref(), on_fire.as_ref());
                    } else {
                        let wait = std::time::Duration::from_millis((ts - now) as u64);
                        tokio::select! {
                            () = tokio::time::sleep(wait) => { fire_due(&store, host.as_ref(), on_fire.as_ref()); }
                            () = arm.notified() => { /* re-evaluate with the new schedule */ }
                        }
                    }
                }
            }
        }
    })
}

/// Minimal, dependency-free 5-field cron evaluator (UTC).
///
/// Supports `*`, single values, `a-b` ranges, comma lists, and `*/n` / `a-b/n`
/// steps for fields: minute(0-59) hour(0-23) day-of-month(1-31) month(1-12)
/// day-of-week(0-6, Sun=0; 7 also accepted as Sunday). Day-of-month and
/// day-of-week combine with Vixie-cron OR semantics when both are restricted.
mod cron {
    use chrono::{Datelike, Duration, TimeZone, Timelike, Utc};
    use std::collections::BTreeSet;

    struct Field {
        allowed: BTreeSet<u32>,
        star: bool,
    }

    fn parse_field(spec: &str, min: u32, max: u32) -> Result<Field, String> {
        let star = spec.trim() == "*";
        let mut set = BTreeSet::new();
        for part in spec.split(',') {
            let (range_part, step) = match part.split_once('/') {
                Some((r, s)) => (
                    r,
                    s.parse::<u32>()
                        .map_err(|_| format!("bad step in '{part}'"))?,
                ),
                None => (part, 1),
            };
            if step == 0 {
                return Err(format!("zero step in '{part}'"));
            }
            let (lo, hi) = if range_part == "*" {
                (min, max)
            } else if let Some((a, b)) = range_part.split_once('-') {
                (
                    a.parse::<u32>()
                        .map_err(|_| format!("bad range '{part}'"))?,
                    b.parse::<u32>()
                        .map_err(|_| format!("bad range '{part}'"))?,
                )
            } else {
                let v: u32 = range_part
                    .parse()
                    .map_err(|_| format!("bad value '{part}'"))?;
                (v, v)
            };
            if lo < min || hi > max || lo > hi {
                return Err(format!("'{part}' out of range {min}-{max}"));
            }
            let mut v = lo;
            while v <= hi {
                set.insert(v);
                v += step;
            }
        }
        if set.is_empty() {
            return Err(format!("empty field '{spec}'"));
        }
        Ok(Field { allowed: set, star })
    }

    fn parse_day_of_week(spec: &str) -> Result<Field, String> {
        let mut field = parse_field(spec, 0, 7)?;
        if field.allowed.remove(&7) {
            field.allowed.insert(0);
        }
        Ok(field)
    }

    /// Next fire time (epoch ms) strictly after `after_ms`, or an error if the
    /// expression is malformed or matches nothing within ~1 year.
    pub fn next_after(expr: &str, after_ms: i64) -> Result<i64, String> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(format!("cron needs 5 fields, got {}", fields.len()));
        }
        let minute = parse_field(fields[0], 0, 59)?;
        let hour = parse_field(fields[1], 0, 23)?;
        let dom = parse_field(fields[2], 1, 31)?;
        let month = parse_field(fields[3], 1, 12)?;
        let dow = parse_day_of_week(fields[4])?;

        let start = Utc
            .timestamp_millis_opt(after_ms)
            .single()
            .ok_or_else(|| "invalid timestamp".to_string())?;
        let mut t = (start + Duration::minutes(1))
            .with_second(0)
            .and_then(|t| t.with_nanosecond(0))
            .ok_or_else(|| "time truncation failed".to_string())?;

        let limit = 366 * 24 * 60 + 60;
        for _ in 0..limit {
            let day_ok = match (dom.star, dow.star) {
                (true, true) => true,
                (false, true) => dom.allowed.contains(&t.day()),
                (true, false) => dow.allowed.contains(&t.weekday().num_days_from_sunday()),
                (false, false) => {
                    dom.allowed.contains(&t.day())
                        || dow.allowed.contains(&t.weekday().num_days_from_sunday())
                }
            };
            if month.allowed.contains(&t.month())
                && day_ok
                && hour.allowed.contains(&t.hour())
                && minute.allowed.contains(&t.minute())
            {
                return Ok(t.timestamp_millis());
            }
            t += Duration::minutes(1);
        }
        Err(format!("no cron match within a year for '{expr}'"))
    }
}

#[cfg(test)]
mod fire_due_tests {
    use super::*;
    use parking_lot::Mutex;

    /// Records every `wake` call instead of touching the real park registry —
    /// proves `fire_due` goes through `CapabilityHost` and never needs to name
    /// `apxm-runtime`'s scheduler module directly.
    #[derive(Default)]
    struct RecordingHost {
        woken: Mutex<Vec<(String, Value)>>,
    }

    impl CapabilityHost for RecordingHost {
        fn wake(&self, wait_key: &str, value: Value) -> usize {
            self.woken.lock().push((wait_key.to_string(), value));
            1
        }
    }

    fn armed_row(id: &str, due_ms: i64) -> ScheduleRow {
        ScheduleRow {
            id: id.to_string(),
            kind: "once".to_string(),
            every_secs: None,
            cron: None,
            next_fire_ms: due_ms,
            recurring: false,
            prompt: None,
            payload: "{}".to_string(),
            status: "armed".to_string(),
            created_at_ms: now_ms(),
            last_fired_ms: None,
        }
    }

    #[test]
    fn fire_due_wakes_through_capability_host_not_park_registry_directly() {
        let store = ToolsStore::in_memory().expect("in-memory store");
        store
            .upsert_schedule(&armed_row("sched-1", now_ms() - 1_000))
            .expect("arm schedule");
        let host = RecordingHost::default();

        let fired = fire_due(&store, &host, None);

        assert_eq!(fired, 1);
        let woken = host.woken.lock();
        assert_eq!(woken.len(), 1);
        assert_eq!(woken[0].0, "sched-1");
    }

    #[test]
    fn fire_due_is_noop_when_nothing_is_due() {
        let store = ToolsStore::in_memory().expect("in-memory store");
        store
            .upsert_schedule(&armed_row("sched-future", now_ms() + 60_000))
            .expect("arm schedule");
        let host = RecordingHost::default();

        let fired = fire_due(&store, &host, None);

        assert_eq!(fired, 0);
        assert!(host.woken.lock().is_empty());
    }
}
