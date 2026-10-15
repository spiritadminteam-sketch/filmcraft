//! End to end with a real VST3 plug-in in the real helper process: scan, register, create,
//! process, change a parameter, survive the host going away.
//!
//! Needs `FILMCRAFT_TEST_VST3` = a folder holding the `vst3` crate's example gain plug-in
//! (MIT OR Apache-2.0; built by CI as `Gain.vst3`) and `filmcraft-plugin-host` built next to the
//! test binary (`cargo build -p filmcraft-plugin-host`). Skipped otherwise.

/// The scans share the process-wide plug-in list: one test at a time.
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn setup() -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("FILMCRAFT_TEST_VST3").map(std::path::PathBuf::from)?;
    // with a test plug-in given, a missing helper is a failure, not a skip
    assert!(filmcraft_plugins::client::host_exe().is_some(), "FILMCRAFT_TEST_VST3 is set but filmcraft-plugin-host is not built");
    Some(dir)
}

#[test]
fn a_real_vst3_gain_plug_in_processes_audio() {
    let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(dir) = setup() else {
        eprintln!("skipped: FILMCRAFT_TEST_VST3 is not set");
        return;
    };
    let cache = std::env::temp_dir().join(format!("fc-vst3-cache-{}.json", std::process::id()));
    filmcraft_plugins::init(Some(cache.clone()));
    let report = filmcraft_plugins::scan(&[dir], &mut |_, _, _| {});
    assert!(report.failed.is_empty(), "failed: {:?}", report.failed);
    let gain = report.plugins.iter().find(|p| p.name.contains("Gain")).unwrap_or_else(|| panic!("no gain plug-in in {:?}", report.plugins));
    assert_eq!(gain.params.len(), 1, "{:?}", gain.params);
    assert_eq!(gain.params[0].title, "Gain");
    // registered as a FilmCraft audio effect, and saved
    let id = gain.effect_id();
    let def = filmcraft_project::find_effect(&id).expect("registered");
    assert_eq!(def.category, filmcraft_plugins::CATEGORY);
    assert_eq!(def.params[0].id, "p0");
    assert!(std::fs::read_to_string(&cache).unwrap().contains(&gain.cid));

    let mut fx = filmcraft_plugins::create_effect(&id, 48000.0, 2).expect("instance");
    let input: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
    // default gain: 1.0 (100 %)
    let (mut l, mut r) = (input.clone(), input.clone());
    fx.process(&mut [&mut l, &mut r]);
    for (a, b) in l.iter().zip(&input) {
        assert!((a - b).abs() < 1e-6, "{a} vs {b}");
    }
    // 25 %: a quarter of the signal
    assert!(fx.set_param("p0", 25.0));
    let (mut l, mut r) = (input.clone(), input.clone());
    fx.process(&mut [&mut l, &mut r]);
    for (a, b) in l.iter().zip(&input) {
        assert!((a - b * 0.25).abs() < 1e-5, "{a} vs {}", b * 0.25);
    }
    assert!(r.iter().zip(&input).all(|(a, b)| (a - b * 0.25).abs() < 1e-5));
    // a long block goes in pieces
    let mut long = vec![1.0f32; 20_000];
    let mut long_r = long.clone();
    fx.process(&mut [&mut long, &mut long_r]);
    assert!(long.iter().all(|v| (v - 0.25).abs() < 1e-5));

    // the helper goes away: the effect passes audio through instead of failing
    filmcraft_plugins::client::shutdown();
    let (mut l, mut r) = (input.clone(), input.clone());
    fx.process(&mut [&mut l, &mut r]);
    assert_eq!(l, input);
    // a new instance starts a new helper
    let mut fx2 = filmcraft_plugins::create_effect(&id, 44100.0, 2).expect("instance");
    fx2.set_param("p0", 50.0);
    let (mut l, mut r) = (input.clone(), input.clone());
    fx2.process(&mut [&mut l, &mut r]);
    assert!(l.iter().zip(&input).all(|(a, b)| (a - b * 0.5).abs() < 1e-5));
    drop(fx);
    drop(fx2);
    let _ = std::fs::remove_file(cache);
}

#[test]
fn a_module_that_is_not_a_plug_in_is_reported_not_fatal() {
    let _g = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if filmcraft_plugins::client::host_exe().is_none() {
        eprintln!("skipped: filmcraft-plugin-host is not built");
        return;
    }
    let dir = std::env::temp_dir().join(format!("fc-vst3-bad-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Broken.vst3"), b"this is not a library").unwrap();
    let report = filmcraft_plugins::scan(std::slice::from_ref(&dir), &mut |_, _, _| {});
    assert_eq!(report.modules, 1);
    assert!(report.plugins.is_empty());
    assert_eq!(report.failed.len(), 1, "{:?}", report.failed);
    let _ = std::fs::remove_dir_all(&dir);
}
