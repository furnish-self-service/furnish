//! System load for load-aware automatic updates: a sliding window (5 minutes at the default
//! sampling interval) of CPU,
//! memory and network use, without the load furnish itself causes.
//!
//! furnish's own load is left out in two ways. While furnish works (an install, a status check,
//! a git pull: anything holding an [`activity`] guard), samples are discarded, which also keeps
//! its downloads out of the network figure. While it is idle, the CPU and memory of its own
//! process tree (e.g. the webview) are subtracted.

use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use serde::{Deserialize, Serialize};
use sysinfo::{Networks, Pid, ProcessesToUpdate, System};

/// Samples per window: averages cover the last 30 samples (5 minutes at the default 10 s).
const WINDOW_SAMPLES: usize = 30;
/// Samples needed before deciding: a fresh start (often a busy login) doesn't count as idle.
const MIN_SAMPLES: usize = 6;

static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static TOUCHED: AtomicBool = AtomicBool::new(false);
static SAMPLES: Mutex<VecDeque<Sample>> = Mutex::new(VecDeque::new());

/// Marks furnish as working while alive; samples taken meanwhile are discarded.
pub struct Activity(());

pub fn activity() -> Activity {
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    TOUCHED.store(true, Ordering::SeqCst);
    Activity(())
}

impl Drop for Activity {
    fn drop(&mut self) {
        TOUCHED.store(true, Ordering::SeqCst);
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Upper limits for "not busy" (averages over the window).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Limits {
    pub cpu_percent: f32,
    pub memory_percent: f32,
    /// Received plus sent, in kilobytes per second.
    pub network_kbps: f64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { cpu_percent: 50.0, memory_percent: 90.0, network_kbps: 1000.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub cpu_percent: f32,
    pub memory_percent: f32,
    pub network_kbps: f64,
}

/// The current load and whether automatic updates should wait.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Load {
    /// Averages over the window (zero while there are no samples).
    pub average: Sample,
    pub samples: usize,
    /// Why updates should wait, e.g. "CPU 84%"; `None` means go ahead.
    pub busy: Option<String>,
    /// When to ask again if busy.
    pub retry_after_seconds: u64,
}

/// Decide from a window of samples (pure, for testing).
pub fn evaluate(samples: &[Sample], limits: &Limits, min_samples: usize) -> (Sample, Option<String>) {
    if samples.len() < min_samples {
        return (Sample::default(), Some("measuring how busy this computer is".into()));
    }
    let n = samples.len() as f64;
    let average = Sample {
        cpu_percent: (samples.iter().map(|s| s.cpu_percent as f64).sum::<f64>() / n) as f32,
        memory_percent: (samples.iter().map(|s| s.memory_percent as f64).sum::<f64>() / n) as f32,
        network_kbps: samples.iter().map(|s| s.network_kbps).sum::<f64>() / n,
    };
    let mut over = Vec::new();
    if average.cpu_percent > limits.cpu_percent {
        over.push(format!("CPU {:.0}%", average.cpu_percent));
    }
    if average.memory_percent > limits.memory_percent {
        over.push(format!("memory {:.0}%", average.memory_percent));
    }
    if average.network_kbps > limits.network_kbps {
        over.push(format!("network {:.1} MB/s", average.network_kbps / 1024.0));
    }
    (average, (!over.is_empty()).then(|| over.join(", ")))
}

/// The current load against `limits`. `interval` is the sampler's (see [`start_sampler`]).
pub fn current(limits: &Limits, interval: Duration) -> Load {
    let samples: Vec<Sample> = SAMPLES.lock().unwrap_or_else(|e| e.into_inner()).iter().copied().collect();
    let (average, busy) = evaluate(&samples, limits, MIN_SAMPLES);
    Load { average, samples: samples.len(), busy, retry_after_seconds: (interval * MIN_SAMPLES as u32).as_secs().max(1) }
}

/// Sample every `interval` on a background thread.
pub fn start_sampler(interval: Duration) {
    std::thread::spawn(move || {
        let (mut sys, mut nets) = (System::new(), Networks::new_with_refreshed_list());
        let Ok(me) = sysinfo::get_current_pid() else {
            return; // no load data on this platform: `current` keeps saying "measuring"
        };
        sys.refresh_cpu_usage();
        sys.refresh_processes(ProcessesToUpdate::All, true);
        loop {
            std::thread::sleep(interval);
            sys.refresh_cpu_usage();
            sys.refresh_memory();
            sys.refresh_processes(ProcessesToUpdate::All, true);
            nets.refresh(true);
            // Discard what furnish caused: anything during or since its last activity.
            if TOUCHED.swap(false, Ordering::SeqCst) || ACTIVE.load(Ordering::SeqCst) > 0 {
                continue;
            }
            let sample = measure(&sys, &nets, me, interval);
            let mut samples = SAMPLES.lock().unwrap_or_else(|e| e.into_inner());
            samples.push_back(sample);
            while samples.len() > WINDOW_SAMPLES {
                samples.pop_front();
            }
        }
    });
}

fn measure(sys: &System, nets: &Networks, me: Pid, interval: Duration) -> Sample {
    let own = own_processes(sys, me);
    let cpus = sys.cpus().len().max(1) as f32;
    // Process CPU is per core (can exceed 100%); global CPU is across all cores.
    let own_cpu: f32 = own.iter().filter_map(|p| sys.process(*p)).map(|p| p.cpu_usage()).sum::<f32>() / cpus;
    let own_memory: u64 = own.iter().filter_map(|p| sys.process(*p)).map(|p| p.memory()).sum();
    let total = sys.total_memory().max(1);
    let bytes: u64 = nets
        .iter()
        .filter(|(name, _)| !name.starts_with("lo"))
        .map(|(_, data)| data.received() + data.transmitted())
        .sum();
    Sample {
        cpu_percent: (sys.global_cpu_usage() - own_cpu).max(0.0),
        memory_percent: sys.used_memory().saturating_sub(own_memory) as f32 * 100.0 / total as f32,
        network_kbps: bytes as f64 / interval.as_secs_f64() / 1024.0,
    }
}

/// furnish's process and all its descendants.
fn own_processes(sys: &System, me: Pid) -> Vec<Pid> {
    let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
    for (pid, process) in sys.processes() {
        if let Some(parent) = process.parent() {
            children.entry(parent).or_default().push(*pid);
        }
    }
    let mut own = vec![me];
    let mut i = 0;
    while i < own.len() {
        own.extend(children.get(&own[i]).into_iter().flatten().copied());
        i += 1;
    }
    own
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(cpu: f32, memory: f32, network: f64) -> Sample {
        Sample { cpu_percent: cpu, memory_percent: memory, network_kbps: network }
    }

    #[test]
    fn waits_for_enough_samples_then_compares_averages() {
        let limits = Limits::default();
        assert!(evaluate(&[sample(1.0, 10.0, 0.0)], &limits, 6).1.is_some()); // too few samples
        let quiet = vec![sample(10.0, 40.0, 50.0); 6];
        assert_eq!(evaluate(&quiet, &limits, 6).1, None);
        // One spike doesn't make the average busy; a sustained load does.
        let mut spike = quiet.clone();
        spike[0] = sample(100.0, 40.0, 50.0);
        assert_eq!(evaluate(&spike, &limits, 6).1, None);
        let busy = vec![sample(80.0, 95.0, 3000.0); 6];
        assert_eq!(evaluate(&busy, &limits, 6).1.as_deref(), Some("CPU 80%, memory 95%, network 2.9 MB/s"));
    }

    #[test]
    fn samples_are_discarded_while_furnish_works() {
        TOUCHED.store(false, Ordering::SeqCst);
        {
            let _working = activity();
            assert!(ACTIVE.load(Ordering::SeqCst) > 0);
        }
        // The sampler swaps this flag and skips the sample when it was set.
        assert!(TOUCHED.swap(false, Ordering::SeqCst));
    }
}
