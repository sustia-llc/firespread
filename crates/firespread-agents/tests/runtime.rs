//! Bus routing, shutdown order, cancellation, member panics, and the agents
//! path against `run_hourly_ensemble`.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use firespread_agents::{
    AgentError, Bus, Delivery, EnsembleSpec, Event, Executor, ForecastConfig, Forecaster,
    GridInputs, INBOX, KanalBus, MemberOutcome, Source, run_members,
};
use firespread_ensemble::{
    DeadFuelMoisture, HourFields, HourlyConfig, HourlyInputs, HourlyPerturbation, RawHour,
    WindowWeather, run_hourly_ensemble, run_hourly_member,
};
use firespread_grid::{ForecastWindow, GridHeader, read_geotiff};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

const DX: f64 = 30.0;
const SEED: u64 = 7;

fn header(nx: usize, ny: usize) -> GridHeader {
    GridHeader {
        nx,
        ny,
        dx: DX,
        dy: DX,
        origin_x: 0.0,
        origin_y: 0.0,
        epsg: 5070,
        nodata: -9999.0,
    }
}

fn hour(n: usize, u10: f32, v10: f32) -> HourFields {
    HourFields::from_raw(
        &RawHour {
            u10_m_s: &vec![u10; n],
            v10_m_s: &vec![v10; n],
            t2m_k: &vec![300.0; n],
            rh2m_pct: &vec![20.0; n],
            tcc_pct: &vec![0.0; n],
        },
        &DeadFuelMoisture::default(),
        n,
    )
    .unwrap()
}

/// 90 x 90 cells: GR2 with a timber-understory band (TU5, canopy) in the
/// north third, a non-burnable column at 70, slope 12 deg facing SE, and a
/// 3 x 3 start mask at the centre; two hours of wind.
fn varied() -> (Box<GridInputs>, WindowWeather) {
    let (nx, ny) = (90, 90);
    let n = nx * ny;
    let mut fbfm40 = vec![102.0_f32; n];
    let mut ch = vec![0.0_f32; n];
    let mut cbh = vec![0.0_f32; n];
    let mut cc = vec![0.0_f32; n];
    let mut mask = vec![0_u8; n];
    for r in 0..ny {
        for c in 0..nx {
            let i = r * nx + c;
            if r < 30 {
                fbfm40[i] = 165.0;
                ch[i] = 15.0;
                cbh[i] = 3.0;
                cc[i] = 40.0;
            }
            if c == 70 {
                fbfm40[i] = 91.0;
            }
            if (44..=46).contains(&r) && (44..=46).contains(&c) {
                mask[i] = 1;
            }
        }
    }
    let grid = Box::new(GridInputs {
        header: header(nx, ny),
        fbfm40,
        slope_deg: vec![12.0; n],
        aspect_deg: vec![135.0; n],
        water: vec![0; n],
        ignited: mask,
        canopy_height_m: ch,
        canopy_base_height_m: cbh,
        canopy_cover_pct: cc,
    });
    let weather = WindowWeather {
        hours: vec![hour(n, 4.0, 3.0), hour(n, 2.0, -4.0)],
        hours_utc: vec!["hour 0".to_string(), "hour 1".to_string()],
        fm100_pct: vec![5.0; n],
        fm1000_pct: vec![7.0; n],
    };
    (grid, weather)
}

fn hourly_inputs() -> Arc<HourlyInputs> {
    let (grid, weather) = varied();
    Arc::new(HourlyInputs::from_layers(&grid.layers(), weather, &HourlyConfig::default()).unwrap())
}

fn spec(members: u32, horizons_h: &[u32]) -> EnsembleSpec {
    EnsembleSpec {
        seed: SEED,
        members,
        horizons_h: horizons_h.to_vec(),
        perturbation: HourlyPerturbation::default(),
    }
}

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("firespread-agents-{name}-{}", std::process::id()))
}

fn config(name: &str, members: u32, threads: usize) -> ForecastConfig {
    ForecastConfig {
        run_id: name.to_string(),
        runs_dir: scratch(name),
        window: ForecastWindow {
            name: "test".to_string(),
            start_t: "2022-04-22T12:00:00".to_string(),
            target_t: "2022-04-23T00:00:00".to_string(),
            fireids: vec![1],
        },
        hourly: HourlyConfig::default(),
        ensemble: spec(members, &[1, 2]),
        threads,
    }
}

#[tokio::test]
async fn bus_delivers_to_the_named_recipient_only() {
    let bus = KanalBus::<u32>::new();
    let a = bus.register("a").unwrap();
    let b = bus.register("b").unwrap();
    bus.send(Delivery {
        recipient: "b".to_string(),
        message: 42,
    })
    .await
    .unwrap();
    let got = tokio::time::timeout(Duration::from_secs(5), b.recv())
        .await
        .expect("b received nothing")
        .unwrap();
    assert_eq!((got.recipient.as_str(), got.message), ("b", 42));
    let stray = a.try_recv().unwrap();
    assert!(
        stray.is_none(),
        "a received {:?} addressed to b",
        stray.map(|d| (d.recipient, d.message))
    );
    assert!(
        b.try_recv().unwrap().is_none(),
        "b received a second message"
    );

    let err = bus
        .send(Delivery {
            recipient: "c".to_string(),
            message: 1,
        })
        .await
        .unwrap_err();
    assert!(
        matches!(&err, AgentError::UnknownAgent { name } if name == "c"),
        "{err:?}"
    );
    assert!(matches!(
        bus.register("a"),
        Err(AgentError::DuplicateAgent { .. })
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_cancels_before_close_and_drains_after() {
    let (grid, weather) = varied();
    let f = Forecaster::new(config("shutdown", 4, 2), Source::Memory { grid, weather })
        .await
        .unwrap();
    let drained = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&drained);
    f.spawn_agent("probe", move |token| async move {
        token.cancelled().await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        flag.store(true, Ordering::SeqCst);
    });
    let report = tokio::time::timeout(Duration::from_secs(20), f.shutdown())
        .await
        .expect("shutdown did not return within 20 s");
    assert!(
        report.cancelled_at_close,
        "tracker closed before the token was cancelled: {report:?}"
    );
    assert_eq!(report.tasks_left, 0, "{report:?}");
    assert!(
        drained.load(Ordering::SeqCst),
        "shutdown returned before the probe task finished its drain"
    );
    assert!(f.inbox().recv().await.is_err(), "bus open after shutdown");
}

/// The member function of the engine, counting starts.
fn counting(
    started: Arc<AtomicUsize>,
    delay: Duration,
) -> impl Fn(
    &HourlyInputs,
    &firespread_ensemble::MemberDraw,
    &mut firespread_spread::Workspace,
    &mut Vec<f32>,
    u32,
) -> firespread_ensemble::Result<firespread_ensemble::HourlyStats>
+ Send
+ Sync
+ 'static {
    move |inputs, draw, ws, arrival, horizon| {
        started.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(delay);
        run_hourly_member(inputs, draw, ws, arrival, horizon)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_token_admits_no_member() {
    let inputs = hourly_inputs();
    let exec = Executor::with_threads(2).unwrap();
    for round in 0..64 {
        let started = Arc::new(AtomicUsize::new(0));
        let token = CancellationToken::new();
        token.cancel();
        let outcome = run_members(
            &exec,
            Arc::clone(&inputs),
            &spec(4, &[1]),
            counting(Arc::clone(&started), Duration::ZERO),
            &token,
            |_| async {},
        )
        .await
        .unwrap();
        let n = started.load(Ordering::SeqCst);
        assert_eq!(
            n, 0,
            "round {round}: {n} members started after cancellation"
        );
        assert!(
            outcome.cancelled && outcome.completed.is_empty(),
            "round {round}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_mid_run_stops_new_members_and_returns_promptly() {
    let inputs = hourly_inputs();
    let exec = Executor::new(1, 2).unwrap();
    let started = Arc::new(AtomicUsize::new(0));
    let token = CancellationToken::new();
    let members = 500;
    let seen_at_cancel = Arc::new(AtomicUsize::new(usize::MAX));
    let t = Instant::now();
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        run_members(
            &exec,
            inputs,
            &spec(members, &[1]),
            counting(Arc::clone(&started), Duration::from_millis(20)),
            &token,
            |p| {
                if p.finished == 1 {
                    seen_at_cancel.store(started.load(Ordering::SeqCst), Ordering::SeqCst);
                    token.cancel();
                }
                async {}
            },
        ),
    )
    .await
    .expect("run did not return within 10 s of cancellation")
    .unwrap();
    let elapsed = t.elapsed();
    let seen = seen_at_cancel.load(Ordering::SeqCst);
    let n = started.load(Ordering::SeqCst);
    assert!(outcome.cancelled);
    assert!(
        n <= seen + exec.max_in_flight(),
        "{n} members started; {seen} had started at cancellation, admission {}",
        exec.max_in_flight()
    );
    assert!(
        outcome.completed.len() < 10,
        "{} of {members} completed",
        outcome.completed.len()
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "returned after {elapsed:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admitted_members_that_have_not_started_are_skipped() {
    let inputs = hourly_inputs();
    // One thread, four permits: members 0-3 are admitted at once and 1-3
    // queue behind member 0.
    let exec = Executor::new(1, 4).unwrap();
    let started = Arc::new(AtomicUsize::new(0));
    let first = Arc::new(Notify::new());
    let token = CancellationToken::new();
    let member_fn = {
        let started = Arc::clone(&started);
        let first = Arc::clone(&first);
        move |inputs: &HourlyInputs,
              draw: &firespread_ensemble::MemberDraw,
              ws: &mut firespread_spread::Workspace,
              arrival: &mut Vec<f32>,
              horizon: u32| {
            if started.fetch_add(1, Ordering::SeqCst) == 0 {
                first.notify_one();
                std::thread::sleep(Duration::from_millis(300));
            }
            run_hourly_member(inputs, draw, ws, arrival, horizon)
        }
    };
    let four = spec(4, &[1]);
    let run = run_members(&exec, inputs, &four, member_fn, &token, |_| async {});
    let control = async {
        first.notified().await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        token.cancel();
    };
    let (outcome, ()) = tokio::join!(run, control);
    let outcome = outcome.unwrap();
    let n = started.load(Ordering::SeqCst);
    assert_eq!(
        n, 1,
        "{n} members started; queued members ignored the stop flag"
    );
    assert_eq!(outcome.completed, vec![0]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_panicking_member_yields_the_typed_error_and_the_others_complete() {
    let inputs = hourly_inputs();
    let exec = Executor::with_threads(2).unwrap();
    let members = 8;
    let progress = Arc::new(AtomicUsize::new(0));
    let failed_progress = Arc::new(AtomicUsize::new(0));
    let member_fn = |inputs: &HourlyInputs,
                     draw: &firespread_ensemble::MemberDraw,
                     ws: &mut firespread_spread::Workspace,
                     arrival: &mut Vec<f32>,
                     horizon: u32| {
        let reference = HourlyPerturbation::default().draw(SEED, 3, DX).unwrap();
        if *draw == reference {
            panic!("injected panic in member 3");
        }
        run_hourly_member(inputs, draw, ws, arrival, horizon)
    };
    let outcome = run_members(
        &exec,
        inputs,
        &spec(members, &[1, 2]),
        member_fn,
        &CancellationToken::new(),
        |p| {
            progress.fetch_add(1, Ordering::SeqCst);
            if matches!(p.outcome, MemberOutcome::Failed(_)) {
                failed_progress.fetch_add(1, Ordering::SeqCst);
            }
            async {}
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.failures.len(), 1, "{:?}", outcome.failures);
    let (member, err) = &outcome.failures[0];
    assert_eq!(*member, 3);
    assert!(
        matches!(err.as_ref(), AgentError::MemberPanicked { member: 3, message }
            if message == "injected panic in member 3"),
        "{err:?}"
    );
    assert_eq!(outcome.completed, vec![0, 1, 2, 4, 5, 6, 7]);
    assert_eq!(outcome.counts.members, members - 1);
    assert_eq!(outcome.counts.summaries.len(), 7);
    assert_eq!(progress.load(Ordering::SeqCst), 8);
    assert_eq!(failed_progress.load(Ordering::SeqCst), 1);
    assert!(!outcome.cancelled);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_agents_path_equals_run_hourly_ensemble() {
    let members = 24;
    let (grid, weather) = varied();
    let want = {
        let inputs =
            HourlyInputs::from_layers(&grid.layers(), weather.clone(), &HourlyConfig::default())
                .unwrap();
        rayon::ThreadPoolBuilder::new()
            .num_threads(3)
            .build()
            .unwrap()
            .install(|| {
                run_hourly_ensemble(
                    &inputs,
                    &HourlyPerturbation::default(),
                    SEED,
                    members,
                    &[1, 2],
                )
            })
            .unwrap()
    };

    let cfg = config("e2e", members, 4);
    let runs_dir = cfg.runs_dir.clone();
    let f = Forecaster::new(cfg, Source::Memory { grid, weather })
        .await
        .unwrap();
    let inbox = f.inbox();
    let mut progress = Vec::new();
    let published = loop {
        let d = tokio::time::timeout(Duration::from_secs(60), inbox.recv())
            .await
            .expect("no event within 60 s")
            .unwrap();
        assert_eq!(d.recipient, INBOX);
        match d.message {
            Event::Progress(p) => progress.push(p.member),
            Event::Published(p) => break p,
            Event::Failed { agent, error } => panic!("{agent} failed: {error:?}"),
            other => panic!("unexpected event {other:?}"),
        }
    };
    let report = f.shutdown().await;
    assert_eq!(report.tasks_left, 0);

    progress.sort_unstable();
    assert_eq!(progress, (0..members).collect::<Vec<_>>());
    assert!(published.outcome.failures.is_empty());
    assert_eq!(
        published.outcome.counts, want,
        "agents vs run_hourly_ensemble"
    );
    for (k, h) in want.horizons_h.iter().enumerate() {
        let path = &published.outputs[&format!("burn_probability_h{h}")];
        let cog = read_geotiff(path).unwrap();
        assert_eq!(cog.data, want.probability(k).unwrap(), "{}", path.display());
    }
    let manifest = firespread_grid::Manifest::read(&published.manifest).unwrap();
    assert_eq!(manifest.members, members);
    assert_eq!(manifest.seed, SEED);
    assert_eq!(
        manifest.outputs.keys().collect::<Vec<_>>(),
        ["burn_probability_h1", "burn_probability_h2"]
    );
    std::fs::remove_dir_all(runs_dir).unwrap();
}
