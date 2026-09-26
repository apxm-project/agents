//! Opt-in, aggregate owner-local persistence timing diagnostics.

use std::io::{self, Write};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

pub use apxm_core::constants::env::APXM_PERSISTENCE_DIAGNOSTICS;

const MAX_TIMING_RECORDS_PER_PHASE: u64 = 32;

#[derive(Clone, Copy)]
pub enum PersistencePhase {
    MetadataBuild,
    MetadataEncode,
    StagedClone,
    StagedCommit,
    StoreValidate,
    StoreSerialize,
    StoreAuthenticate,
    StoreWrite,
    StoreFileSync,
    StoreReplace,
    StoreDirectorySync,
}

impl PersistencePhase {
    const fn index(self) -> usize {
        self as usize
    }

    const fn name(self) -> &'static str {
        match self {
            Self::MetadataBuild => "metadata_build",
            Self::MetadataEncode => "metadata_encode",
            Self::StagedClone => "staged_clone",
            Self::StagedCommit => "staged_commit",
            Self::StoreValidate => "store_validate",
            Self::StoreSerialize => "store_serialize",
            Self::StoreAuthenticate => "store_authenticate",
            Self::StoreWrite => "store_write",
            Self::StoreFileSync => "store_file_sync",
            Self::StoreReplace => "store_replace",
            Self::StoreDirectorySync => "store_directory_sync",
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Timing {
    count: u64,
    elapsed_us: u64,
}

static ENABLED: OnceLock<bool> = OnceLock::new();
static TIMINGS: OnceLock<Mutex<[Timing; 11]>> = OnceLock::new();
static EMISSION_ORDER: OnceLock<Mutex<()>> = OnceLock::new();

/// Time one exact phase while preserving its result and error path.
pub fn time_persistence<T>(phase: PersistencePhase, action: impl FnOnce() -> T) -> T {
    if !*ENABLED.get_or_init(|| {
        matches!(
            std::env::var(APXM_PERSISTENCE_DIAGNOSTICS).as_deref(),
            Ok("1")
        )
    }) {
        return action();
    }
    let started = Instant::now();
    let result = action();
    let elapsed_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    record_timing_with(
        TIMINGS.get_or_init(|| Mutex::new([Timing::default(); 11])),
        EMISSION_ORDER.get_or_init(|| Mutex::new(())),
        phase,
        elapsed_us,
        |record| write_record(record, &mut io::stderr().lock()),
    );
    result
}

fn record_timing_with(
    timings: &Mutex<[Timing; 11]>,
    emission_order: &Mutex<()>,
    phase: PersistencePhase,
    elapsed_us: u64,
    emit: impl FnOnce(&str),
) {
    let Ok(_order) = emission_order.lock() else {
        return;
    };
    if let Some(record) = accumulate_timing(timings, phase, elapsed_us) {
        emit(&record);
    }
}

fn write_record(record: &str, writer: &mut impl Write) {
    let _ = writeln!(writer, "{record}");
}

#[cfg(test)]
fn record_timing_into(
    timings: &Mutex<[Timing; 11]>,
    emission_order: &Mutex<()>,
    phase: PersistencePhase,
    elapsed_us: u64,
    writer: &mut impl Write,
) {
    record_timing_with(timings, emission_order, phase, elapsed_us, |record| {
        write_record(record, writer)
    });
}

fn accumulate_timing(
    timings: &Mutex<[Timing; 11]>,
    phase: PersistencePhase,
    elapsed_us: u64,
) -> Option<String> {
    let Ok(mut timings) = timings.lock() else {
        return None;
    };
    let timing = &mut timings[phase.index()];
    timing.count = timing.count.saturating_add(1);
    timing.elapsed_us = timing.elapsed_us.saturating_add(elapsed_us);
    if timing.count % 100 == 0 && timing.count / 100 <= MAX_TIMING_RECORDS_PER_PHASE {
        return Some(timing_record(phase, *timing));
    }
    None
}

fn timing_record(phase: PersistencePhase, timing: Timing) -> String {
    format!(
        "apxm.persistence-timing {{\"phase\":\"{}\",\"count\":{},\"elapsed_us\":{}}}",
        phase.name(),
        timing.count,
        timing.elapsed_us
    )
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    use super::{
        PersistencePhase, Timing, accumulate_timing, record_timing_into, record_timing_with,
        timing_record,
    };

    #[test]
    fn records_have_only_closed_phase_and_integer_counters() {
        let record = timing_record(
            PersistencePhase::StoreSerialize,
            Timing {
                count: 100,
                elapsed_us: 1234,
            },
        );
        let body = record.strip_prefix("apxm.persistence-timing ").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(parsed["phase"], "store_serialize");
        assert_eq!(parsed["count"], 100);
        assert_eq!(parsed["elapsed_us"], 1234);
        assert_eq!(parsed.as_object().unwrap().len(), 3);
    }

    #[test]
    fn output_is_capped_and_writer_failures_do_not_interrupt_counting() {
        struct RefuseWrite;
        impl Write for RefuseWrite {
            fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("diagnostic sink unavailable"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let timings = Mutex::new([Timing::default(); 11]);
        let order = Mutex::new(());
        for _ in 0..100 {
            record_timing_into(
                &timings,
                &order,
                PersistencePhase::StoreSerialize,
                1,
                &mut RefuseWrite,
            );
        }
        assert_eq!(
            timings.lock().unwrap()[PersistencePhase::StoreSerialize.index()].count,
            100
        );
        let mut output = Vec::new();
        for _ in 100..3500 {
            record_timing_into(
                &timings,
                &order,
                PersistencePhase::StoreSerialize,
                1,
                &mut output,
            );
        }
        assert_eq!(output.iter().filter(|byte| **byte == b'\n').count(), 31);
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("\"count\":3200")
        );
    }

    #[test]
    fn poisoned_timing_lock_and_counter_overflow_are_best_effort() {
        let timings = Arc::new(Mutex::new([Timing::default(); 11]));
        let order = Mutex::new(());
        let mut output = Vec::new();
        {
            let mut guard = timings.lock().unwrap();
            guard[PersistencePhase::StoreWrite.index()].count = u64::MAX - 1;
            guard[PersistencePhase::StoreWrite.index()].elapsed_us = u64::MAX - 1;
        }
        record_timing_into(
            &timings,
            &order,
            PersistencePhase::StoreWrite,
            10,
            &mut output,
        );
        assert_eq!(
            timings.lock().unwrap()[PersistencePhase::StoreWrite.index()].count,
            u64::MAX
        );
        assert_eq!(
            timings.lock().unwrap()[PersistencePhase::StoreWrite.index()].elapsed_us,
            u64::MAX
        );
        assert!(output.is_empty());

        let poisoned = Arc::clone(&timings);
        let _ = std::thread::spawn(move || {
            let _guard = poisoned.lock().unwrap();
            panic!("poison diagnostic counter");
        })
        .join();
        assert!(accumulate_timing(&timings, PersistencePhase::StoreWrite, 1).is_none());
    }

    #[test]
    fn concurrent_checkpoints_are_emitted_in_count_order() {
        let timings = Arc::new(Mutex::new([Timing::default(); 11]));
        let order = Arc::new(Mutex::new(()));
        let emitted = Arc::new(Mutex::new(Vec::new()));
        let workers = (0..4)
            .map(|_| {
                let timings = Arc::clone(&timings);
                let order = Arc::clone(&order);
                let emitted = Arc::clone(&emitted);
                std::thread::spawn(move || {
                    for _ in 0..100 {
                        record_timing_with(
                            &timings,
                            &order,
                            PersistencePhase::MetadataBuild,
                            1,
                            |record| {
                                let body = record.strip_prefix("apxm.persistence-timing ").unwrap();
                                let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
                                emitted
                                    .lock()
                                    .unwrap()
                                    .push(parsed["count"].as_u64().unwrap());
                            },
                        );
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(*emitted.lock().unwrap(), vec![100, 200, 300, 400]);
    }
}
