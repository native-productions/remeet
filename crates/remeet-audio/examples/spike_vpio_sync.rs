//! Fase 1 spike for the VPIO clean-mic rewrite. Answers the two questions the plan
//! flagged as the riskiest, before any capture code is touched:
//!
//!   1. Can we enumerate input devices and select one? (the "pick your mic" feature)
//!   2. Do the mic's VPIO buffer timestamps live on the same clock as ScreenCaptureKit's
//!      audio PTS, so the two engines can be aligned by a common time base?
//!
//! Both are answered without ScreenCaptureKit (so no Screen Recording prompt): device
//! selection is pure CoreAudio, and the clock question reduces to "is the AVAudioTime
//! host time the mach host time?" — which we check by converting a tap timestamp with
//! `cm::Clock::make_host_time_from_sys_units` (the same host-time clock SCK PTS uses)
//! and comparing it to the host clock's own reading of "now".
//!
//! Run: `cargo run -p remeet-audio --example spike_vpio_sync`

use std::sync::{Arc, Mutex};
use std::time::Duration;

use cidre::{av, cm, core_audio as ca};

fn main() {
    enumerate_devices();
    prove_selectable();
    probe_clock();
}

/// (1a) List input-capable devices — name, uid, channel count — the raw material for a
/// Settings dropdown.
fn enumerate_devices() {
    println!("== input devices ==");
    let default = ca::System::default_input_device().ok();
    let devices = match ca::System::devices() {
        Ok(d) => d,
        Err(e) => {
            println!("  could not list devices: {e:?}");
            return;
        }
    };

    for d in &devices {
        // Input-capable iff it has an input-scope format with channels. Output-only
        // devices error or report zero here, which is exactly the filter we want.
        let channels = d.input_asbd().map(|a| a.channels_per_frame).unwrap_or(0);
        if channels == 0 {
            continue;
        }
        let name = d.name().map(|s| s.to_string()).unwrap_or_default();
        let uid = d.uid().map(|s| s.to_string()).unwrap_or_default();
        let marker = if default == Some(*d) { "*" } else { " " };
        println!("  {marker} {name}  (ch={channels})  uid={uid}");
    }
    println!("  (* = current default input)");
}

/// (1b) Prove the default input device is settable — set it to the current default (a
/// no-op that still exercises the write path CoreAudio would reject if it were not
/// allowed). This is the lever the device picker will pull before starting the engine.
fn prove_selectable() {
    println!("== device selection ==");
    let Ok(default) = ca::System::default_input_device() else {
        println!("  no default input device");
        return;
    };
    let addr = ca::PropSelector::HW_DEFAULT_INPUT_DEVICE.global_addr();
    match ca::System::OBJ.set_prop(&addr, &default.0) {
        Ok(()) => println!("  set default input device: OK (settable)"),
        Err(e) => println!("  set default input device FAILED: {e:?}"),
    }

    // And prove uid -> device round-trips, which is how a saved setting resolves.
    if let Ok(uid) = default.uid() {
        match ca::Device::with_uid(&uid) {
            Ok(dev) => println!("  uid->device round-trip: OK (match={})", dev == default),
            Err(e) => println!("  uid->device FAILED: {e:?}"),
        }
    }
}

/// (2) Confirm the VPIO tap's timestamps are on the host time clock — the same clock
/// SCK audio PTS uses — so the two capture engines can be aligned by converting each to
/// seconds and taking the difference.
fn probe_clock() {
    println!("== mic clock probe (VPIO on) ==");
    let samples: Arc<Mutex<Vec<(u64, f64, f64)>>> = Arc::new(Mutex::new(Vec::new()));

    let mut engine = av::AudioEngine::new();
    let mut input = engine.input_node();
    if let Err(e) = input.set_vp_enabled(true) {
        println!("  could not enable VPIO: {e:?}");
        return;
    }
    let fmt = input.output_format_for_bus(0);

    let sink = samples.clone();
    let tap = move |_buf: &av::AudioPcmBuf, when: &av::AudioTime| {
        if !when.is_host_time_valid() {
            return;
        }
        let host_time = when.host_time();
        // Convert the tap's mach host time into a cm::Time on the host-time clock — the
        // exact clock SCK stamps its audio PTS with — then read that clock's "now".
        let stamp = cm::Clock::make_host_time_from_sys_units(host_time).as_secs();
        let now = cm::Clock::host_time_clock().time().as_secs();
        let mut g = sink.lock().unwrap();
        if g.len() < 5 {
            g.push((host_time, stamp, now));
        }
    };
    if let Err(e) = input.install_tap_on_bus(0, 4096, Some(&fmt), tap) {
        println!("  could not install tap: {e:?}");
        return;
    }

    engine.prepare();
    if let Err(e) = engine.start() {
        println!("  could not start engine: {e:?}");
        return;
    }
    std::thread::sleep(Duration::from_secs(2));
    engine.stop();
    unsafe { input.remove_tap_on_bus_throws(0) };

    println!("  tap timestamp (converted) vs host clock now:");
    for (host_time, stamp, now) in samples.lock().unwrap().iter() {
        println!(
            "    host_time={host_time}  stamp={stamp:.4}s  now={now:.4}s  behind_now={:.4}s",
            now - stamp
        );
    }
    println!(
        "  If `behind_now` is small and positive (~ one buffer of latency), the mic\n  \
         timestamps ARE on the host clock == SCK PTS basis. Lead-in sync is sound."
    );
}
